//! Typed, parameterized sort/filter/keyset SQL for table browsing.
//!
//! Identifiers come only from the validated `TableSchema` and are quoted by the
//! dialect; every user value is a bound parameter. NULL ordering is explicit
//! (ascending NULLS LAST, descending NULLS FIRST) on every engine, and the seek
//! predicate mirrors that order so pages neither skip nor repeat rows.

use crate::domain::*;

pub(crate) const MAX_SORT_KEYS: usize = 8;
pub(crate) const MAX_FILTERS: usize = 16;
pub(crate) const MAX_FILTER_VALUE_BYTES: usize = 64 * 1024;
const LIKE_ESCAPE: char = '!';

/// Engine-specific SQL spelling. `index` is the zero-based bind position.
pub(crate) trait Dialect {
    /// Column reference usable in WHERE and ORDER BY of the browse query.
    fn column(&self, name: &str) -> String;
    /// Typed bind comparable with `column` using the column's own ordering,
    /// or `None` when the type cannot be compared or ordered safely.
    fn typed_bind(&self, column: &ColumnInfo, index: usize) -> Option<String>;
    /// Case-insensitive (PostgreSQL) or collation-following (MySQL) LIKE test.
    fn like(&self, column: &ColumnInfo, index: usize) -> String;
    fn order_term(&self, expression: &str, descending: bool) -> String;
}

pub(crate) struct Built {
    /// Empty, or ` WHERE ...`.
    pub where_sql: String,
    /// Empty, or ` ORDER BY ...`.
    pub order_sql: String,
    pub binds: Vec<Option<String>>,
    /// Column positions of the effective ordering, in cursor order.
    pub keys: Vec<usize>,
    /// Whether keyset continuation is available (supported primary key).
    pub pageable: bool,
}

impl Built {
    /// Continuation cursor for the last row of a page.
    pub fn cursor(&self, row: &TextRow) -> Vec<Option<String>> {
        self.keys.iter().map(|&i| row[i].clone()).collect()
    }
}

fn position(schema: &TableSchema, name: &str) -> AppResult<usize> {
    schema
        .columns
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| AppError::invalid(format!("Unknown column {name:?}")))
}

fn orderable(column: &ColumnInfo, dialect: &dyn Dialect) -> bool {
    !matches!(column.kind, ColumnKind::Json | ColumnKind::Binary)
        && dialect.typed_bind(column, 0).is_some()
}

fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_') || ch == LIKE_ESCAPE {
            escaped.push(LIKE_ESCAPE);
        }
        escaped.push(ch);
    }
    escaped
}

pub(crate) fn like_escape_clause() -> String {
    format!(" ESCAPE '{LIKE_ESCAPE}'")
}

struct Binds(Vec<Option<String>>);
impl Binds {
    fn push(&mut self, value: Option<String>) -> usize {
        self.0.push(value);
        self.0.len() - 1
    }
}

pub(crate) fn build(
    schema: &TableSchema,
    query: &BrowseQuery,
    after: Option<&[Option<String>]>,
    dialect: &dyn Dialect,
) -> AppResult<Built> {
    if query.sort.len() > MAX_SORT_KEYS {
        return Err(AppError::invalid(format!(
            "Sort by at most {MAX_SORT_KEYS} columns"
        )));
    }
    if query.filters.len() > MAX_FILTERS {
        return Err(AppError::invalid(format!(
            "Use at most {MAX_FILTERS} filters"
        )));
    }
    let pageable = !schema.primary_key.is_empty()
        && schema
            .primary_key
            .iter()
            .all(|key| position(schema, key).is_ok_and(|i| orderable(&schema.columns[i], dialect)));

    // Effective ordering: user keys, then primary-key tie-breakers.
    let mut order: Vec<(usize, bool)> = Vec::new();
    for key in &query.sort {
        let i = position(schema, &key.column)?;
        if !orderable(&schema.columns[i], dialect) {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                format!("Column {:?} cannot be sorted", key.column),
            ));
        }
        if order.iter().any(|(j, _)| *j == i) {
            return Err(AppError::invalid(format!(
                "Column {:?} is sorted more than once",
                key.column
            )));
        }
        order.push((i, key.descending));
    }
    if pageable {
        for key in &schema.primary_key {
            let i = position(schema, key)?;
            if !order.iter().any(|(j, _)| *j == i) {
                order.push((i, false));
            }
        }
    }

    let mut binds = Binds(Vec::new());
    let mut predicates = Vec::new();
    for filter in &query.filters {
        let i = position(schema, &filter.column)?;
        let column = &schema.columns[i];
        let expression = dialect.column(&column.name);
        let value = match (&filter.op, &filter.value) {
            (FilterOp::IsNull | FilterOp::IsNotNull, None) => None,
            (FilterOp::IsNull | FilterOp::IsNotNull, Some(_)) => {
                return Err(AppError::invalid("NULL tests do not take a value"));
            }
            (_, None) => return Err(AppError::invalid("This filter needs a value")),
            (_, Some(value)) if value.len() > MAX_FILTER_VALUE_BYTES => {
                return Err(AppError::invalid("A filter value exceeds 64 KiB"));
            }
            (_, Some(value)) => Some(value.as_str()),
        };
        let predicate = match filter.op {
            FilterOp::IsNull => format!("{expression} IS NULL"),
            FilterOp::IsNotNull => format!("{expression} IS NOT NULL"),
            FilterOp::Contains | FilterOp::StartsWith => {
                if column.kind == ColumnKind::Binary {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        format!("Column {:?} cannot be searched as text", column.name),
                    ));
                }
                let escaped = escape_like(value.unwrap_or_default());
                let pattern = if filter.op == FilterOp::Contains {
                    format!("%{escaped}%")
                } else {
                    format!("{escaped}%")
                };
                let index = binds.push(Some(pattern));
                dialect.like(column, index)
            }
            _ => {
                if !orderable(column, dialect) {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        format!("Column {:?} cannot be compared", column.name),
                    ));
                }
                let operator = match filter.op {
                    FilterOp::Eq => "=",
                    FilterOp::NotEq => "<>",
                    FilterOp::Lt => "<",
                    FilterOp::Lte => "<=",
                    FilterOp::Gt => ">",
                    _ => ">=",
                };
                let index = binds.push(value.map(str::to_owned));
                let bind = dialect.typed_bind(column, index).unwrap_or_default();
                format!("{expression} {operator} {bind}")
            }
        };
        predicates.push(predicate);
    }

    if let Some(after) = after {
        if !pageable || after.len() != order.len() {
            return Err(AppError::invalid("Invalid table paging key"));
        }
        let pk_positions = schema
            .primary_key
            .iter()
            .map(|key| position(schema, key))
            .collect::<AppResult<Vec<_>>>()?;
        if order
            .iter()
            .zip(after)
            .any(|((i, _), value)| value.is_none() && pk_positions.contains(i))
        {
            return Err(AppError::invalid("Invalid table paging key"));
        }
        predicates.push(seek(
            schema,
            &order,
            after,
            query.sort.is_empty(),
            dialect,
            &mut binds,
        ));
    }

    let where_sql = if predicates.is_empty() {
        String::new()
    } else {
        format!(
            " WHERE {}",
            predicates
                .iter()
                .map(|p| format!("({p})"))
                .collect::<Vec<_>>()
                .join(" AND ")
        )
    };
    let order_sql = if order.is_empty() {
        String::new()
    } else {
        format!(
            " ORDER BY {}",
            order
                .iter()
                .map(|(i, descending)| {
                    dialect.order_term(&dialect.column(&schema.columns[*i].name), *descending)
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    Ok(Built {
        where_sql,
        order_sql,
        binds: binds.0,
        keys: order.into_iter().map(|(i, _)| i).collect(),
        pageable,
    })
}

/// Lexicographic "strictly after the cursor" predicate under the fixed NULL
/// order. Without user sort keys every key is a non-null ascending primary-key
/// column, so the index-friendly row-value comparison is equivalent.
fn seek(
    schema: &TableSchema,
    order: &[(usize, bool)],
    after: &[Option<String>],
    primary_key_only: bool,
    dialect: &dyn Dialect,
    binds: &mut Binds,
) -> String {
    let column = |i: usize| &schema.columns[i];
    if primary_key_only {
        let mut columns = Vec::new();
        let mut values = Vec::new();
        for ((i, _), value) in order.iter().zip(after) {
            columns.push(dialect.column(&column(*i).name));
            let index = binds.push(value.clone());
            values.push(dialect.typed_bind(column(*i), index).unwrap_or_default());
        }
        return format!("({}) > ({})", columns.join(","), values.join(","));
    }
    let mut alternatives = Vec::new();
    for (n, ((i, descending), value)) in order.iter().zip(after).enumerate() {
        if !descending && value.is_none() {
            // Nothing sorts after NULL ascending except ties on later keys.
            continue;
        }
        let expression = dialect.column(&column(*i).name);
        // Bind in textual order: prior equalities, then this key's step.
        let mut terms = Vec::new();
        for ((j, _), prior) in order[..n].iter().zip(after) {
            let prior_expression = dialect.column(&column(*j).name);
            terms.push(match prior {
                None => format!("{prior_expression} IS NULL"),
                Some(value) => {
                    let index = binds.push(Some(value.clone()));
                    let bind = dialect.typed_bind(column(*j), index).unwrap_or_default();
                    format!("{prior_expression} = {bind}")
                }
            });
        }
        terms.push(match (descending, value) {
            (true, None) => format!("{expression} IS NOT NULL"),
            (false, Some(value)) => {
                let index = binds.push(Some(value.clone()));
                let bind = dialect.typed_bind(column(*i), index).unwrap_or_default();
                format!("({expression} > {bind} OR {expression} IS NULL)")
            }
            (_, value) => {
                let index = binds.push(value.clone());
                let bind = dialect.typed_bind(column(*i), index).unwrap_or_default();
                format!("{expression} < {bind}")
            }
        });
        alternatives.push(format!("({})", terms.join(" AND ")));
    }
    if alternatives.is_empty() {
        // Only reachable for an all-NULL ascending cursor without a primary
        // key tie-breaker, which `build` rejects; stay conservative.
        "FALSE".into()
    } else {
        alternatives.join(" OR ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::{mysql::MysqlDialect, postgres::PostgresDialect};

    fn column(name: &str, database_type: &str, kind: ColumnKind, primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            id: name.into(),
            name: name.into(),
            database_type: database_type.into(),
            kind,
            nullable: !primary_key,
            primary_key,
            foreign_key: false,
            editable: false,
        }
    }

    fn schema(types: (&str, &str, &str)) -> TableSchema {
        TableSchema {
            table: TableRef {
                schema: "public".into(),
                name: "browse_sort".into(),
            },
            columns: vec![
                column("id", types.0, ColumnKind::Integer, true),
                column("bucket", types.1, ColumnKind::Integer, false),
                column("label", types.2, ColumnKind::Text, false),
                column("payload", "jsonb", ColumnKind::Json, false),
                column("raw", "bytea", ColumnKind::Binary, false),
            ],
            primary_key: vec!["id".into()],
            foreign_keys: vec![],
            editable: true,
        }
    }

    fn pg() -> TableSchema {
        schema(("bigint", "integer", "text"))
    }

    fn sort(column: &str, descending: bool) -> SortKey {
        SortKey {
            column: column.into(),
            descending,
        }
    }

    fn filter(column: &str, op: FilterOp, value: Option<&str>) -> ColumnFilter {
        ColumnFilter {
            column: column.into(),
            op,
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn primary_key_only_keeps_the_row_value_fast_path() {
        let built = build(
            &pg(),
            &BrowseQuery::default(),
            Some(&[Some("10".into())]),
            &PostgresDialect,
        )
        .unwrap();
        assert_eq!(
            built.where_sql,
            " WHERE ((__joinery_source.\"id\") > (($1::text)::bigint))"
        );
        assert_eq!(
            built.order_sql,
            " ORDER BY __joinery_source.\"id\" ASC NULLS LAST"
        );
        assert_eq!(built.binds, [Some("10".into())]);
        assert!(built.pageable);
    }

    #[test]
    fn user_sort_adds_primary_key_tie_breaker_and_explicit_null_order() {
        let query = BrowseQuery {
            sort: vec![sort("bucket", true), sort("label", false)],
            filters: vec![],
        };
        let built = build(&pg(), &query, None, &PostgresDialect).unwrap();
        assert_eq!(
            built.order_sql,
            " ORDER BY __joinery_source.\"bucket\" DESC NULLS FIRST,__joinery_source.\"label\" ASC NULLS LAST,__joinery_source.\"id\" ASC NULLS LAST"
        );
        assert_eq!(built.keys, [1, 2, 0]);
    }

    #[test]
    fn seek_predicate_handles_null_cursor_values_in_both_directions() {
        let query = BrowseQuery {
            sort: vec![sort("bucket", true), sort("label", false)],
            filters: vec![],
        };
        // bucket NULL (descending: NULLs first), label 'x' (ascending), id 7.
        let built = build(
            &pg(),
            &query,
            Some(&[None, Some("x".into()), Some("7".into())]),
            &PostgresDialect,
        )
        .unwrap();
        let b = "__joinery_source.\"bucket\"";
        let l = "__joinery_source.\"label\"";
        let i = "__joinery_source.\"id\"";
        assert_eq!(
            built.where_sql,
            format!(
                " WHERE (({b} IS NOT NULL) OR ({b} IS NULL AND ({l} > ($1::text)::text OR {l} IS NULL)) OR ({b} IS NULL AND {l} = ($2::text)::text AND ({i} > ($3::text)::bigint OR {i} IS NULL)))"
            )
        );
        assert_eq!(
            built.binds,
            [Some("x".into()), Some("x".into()), Some("7".into())]
        );
    }

    #[test]
    fn ascending_null_cursor_value_only_continues_through_ties() {
        let query = BrowseQuery {
            sort: vec![sort("bucket", false)],
            filters: vec![],
        };
        let built = build(
            &pg(),
            &query,
            Some(&[None, Some("7".into())]),
            &PostgresDialect,
        )
        .unwrap();
        let b = "__joinery_source.\"bucket\"";
        let i = "__joinery_source.\"id\"";
        assert_eq!(
            built.where_sql,
            format!(" WHERE (({b} IS NULL AND ({i} > ($1::text)::bigint OR {i} IS NULL)))")
        );
    }

    #[test]
    fn filters_are_bound_and_like_patterns_are_escaped() {
        let query = BrowseQuery {
            sort: vec![],
            filters: vec![
                filter("label", FilterOp::Contains, Some("50%_off!")),
                filter("bucket", FilterOp::Gte, Some("3")),
                filter("payload", FilterOp::IsNull, None),
            ],
        };
        let built = build(&pg(), &query, None, &PostgresDialect).unwrap();
        assert_eq!(
            built.where_sql,
            " WHERE (__joinery_source.\"label\"::text ILIKE $1::text ESCAPE '!') AND (__joinery_source.\"bucket\" >= ($2::text)::bigint) AND (__joinery_source.\"payload\" IS NULL)"
        );
        assert_eq!(
            built.binds,
            [Some("%50!%!_off!!%".into()), Some("3".into())]
        );
    }

    #[test]
    fn hostile_values_stay_bound_and_unknown_columns_are_rejected() {
        let hostile = "'); DROP TABLE browse_sort; --";
        let query = BrowseQuery {
            sort: vec![],
            filters: vec![filter("label", FilterOp::Eq, Some(hostile))],
        };
        let built = build(&pg(), &query, None, &PostgresDialect).unwrap();
        assert!(!built.where_sql.contains("DROP"));
        assert_eq!(built.binds, [Some(hostile.into())]);
        let unknown = BrowseQuery {
            sort: vec![sort("label\"; DROP TABLE x; --", false)],
            filters: vec![],
        };
        assert!(build(&pg(), &unknown, None, &PostgresDialect).is_err());
    }

    #[test]
    fn unsupported_operations_and_limits_are_rejected() {
        let reject = |query: BrowseQuery| build(&pg(), &query, None, &PostgresDialect).is_err();
        assert!(reject(BrowseQuery {
            sort: vec![sort("payload", false)],
            filters: vec![],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![sort("bucket", false), sort("bucket", true)],
            filters: vec![],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: vec![filter("raw", FilterOp::Contains, Some("00"))],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: vec![filter("payload", FilterOp::Gt, Some("1"))],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: vec![filter("bucket", FilterOp::Eq, None)],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: vec![filter("bucket", FilterOp::IsNull, Some("1"))],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: vec![filter(
                "label",
                FilterOp::Eq,
                Some(&"x".repeat(MAX_FILTER_VALUE_BYTES + 1))
            )],
        }));
        assert!(reject(BrowseQuery {
            sort: vec![],
            filters: (0..=MAX_FILTERS)
                .map(|_| filter("bucket", FilterOp::IsNull, None))
                .collect(),
        }));
    }

    #[test]
    fn cursor_shape_is_validated() {
        let query = BrowseQuery {
            sort: vec![sort("bucket", false)],
            filters: vec![],
        };
        assert!(build(&pg(), &query, Some(&[Some("1".into())]), &PostgresDialect).is_err());
        assert!(
            build(
                &pg(),
                &query,
                Some(&[Some("1".into()), None]),
                &PostgresDialect
            )
            .is_err()
        );
        let mut keyless = pg();
        keyless.primary_key.clear();
        let built = build(&keyless, &query, None, &PostgresDialect).unwrap();
        assert!(!built.pageable);
        assert!(
            build(
                &keyless,
                &query,
                Some(&[Some("1".into())]),
                &PostgresDialect
            )
            .is_err()
        );
    }

    #[test]
    fn mysql_emulates_null_order_and_binds_text_with_column_collation() {
        let schema = schema(("bigint", "int", "varchar(20)"));
        let query = BrowseQuery {
            sort: vec![sort("bucket", true), sort("label", false)],
            filters: vec![filter("label", FilterOp::StartsWith, Some("a"))],
        };
        let built = build(
            &schema,
            &query,
            Some(&[Some("3".into()), Some("b".into()), Some("9".into())]),
            &MysqlDialect,
        )
        .unwrap();
        assert_eq!(
            built.order_sql,
            " ORDER BY `bucket` IS NULL DESC,`bucket` DESC,`label` IS NULL,`label` ASC,`id` IS NULL,`id` ASC"
        );
        assert!(
            built
                .where_sql
                .starts_with(" WHERE (`label` LIKE ? ESCAPE '!') AND (")
        );
        assert!(built.where_sql.contains("`bucket` < CAST(? AS SIGNED)"));
        assert!(built.where_sql.contains("`label` > ?"));
        assert_eq!(built.binds[0].as_deref(), Some("a%"));
    }
}
