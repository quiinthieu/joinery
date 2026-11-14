//! Closed owner-scoped metadata transport, without SQL or transaction authority.
//! A successful single read does not establish a fresh complete inspection.
use crate::relation_inspection::{
    MAX_PREVIEW_FILTER_VALUE_BYTES, MAX_PREVIEW_FILTERS, MAX_PREVIEW_OFFSET, RelationFilter,
    RelationFilterOperator, RelationSort,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Kind {
    Context,
    QuotedIdentifierPolicy,
    CurrentTransaction,
    ExactRelation,
    Columns,
    Describe,
    ZeroRowResolution,
    Preview,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    user: String,
    role: Option<String>,
    database: Option<String>,
    schema: Option<String>,
    warehouse: Option<String>,
    session: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Target {
    database: String,
    schema: String,
    name: String,
    table_type: String,
    columns: Option<Vec<String>>,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    sort: Option<RelationSort>,
    #[serde(default)]
    filters: Vec<RelationFilter>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    owner: String,
    expected: Expected,
    kind: Kind,
    target: Option<Target>,
    credits: u32,
}
fn text(value: &str, bytes: usize) -> bool {
    !value.is_empty() && value.len() <= bytes && !value.contains('\0')
}
fn identifier(value: &str) -> bool {
    text(value, 1020) && value.chars().count() <= 255
}

/// Separate from the ordinary query ceiling and immutable batch evidence.
/// Counts and bytes include every emitted batch, even after local discard.
pub(crate) struct ReadState {
    max_rows: u64,
    max_bytes: usize,
    rows: u64,
    bytes: usize,
    described: bool,
}
impl ReadState {
    pub(crate) fn parse(payload: &Value) -> Result<Self, &'static str> {
        // Option<T> would otherwise accept missing nullable fields; the wire
        // contract requires them explicitly, matching the helper's exact shape.
        let exact = |value: &Value, keys: &[&str]| {
            value
                .as_object()
                .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
        };
        if !exact(payload, &["owner", "expected", "kind", "target", "credits"])
            || !exact(
                &payload["expected"],
                &["user", "role", "database", "schema", "warehouse", "session"],
            )
        {
            return Err("invalid owner metadata request");
        }
        let r: Request = serde_json::from_value(payload.clone())
            .map_err(|_| "invalid owner metadata request")?;
        let valid_owner = Uuid::parse_str(&r.owner).is_ok_and(|id| {
            id.to_string() == r.owner
                && matches!(id.get_version_num(), 1..=5)
                && id.get_variant() == uuid::Variant::RFC4122
        });
        if !valid_owner
            || !(1..=32).contains(&r.credits)
            || !identifier(&r.expected.user)
            || !text(&r.expected.session, 1024)
            || [
                &r.expected.role,
                &r.expected.database,
                &r.expected.schema,
                &r.expected.warehouse,
            ]
            .into_iter()
            .flatten()
            .any(|s| !identifier(s))
        {
            return Err("invalid owner metadata request");
        }
        let (max_rows, max_bytes) = match r.kind {
            Kind::Context | Kind::QuotedIdentifierPolicy | Kind::CurrentTransaction => {
                if r.target.is_some() {
                    return Err("unexpected owner metadata target");
                }
                (1, 64 * 1024)
            }
            Kind::Preview => {
                let t = r.target.ok_or("missing owner metadata target")?;
                let cols = t.columns.as_ref().ok_or("missing owner preview columns")?;
                let unique: HashSet<&str> = cols.iter().map(String::as_str).collect();
                let target_obj = payload["target"]
                    .as_object()
                    .ok_or("invalid owner preview target")?;
                let required_present = ["database", "schema", "name", "tableType", "columns"]
                    .iter()
                    .all(|k| target_obj.contains_key(*k));
                let allowed_keys = target_obj.keys().all(|k| {
                    matches!(
                        k.as_str(),
                        "database"
                            | "schema"
                            | "name"
                            | "tableType"
                            | "columns"
                            | "offset"
                            | "sort"
                            | "filters"
                    )
                });
                let valid_sort = t
                    .sort
                    .as_ref()
                    .is_none_or(|sort| unique.contains(sort.column.as_str()));
                let valid_filters = t.filters.len() <= MAX_PREVIEW_FILTERS
                    && t.filters.iter().all(|f| {
                        unique.contains(f.column.as_str())
                            && match f.operator {
                                RelationFilterOperator::IsNull
                                | RelationFilterOperator::IsNotNull => f.value.is_none(),
                                RelationFilterOperator::Equals
                                | RelationFilterOperator::NotEquals => {
                                    f.value.as_deref().is_some_and(|v| {
                                        v.len() <= MAX_PREVIEW_FILTER_VALUE_BYTES
                                            && !v.contains('\0')
                                    })
                                }
                                RelationFilterOperator::Contains => f
                                    .value
                                    .as_deref()
                                    .is_some_and(|v| text(v, MAX_PREVIEW_FILTER_VALUE_BYTES)),
                            }
                    });
                if !required_present
                    || !allowed_keys
                    || t.offset > MAX_PREVIEW_OFFSET
                    || !valid_sort
                    || !valid_filters
                    || [&t.database, &t.schema, &t.name]
                        .into_iter()
                        .any(|s| !identifier(s))
                    || r.expected.database.as_deref() != Some(t.database.as_str())
                    || t.schema == "INFORMATION_SCHEMA"
                    || !matches!(t.table_type.as_str(), "BASE TABLE" | "TEMPORARY TABLE")
                    || !(1..=1000).contains(&cols.len())
                    || unique.len() != cols.len()
                    || cols.iter().any(|c| !identifier(c))
                {
                    return Err("invalid owner preview target");
                }
                (501, 2 * 1024 * 1024)
            }
            kind => {
                let t = r.target.ok_or("missing owner metadata target")?;
                if !exact(
                    &payload["target"],
                    &["database", "schema", "name", "tableType"],
                ) || [&t.database, &t.schema, &t.name]
                    .into_iter()
                    .any(|s| !identifier(s))
                    || r.expected.database.as_deref() != Some(t.database.as_str())
                    || !matches!(t.table_type.as_str(), "BASE TABLE" | "TEMPORARY TABLE")
                {
                    return Err("invalid owner metadata target");
                }
                match kind {
                    Kind::ExactRelation => (3, 2 * 1024 * 1024),
                    Kind::Columns | Kind::Describe => (1001, 1024 * 1024),
                    Kind::ZeroRowResolution => (0, 1024 * 1024),
                    _ => unreachable!(),
                }
            }
        };
        Ok(Self {
            max_rows,
            max_bytes,
            rows: 0,
            bytes: 0,
            described: false,
        })
    }

    pub(crate) fn columns(&mut self, columns: &Value) -> bool {
        let Some(items) = columns.as_array() else {
            return false;
        };
        let valid = items.len() <= 1000
            && items.iter().all(|c| {
                c.as_object().is_some_and(|o| o.len() == 2)
                    && c["name"].as_str().is_some_and(|s| text(s, 4096))
                    && c["type"].as_str().is_some_and(|s| text(s, 128))
            });
        let Ok(bytes) = serde_json::to_vec(columns) else {
            return false;
        };
        if self.described || !valid || bytes.len() + 2 > self.max_bytes {
            return false;
        }
        self.bytes = bytes.len() + 2;
        self.described = true;
        true
    }

    pub(crate) fn batch(&mut self, rows: &Value) -> bool {
        let Some(items) = rows.as_array() else {
            return false;
        };
        let Ok(bytes) = serde_json::to_vec(rows) else {
            return false;
        };
        let Some(count) = self.rows.checked_add(items.len() as u64) else {
            return false;
        };
        let Some(total) = self.bytes.checked_add(bytes.len()) else {
            return false;
        };
        if !self.described || count > self.max_rows || total > self.max_bytes {
            return false;
        }
        self.rows = count;
        self.bytes = total;
        true
    }

    pub(crate) fn terminal(&self, result: &Value) -> bool {
        let Some(object) = result.as_object() else {
            return false;
        };
        let (Some(seen), Some(retained), Some(truncated)) = (
            result["rowsSeen"].as_u64(),
            result["rowsRetained"].as_u64(),
            result["truncated"].as_bool(),
        ) else {
            return false;
        };
        self.described
            && retained == self.rows
            && seen >= retained
            && (truncated || seen == retained)
            && object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "rowsSeen" | "rowsRetained" | "truncated" | "sdkRequestId" | "queryId"
                )
            })
    }
}

#[cfg(test)]
#[path = "snowflake_mutation_read_tests.rs"]
mod tests;
