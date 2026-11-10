//! Pure observations of one existing SDK session. Callers own the connection,
//! serialized lane, deadline, identity pinning and publication. No helper here
//! changes a setting, recovers a transaction or infers transaction usability.

use super::snowflake_metadata::{self as metadata, LiveContext, MetadataResult};
use crate::domain::{AppError, AppResult, ColumnKind, ErrorCode, TextRow};
use std::collections::HashSet;
use std::io::{self, Write};

pub(crate) const OBSERVATION_MAX_BYTES: usize = 64 * 1024;
pub(crate) const TRANSACTION_ID_MAX_BYTES: usize = 1024;

// SHOW LIKE permits wildcard matches, including underscores. Parsing must still
// require the exact returned key, and read value rather than default or level.
// https://docs.snowflake.com/en/sql-reference/sql/show-parameters
pub(crate) const AUTOCOMMIT_SQL: &str = "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION";
pub(crate) const ABORT_ON_ERROR_SQL: &str =
    "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION";
// Cast on the server before the SDK can represent a numeric ID as JS Number.
// This identifies activity, not whether an open transaction remains usable.
// https://docs.snowflake.com/en/sql-reference/functions/current_transaction
pub(crate) const CURRENT_TRANSACTION_SQL: &str =
    "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID";

/// A successfully parsed five-statement sample. SQL NULL transaction_id means
/// no transaction was observed; a failed/unavailable sample is not this value.
/// Nullable role and other context fields are never filled from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservationSample {
    pub context: LiveContext,
    pub autocommit: bool,
    pub abort_on_error: bool,
    pub transaction_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleConsistency {
    Unavailable,
    Stable,
    Changed,
}

/// Compare complete observations, not just context fingerprints. Neither two
/// unavailable samples nor an unavailable sample and an observed NULL are stable.
pub(crate) fn compare_samples(
    before: Option<&ObservationSample>,
    after: Option<&ObservationSample>,
) -> SampleConsistency {
    match (before, after) {
        (Some(before), Some(after)) if before == after => SampleConsistency::Stable,
        (Some(_), Some(_)) => SampleConsistency::Changed,
        _ => SampleConsistency::Unavailable,
    }
}

pub(crate) fn parse_sample(
    context: &MetadataResult,
    policy: &MetadataResult,
    autocommit: &MetadataResult,
    abort_on_error: &MetadataResult,
    transaction: &MetadataResult,
) -> AppResult<ObservationSample> {
    complete_row(context)?;
    complete_row(policy)?;
    Ok(ObservationSample {
        // Reuse the full structured context and quoted-identifier policy parser.
        // Unsupported policy stays an error, rather than a profile substitution.
        context: metadata::parse_context(context, policy)?,
        autocommit: parse_autocommit(autocommit)?,
        abort_on_error: parse_abort_on_error(abort_on_error)?,
        transaction_id: parse_transaction_id(transaction)?,
    })
}

pub(crate) fn parse_autocommit(result: &MetadataResult) -> AppResult<bool> {
    parse_parameter(result, "AUTOCOMMIT")
}

pub(crate) fn parse_abort_on_error(result: &MetadataResult) -> AppResult<bool> {
    parse_parameter(result, "TRANSACTION_ABORT_ON_ERROR")
}

fn parse_parameter(result: &MetadataResult, expected_key: &str) -> AppResult<bool> {
    let row = complete_row(result)?;
    let find = |name: &str| -> AppResult<usize> {
        let position = result
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .ok_or_else(malformed)?;
        if result.columns[position].kind != ColumnKind::Text {
            return Err(malformed());
        }
        Ok(position)
    };
    if row[find("key")?].as_deref() != Some(expected_key) {
        return Err(malformed());
    }
    match row[find("value")?].as_deref() {
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        _ => Err(malformed()),
    }
}

pub(crate) fn parse_transaction_id(result: &MetadataResult) -> AppResult<Option<String>> {
    let row = complete_row(result)?;
    if result.columns.len() != 1
        || result.columns[0].name != "TRANSACTION_ID"
        || result.columns[0].kind != ColumnKind::Text
    {
        return Err(malformed());
    }
    row[0]
        .as_deref()
        .map(|value| {
            if value.len() > TRANSACTION_ID_MAX_BYTES {
                return Err(limit());
            }
            if value.is_empty() || value.contains('\0') {
                return Err(malformed());
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn complete_row(result: &MetadataResult) -> AppResult<&TextRow> {
    if result.truncated || result.rows_seen != result.rows.len() as u64 {
        return Err(limit());
    }
    if result.rows.len() != 1
        || result.columns.is_empty()
        || result.columns.len() > metadata::COLUMN_LIMIT
        || result.rows[0].len() != result.columns.len()
    {
        return Err(malformed());
    }
    // Includes descriptor strings and fields not consumed by the parser. Stop
    // serialization at the budget rather than allocate an unbounded JSON copy.
    serde_json::to_writer(BudgetWriter(OBSERVATION_MAX_BYTES), result).map_err(|_| limit())?;
    let mut names = HashSet::new();
    if result
        .columns
        .iter()
        .any(|column| !names.insert(column.name.to_ascii_lowercase()))
    {
        return Err(malformed());
    }
    Ok(&result.rows[0])
}

struct BudgetWriter(usize);
impl Write for BudgetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("Snowflake observation byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake session observation is incomplete or exceeds its bounded limit",
    )
}
fn malformed() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake session observation has an invalid result shape or value",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ColumnInfo;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            id: "wire".into(),
            name: name.into(),
            database_type: "VARCHAR".into(),
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }
    fn result(names: &[&str], rows: Vec<TextRow>) -> MetadataResult {
        MetadataResult {
            columns: names.iter().map(|name| column(name)).collect(),
            rows_seen: rows.len() as u64,
            rows,
            truncated: false,
        }
    }
    fn row(values: &[&str]) -> TextRow {
        values.iter().map(|value| Some((*value).into())).collect()
    }
    fn parameter(key: &str, value: &str) -> MetadataResult {
        result(
            &["value", "default", "level", "key"],
            vec![row(&[value, "false", "ACCOUNT", key])],
        )
    }
    fn transaction(value: Option<&str>) -> MetadataResult {
        result(&["TRANSACTION_ID"], vec![vec![value.map(str::to_owned)]])
    }
    fn context() -> MetadataResult {
        result(
            &[
                "PRINCIPAL",
                "ROLE_NAME",
                "DATABASE_NAME",
                "SCHEMA_NAME",
                "WAREHOUSE_NAME",
                "SESSION_ID",
                "SEARCH_PATH",
                "SECONDARY_ROLES",
            ],
            vec![row(&[
                "USER_NAME",
                "ANALYST",
                "DB",
                "PUBLIC",
                "WH",
                "opaque-S7/session",
                r#"["DB.PUBLIC","DB.OTHER"]"#,
                r#"{"roles":"R1,R2","value":"ALL"}"#,
            ])],
        )
    }
    fn policy() -> MetadataResult {
        parameter("QUOTED_IDENTIFIERS_IGNORE_CASE", "false")
    }
    fn sample() -> ObservationSample {
        parse_sample(
            &context(),
            &policy(),
            &parameter("AUTOCOMMIT", "true"),
            &parameter("TRANSACTION_ABORT_ON_ERROR", "false"),
            &transaction(None),
        )
        .unwrap()
    }

    #[test]
    fn fixed_sql_observes_without_setting_or_recovering_anything() {
        assert_eq!(
            AUTOCOMMIT_SQL,
            "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION"
        );
        assert_eq!(
            ABORT_ON_ERROR_SQL,
            "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION"
        );
        assert_eq!(
            CURRENT_TRANSACTION_SQL,
            "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID"
        );
    }

    #[test]
    fn parameter_uses_current_value_and_accepts_inherited_level_and_reordered_headers() {
        let mut data = parameter("AUTOCOMMIT", "true");
        assert!(parse_autocommit(&data).unwrap());
        data.rows[0][1] = Some("not-a-boolean-default".into());
        data.rows[0][2] = None;
        data.columns[0].name = "VaLuE".into();
        data.columns[3].name = "KEY".into();
        assert!(parse_autocommit(&data).unwrap());
        assert!(!parse_autocommit(&parameter("AUTOCOMMIT", "FALSE")).unwrap());
        assert!(parse_abort_on_error(&parameter("TRANSACTION_ABORT_ON_ERROR", "TrUe")).unwrap());
    }

    #[test]
    fn parameter_rejects_wildcard_matches_wrong_keys_and_non_boolean_values() {
        for key in [
            "AUTOCOMMIT_API_SUPPORTED",
            "autocommit",
            " AUTOCOMMIT",
            "",
            "TRANSACTIONXABORTXONXERROR",
        ] {
            assert!(parse_autocommit(&parameter(key, "true")).is_err());
        }
        assert!(parse_abort_on_error(&parameter("ABORT_TRANSACTION_ON_ERROR", "true")).is_err());
        for value in [
            "", "1", "0", "on", "off", "true ", " false", "null", "TRUE\0",
        ] {
            assert!(parse_autocommit(&parameter("AUTOCOMMIT", value)).is_err());
            assert!(parse_abort_on_error(&parameter("TRANSACTION_ABORT_ON_ERROR", value)).is_err());
        }
        let mut data = parameter("AUTOCOMMIT", "true");
        data.rows[0][0] = None;
        assert!(parse_autocommit(&data).is_err());
        data.rows[0][0] = Some("true".into());
        data.rows[0][3] = None;
        assert!(parse_autocommit(&data).is_err());
    }

    #[test]
    fn parameter_rejects_missing_duplicate_and_nontext_required_headers() {
        for position in [0, 3] {
            let mut data = parameter("AUTOCOMMIT", "true");
            data.columns[position].name = "not_expected".into();
            assert!(parse_autocommit(&data).is_err());
            data = parameter("AUTOCOMMIT", "true");
            data.columns[position].kind = ColumnKind::Boolean;
            assert!(parse_autocommit(&data).is_err());
            data = parameter("AUTOCOMMIT", "true");
            let mut duplicate = data.columns[position].clone();
            duplicate.name = duplicate.name.to_ascii_uppercase();
            data.columns.push(duplicate);
            data.rows[0].push(Some("true".into()));
            assert!(parse_autocommit(&data).is_err());
        }
        let mut data = parameter("AUTOCOMMIT", "true");
        data.columns.push(column("LEVEL"));
        data.rows[0].push(None);
        assert!(parse_autocommit(&data).is_err());
    }

    #[test]
    fn transaction_null_is_distinct_from_opaque_text_without_number_conversion() {
        assert_eq!(parse_transaction_id(&transaction(None)).unwrap(), None);
        for id in [
            "opaque-A7/session",
            "9007199254740993",
            "-1700000000000000000",
        ] {
            assert_eq!(
                parse_transaction_id(&transaction(Some(id)))
                    .unwrap()
                    .as_deref(),
                Some(id)
            );
        }
        let boundary = "x".repeat(TRANSACTION_ID_MAX_BYTES);
        assert_eq!(
            parse_transaction_id(&transaction(Some(&boundary))).unwrap(),
            Some(boundary)
        );
    }

    #[test]
    fn transaction_rejects_empty_nul_oversized_and_wrong_typed_shape() {
        for id in [
            String::new(),
            "bad\0id".into(),
            "x".repeat(TRANSACTION_ID_MAX_BYTES + 1),
            "é".repeat(TRANSACTION_ID_MAX_BYTES),
        ] {
            assert!(parse_transaction_id(&transaction(Some(&id))).is_err());
        }
        let mut data = transaction(None);
        data.columns[0].kind = ColumnKind::Decimal;
        assert!(parse_transaction_id(&data).is_err());
        data = transaction(None);
        data.columns[0].name = "CURRENT_TRANSACTION()".into();
        assert!(parse_transaction_id(&data).is_err());
        data = transaction(None);
        data.columns.push(column("extra"));
        data.rows[0].push(None);
        assert!(parse_transaction_id(&data).is_err());
    }

    #[test]
    fn parsers_reject_incomplete_and_non_single_row_results() {
        for initial in [parameter("AUTOCOMMIT", "true"), transaction(None)] {
            let parse = |data: &MetadataResult| {
                if data.columns.len() == 1 {
                    parse_transaction_id(data).map(|_| ())
                } else {
                    parse_autocommit(data).map(|_| ())
                }
            };
            let mut data = MetadataResult {
                truncated: true,
                ..initial.clone()
            };
            assert!(parse(&data).is_err());
            data = initial.clone();
            data.rows_seen += 1;
            assert!(parse(&data).is_err());
            data = initial.clone();
            data.rows.clear();
            data.rows_seen = 0;
            assert!(parse(&data).is_err());
            data = initial.clone();
            data.rows.push(data.rows[0].clone());
            data.rows_seen = 2;
            assert!(parse(&data).is_err());
            data = initial.clone();
            data.rows[0].pop();
            assert!(parse(&data).is_err());
            data = initial;
            data.rows[0].push(None);
            assert!(parse(&data).is_err());
        }
    }

    #[test]
    fn byte_budget_includes_unconsumed_fields_and_descriptors() {
        let mut data = parameter("AUTOCOMMIT", "true");
        data.rows[0][1] = Some("x".repeat(OBSERVATION_MAX_BYTES));
        assert_eq!(
            parse_autocommit(&data).unwrap_err().code,
            ErrorCode::ResultLimitReached
        );
        data = parameter("AUTOCOMMIT", "true");
        data.columns[2].database_type = "x".repeat(OBSERVATION_MAX_BYTES);
        assert!(parse_autocommit(&data).is_err());
        let mut data = transaction(None);
        data.columns[0].id = "x".repeat(OBSERVATION_MAX_BYTES);
        assert!(parse_transaction_id(&data).is_err());
    }

    #[test]
    fn sample_preserves_nullable_role_and_other_context_without_profile_fallbacks() {
        let mut data = context();
        for position in [1, 2, 3, 4] {
            data.rows[0][position] = None;
        }
        data.rows[0][6] = Some("[]".into());
        let observed = parse_sample(
            &data,
            &policy(),
            &parameter("AUTOCOMMIT", "false"),
            &parameter("TRANSACTION_ABORT_ON_ERROR", "true"),
            &transaction(Some("opaque-TX")),
        )
        .unwrap();
        assert_eq!(observed.context.role, None);
        assert_eq!(observed.context.database, None);
        assert_eq!(observed.context.schema, None);
        assert_eq!(observed.context.warehouse, None);
        assert!(!observed.autocommit);
        assert!(observed.abort_on_error);
        assert_eq!(observed.transaction_id.as_deref(), Some("opaque-TX"));
    }

    #[test]
    fn sample_rejects_unsupported_or_malformed_policy_and_context() {
        let parse = |context: &MetadataResult, policy: &MetadataResult| {
            parse_sample(
                context,
                policy,
                &parameter("AUTOCOMMIT", "true"),
                &parameter("TRANSACTION_ABORT_ON_ERROR", "false"),
                &transaction(None),
            )
        };
        let mut policy = policy();
        policy.rows[0][0] = Some("true".into());
        assert_eq!(
            parse(&context(), &policy).unwrap_err().code,
            ErrorCode::Unsupported
        );
        policy.rows[0][0] = Some("invalid".into());
        assert!(parse(&context(), &policy).is_err());
        policy.rows[0][0] = Some("false".into());
        let mut data = context();
        data.rows[0][1] = Some(String::new());
        assert!(parse(&data, &policy).is_err());
        data = context();
        data.columns[5].kind = ColumnKind::Decimal;
        assert!(parse(&data, &policy).is_err());
        policy.columns.push(column("KEY"));
        policy.rows[0].push(None);
        assert!(parse(&context(), &policy).is_err());
    }

    #[test]
    fn comparison_never_treats_unavailable_as_observed_null_or_stable() {
        let sample = sample();
        assert_eq!(compare_samples(None, None), SampleConsistency::Unavailable);
        assert_eq!(
            compare_samples(Some(&sample), None),
            SampleConsistency::Unavailable
        );
        assert_eq!(
            compare_samples(None, Some(&sample)),
            SampleConsistency::Unavailable
        );
        assert_eq!(
            compare_samples(Some(&sample), Some(&sample.clone())),
            SampleConsistency::Stable
        );
    }

    #[test]
    fn comparison_tracks_full_context_mode_policy_and_transaction_identity() {
        let original = sample();
        let changes: &[fn(&mut ObservationSample)] = &[
            |s| s.autocommit = !s.autocommit,
            |s| s.abort_on_error = !s.abort_on_error,
            |s| s.transaction_id = Some("opaque-TX".into()),
            |s| s.context.principal = "OTHER".into(),
            |s| s.context.session_id = "other-session".into(),
            |s| s.context.role = None,
            |s| s.context.database = Some("OTHER".into()),
            |s| s.context.schema = Some("OTHER".into()),
            |s| s.context.warehouse = Some("OTHER".into()),
            |s| s.context.secondary_roles.value = "NONE".into(),
            |s| s.context.secondary_roles.roles = "OTHER".into(),
            |s| s.context.search_path.reverse(),
            |s| s.context.quoted_identifiers_ignore_case = true,
        ];
        for change in changes {
            let mut changed = original.clone();
            change(&mut changed);
            assert_eq!(
                compare_samples(Some(&original), Some(&changed)),
                SampleConsistency::Changed
            );
        }
        let mut transaction = original.clone();
        transaction.transaction_id = Some("tx-A".into());
        let mut changed = transaction.clone();
        changed.transaction_id = Some("tx-B".into());
        assert_eq!(
            compare_samples(Some(&transaction), Some(&changed)),
            SampleConsistency::Changed
        );
    }
}
