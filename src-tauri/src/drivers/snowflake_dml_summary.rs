//! Pure parsing of Snowflake DML status rows. An unavailable summary says
//! nothing about statement acknowledgement, transaction outcome or data effects.

use crate::domain::{
    ColumnInfo, ColumnKind, SnowflakeDmlKind, SnowflakeDmlSummary, SnowflakeDmlSummaryStatus,
    TextRow,
};
use std::io::{self, Write};

pub const MAX_SUMMARY_BYTES: usize = 64 * 1024;
const MULTI_JOINED: &str = "number of multi-joined rows updated";

/// Accept only the reviewed status names and FIXED wire type. The caller must
/// supply the complete retained result, never a sample of a larger result.
/// Counts remain exact decimal strings; status-row count is not affected rows.
pub fn parse(
    kind: SnowflakeDmlKind,
    columns: &[ColumnInfo],
    rows: &[TextRow],
    truncated: bool,
) -> SnowflakeDmlSummary {
    parse_complete(kind, columns, rows, truncated).unwrap_or_else(SnowflakeDmlSummary::unavailable)
}

fn parse_complete(
    kind: SnowflakeDmlKind,
    columns: &[ColumnInfo],
    rows: &[TextRow],
    truncated: bool,
) -> Option<SnowflakeDmlSummary> {
    if truncated
        || rows.len() != 1
        || columns.is_empty()
        || columns.len() > 2
        || rows[0].len() != columns.len()
    {
        return None;
    }
    // Includes every descriptor field, even identifiers and flags not used by
    // this parser. Serialization stops at the cap without allocating a copy.
    serde_json::to_writer(BudgetWriter(MAX_SUMMARY_BYTES), &(columns, rows)).ok()?;
    let expected = match kind {
        SnowflakeDmlKind::Insert => "number of rows inserted",
        SnowflakeDmlKind::Update => "number of rows updated",
        SnowflakeDmlKind::Delete => "number of rows deleted",
    };
    let mut affected_rows = None;
    let mut multi_joined_rows = None;
    for (column, cell) in columns.iter().zip(&rows[0]) {
        if !column.database_type.eq_ignore_ascii_case("FIXED") || column.kind != ColumnKind::Decimal
        {
            return None;
        }
        let value = count(cell.as_deref()?)?;
        if column.name == expected && affected_rows.is_none() {
            affected_rows = Some(value);
        } else if kind == SnowflakeDmlKind::Update
            && column.name == MULTI_JOINED
            && multi_joined_rows.is_none()
        {
            multi_joined_rows = Some(value);
        } else {
            // Includes duplicate names, other DML kinds and unknown variants.
            return None;
        }
    }
    Some(SnowflakeDmlSummary {
        status: SnowflakeDmlSummaryStatus::Validated,
        affected_rows: Some(affected_rows?),
        multi_joined_rows,
    })
}

fn count(text: &str) -> Option<String> {
    // Snowflake NUMBER has at most 38 decimal digits. No signs, whitespace,
    // exponent/fraction syntax, locale formatting or Unicode digit aliases.
    if text.is_empty() || text.len() > 38 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.bytes()
        .try_fold(0u128, |value, byte| {
            value.checked_mul(10)?.checked_add(u128::from(byte - b'0'))
        })
        .map(|value| value.to_string())
}

struct BudgetWriter(usize);
impl Write for BudgetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("Snowflake DML summary limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            id: "c0".into(),
            name: name.into(),
            database_type: "FIXED".into(),
            kind: ColumnKind::Decimal,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    fn row(values: &[&str]) -> TextRow {
        values.iter().map(|value| Some((*value).into())).collect()
    }

    fn unavailable(summary: SnowflakeDmlSummary) {
        assert_eq!(summary, SnowflakeDmlSummary::unavailable());
    }

    #[test]
    fn parses_each_kind_without_confusing_status_rows_with_affected_rows() {
        for (kind, name) in [
            (SnowflakeDmlKind::Insert, "number of rows inserted"),
            (SnowflakeDmlKind::Update, "number of rows updated"),
            (SnowflakeDmlKind::Delete, "number of rows deleted"),
        ] {
            for value in [
                "0",
                "42",
                "9007199254740993",
                "99999999999999999999999999999999999999",
            ] {
                let summary = parse(kind, &[column(name)], &[row(&[value])], false);
                assert_eq!(summary.status, SnowflakeDmlSummaryStatus::Validated);
                assert_eq!(summary.affected_rows.as_deref(), Some(value));
                assert_eq!(summary.multi_joined_rows, None);
            }
        }
    }

    #[test]
    fn update_preserves_separate_optional_multi_joined_count_in_either_order() {
        for reversed in [false, true] {
            let mut columns = vec![column("number of rows updated"), column(MULTI_JOINED)];
            let mut values = row(&["9007199254740993", "2"]);
            if reversed {
                columns.reverse();
                values.reverse();
            }
            let summary = parse(SnowflakeDmlKind::Update, &columns, &[values], false);
            assert_eq!(summary.status, SnowflakeDmlSummaryStatus::Validated);
            assert_eq!(summary.affected_rows.as_deref(), Some("9007199254740993"));
            assert_eq!(summary.multi_joined_rows.as_deref(), Some("2"));
        }
        unavailable(parse(
            SnowflakeDmlKind::Update,
            &[column(MULTI_JOINED)],
            &[row(&["2"])],
            false,
        ));
    }

    #[test]
    fn accepts_only_fixed_wire_type_casing_and_matching_numeric_kind() {
        for ty in [
            "FIXED",
            "fixed",
            "FiXeD",
            "NUMBER",
            "DECIMAL",
            "INTEGER",
            "REAL",
            "FLOAT",
            "TEXT",
            " FIXED",
            "FIXED(38,0)",
        ] {
            let mut descriptor = column("number of rows inserted");
            descriptor.database_type = ty.into();
            let summary = parse(
                SnowflakeDmlKind::Insert,
                &[descriptor],
                &[row(&["1"])],
                false,
            );
            assert_eq!(
                summary.status == SnowflakeDmlSummaryStatus::Validated,
                ty.eq_ignore_ascii_case("fixed"),
                "{ty}"
            );
        }
        for kind in [ColumnKind::Text, ColumnKind::Integer, ColumnKind::Unknown] {
            let mut descriptor = column("number of rows inserted");
            descriptor.kind = kind;
            unavailable(parse(
                SnowflakeDmlKind::Insert,
                &[descriptor],
                &[row(&["1"])],
                false,
            ));
        }
    }

    #[test]
    fn rejects_null_nonintegral_signed_unicode_and_overflowing_values() {
        let columns = [column("number of rows deleted")];
        for value in [
            "",
            "-1",
            "+1",
            "1.0",
            "1e2",
            " 1",
            "1 ",
            "1,000",
            "١",
            "１",
            "NaN",
            "Infinity",
            "1\0",
            "340282366920938463463374607431768211456",
            "100000000000000000000000000000000000000",
        ] {
            unavailable(parse(
                SnowflakeDmlKind::Delete,
                &columns,
                &[row(&[value])],
                false,
            ));
        }
        unavailable(parse(
            SnowflakeDmlKind::Delete,
            &columns,
            &[vec![None]],
            false,
        ));
        assert_eq!(
            parse(
                SnowflakeDmlKind::Delete,
                &columns,
                &[row(&["00042"])],
                false
            )
            .affected_rows
            .as_deref(),
            Some("42")
        );
    }

    #[test]
    fn rejects_missing_extra_truncated_and_ragged_rows_or_columns() {
        let columns = [column("number of rows inserted")];
        for rows in [
            vec![],
            vec![row(&[])],
            vec![row(&["1", "2"])],
            vec![row(&["1"]), row(&["2"])],
        ] {
            unavailable(parse(SnowflakeDmlKind::Insert, &columns, &rows, false));
        }
        unavailable(parse(
            SnowflakeDmlKind::Insert,
            &columns,
            &[row(&["1"])],
            true,
        ));
        unavailable(parse(SnowflakeDmlKind::Insert, &[], &[row(&[])], false));
        unavailable(parse(
            SnowflakeDmlKind::Update,
            &[
                column("number of rows updated"),
                column(MULTI_JOINED),
                column("unexpected"),
            ],
            &[row(&["1", "2", "3"])],
            false,
        ));
    }

    #[test]
    fn rejects_duplicate_foreign_and_unreviewed_status_names() {
        for names in [
            vec!["number of rows updated", "number of rows updated"],
            vec![MULTI_JOINED, MULTI_JOINED],
            vec!["NUMBER OF ROWS UPDATED"],
            vec!["number of rows updated "],
            vec!["number of rows inserted"],
            vec!["number of rows updated", "unknown"],
        ] {
            let columns: Vec<_> = names.iter().map(|name| column(name)).collect();
            let values = vec![Some("1".into()); columns.len()];
            unavailable(parse(SnowflakeDmlKind::Update, &columns, &[values], false));
        }
        for kind in [SnowflakeDmlKind::Insert, SnowflakeDmlKind::Delete] {
            let name = if kind == SnowflakeDmlKind::Insert {
                "number of rows inserted"
            } else {
                "number of rows deleted"
            };
            unavailable(parse(
                kind,
                &[column(name), column(MULTI_JOINED)],
                &[row(&["1", "2"])],
                false,
            ));
        }
    }

    #[test]
    fn accounts_for_uninterpreted_descriptors_and_exact_serialized_byte_boundary() {
        let mut columns = [column("number of rows inserted")];
        let rows = [row(&["1"])];
        let initial = serde_json::to_vec(&(&columns, &rows)).unwrap().len();
        columns[0]
            .id
            .push_str(&"x".repeat(MAX_SUMMARY_BYTES - initial));
        assert_eq!(
            serde_json::to_vec(&(&columns, &rows)).unwrap().len(),
            MAX_SUMMARY_BYTES
        );
        assert_eq!(
            parse(SnowflakeDmlKind::Insert, &columns, &rows, false).status,
            SnowflakeDmlSummaryStatus::Validated
        );
        columns[0].id.push('x');
        unavailable(parse(SnowflakeDmlKind::Insert, &columns, &rows, false));
        columns[0].id = "雪".repeat(MAX_SUMMARY_BYTES / 3);
        unavailable(parse(SnowflakeDmlKind::Insert, &columns, &rows, false));
        columns[0].id = "\n".repeat(MAX_SUMMARY_BYTES / 2);
        unavailable(parse(SnowflakeDmlKind::Insert, &columns, &rows, false));
    }
}
