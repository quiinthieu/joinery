use crate::domain::{ActualPlanFormat, AppError, AppResult, EngineKind, ErrorCode};
use sqlparser::{
    ast::Statement,
    dialect::{Dialect, MySqlDialect, PostgreSqlDialect},
    parser::Parser,
};

/// Validate the complete input before adding EXPLAIN so a second statement
/// can never escape the wrapper. Executing plans still require safety review.
pub(crate) fn explain_sql(
    engine: EngineKind,
    sql: &str,
    analyze: bool,
    actual_plan: Option<ActualPlanFormat>,
) -> AppResult<String> {
    let (dialect, prefix): (Box<dyn Dialect>, &str) = match (engine, analyze) {
        (EngineKind::Postgres, false) => (Box::new(PostgreSqlDialect {}), "EXPLAIN (FORMAT JSON)"),
        (EngineKind::Postgres, true) => (
            Box::new(PostgreSqlDialect {}),
            "EXPLAIN (ANALYZE, FORMAT JSON)",
        ),
        (EngineKind::Mysql, false) => (Box::new(MySqlDialect {}), "EXPLAIN FORMAT=JSON"),
        (EngineKind::Mysql, true) if actual_plan == Some(ActualPlanFormat::MariaDbJson) => {
            (Box::new(MySqlDialect {}), "ANALYZE FORMAT=JSON")
        }
        _ => {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                if analyze {
                    "Actual plans are available for PostgreSQL and capability-checked MariaDB sessions."
                } else {
                    "Estimated plans are available for PostgreSQL and MySQL/MariaDB."
                },
            ));
        }
    };
    if sql.len() > 1024 * 1024 - prefix.len() - 1 {
        return Err(AppError::invalid("A query must be smaller than 1 MiB"));
    }
    if engine == EngineKind::Mysql && crate::safety::has_mysql_executable_comment(sql) {
        return Err(AppError::invalid("Executable comments cannot be explained"));
    }
    let statements = Parser::parse_sql(dialect.as_ref(), sql)
        .map_err(|_| AppError::invalid("Select one complete query to explain"))?;
    if statements.len() != 1 || !matches!(&statements[0], Statement::Query(_)) {
        return Err(AppError::invalid(
            "Select one SELECT or WITH query to explain",
        ));
    }
    Ok(format!("{prefix}\n{sql}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_query_without_rewriting_literals_or_comments() {
        for engine in [EngineKind::Postgres, EngineKind::Mysql] {
            let sql = "/* keep */ WITH q AS (SELECT ';' AS value) SELECT * FROM q; -- end";
            let output = explain_sql(engine, sql, false, None).unwrap();
            assert!(output.ends_with(sql));
            assert!(!output.contains("ANALYZE"));
        }
    }

    #[test]
    fn rejects_batches_mutations_existing_explain_and_executable_comments() {
        for engine in [EngineKind::Postgres, EngineKind::Mysql] {
            for sql in [
                "SELECT 1; DELETE FROM t",
                "SELECT 1; SELECT 2",
                "DELETE FROM t",
                "EXPLAIN ANALYZE SELECT 1",
                "",
                "-- nothing",
                "SELECT '",
            ] {
                assert!(
                    explain_sql(engine, sql, false, None).is_err(),
                    "{engine:?}: {sql}"
                );
            }
        }
        assert!(
            explain_sql(
                EngineKind::Mysql,
                "SELECT 1 /*!; DELETE FROM t */",
                false,
                None
            )
            .is_err()
        );
        assert!(
            explain_sql(
                EngineKind::Mysql,
                "SELECT 1 /*M!; DELETE FROM t */",
                false,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn unsupported_engines_and_oversized_input_fail_closed() {
        for engine in [EngineKind::Snowflake, EngineKind::Redis] {
            assert!(explain_sql(engine, "SELECT 1", false, None).is_err());
        }
        assert!(explain_sql(EngineKind::Postgres, &" ".repeat(1024 * 1024), false, None).is_err());
    }

    #[test]
    fn actual_plan_requires_a_supported_engine_format_and_validates_the_whole_query() {
        let sql = "/* preserve */ SELECT 'é; value'; -- end";
        assert_eq!(
            explain_sql(EngineKind::Postgres, sql, true, None).unwrap(),
            format!("EXPLAIN (ANALYZE, FORMAT JSON)\n{sql}")
        );
        for engine in [EngineKind::Mysql, EngineKind::Snowflake, EngineKind::Redis] {
            assert_eq!(
                explain_sql(engine, "SELECT 1", true, None)
                    .unwrap_err()
                    .code,
                ErrorCode::Unsupported
            );
        }
        for sql in [
            "SELECT 1; SELECT 2",
            "DELETE FROM t",
            "EXPLAIN SELECT 1",
            "SELECT '",
        ] {
            assert!(
                explain_sql(EngineKind::Postgres, sql, true, None).is_err(),
                "{sql}"
            );
        }
        let format = Some(ActualPlanFormat::MariaDbJson);
        assert_eq!(
            explain_sql(EngineKind::Mysql, sql, true, format).unwrap(),
            format!("ANALYZE FORMAT=JSON\n{sql}")
        );
        for sql in [
            "SELECT 1; SELECT 2",
            "DELETE FROM t",
            "ANALYZE SELECT 1",
            "SELECT 1 /*!; DELETE FROM t */",
            "SELECT 1 /*M!; DELETE FROM t */",
            "SELECT '",
        ] {
            assert!(
                explain_sql(EngineKind::Mysql, sql, true, format).is_err(),
                "{sql}"
            );
        }
        assert_eq!(
            explain_sql(
                EngineKind::Mysql,
                sql,
                true,
                Some(ActualPlanFormat::PostgresJson)
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
    }
}
