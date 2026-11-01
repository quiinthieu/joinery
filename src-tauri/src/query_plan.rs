use crate::domain::{AppError, AppResult, EngineKind, ErrorCode};
use sqlparser::{
    ast::Statement,
    dialect::{Dialect, MySqlDialect, PostgreSqlDialect},
    parser::Parser,
};

/// Generate an estimated plan only. Validate the complete input before adding
/// EXPLAIN so a second statement can never escape the wrapper.
pub(crate) fn estimated_sql(engine: EngineKind, sql: &str) -> AppResult<String> {
    let (dialect, prefix): (Box<dyn Dialect>, &str) = match engine {
        EngineKind::Postgres => (Box::new(PostgreSqlDialect {}), "EXPLAIN (FORMAT JSON)"),
        EngineKind::Mysql => (Box::new(MySqlDialect {}), "EXPLAIN FORMAT=JSON"),
        _ => {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Estimated plans are available for PostgreSQL and MySQL/MariaDB.",
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
            let output = estimated_sql(engine, sql).unwrap();
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
                assert!(estimated_sql(engine, sql).is_err(), "{engine:?}: {sql}");
            }
        }
        assert!(estimated_sql(EngineKind::Mysql, "SELECT 1 /*!; DELETE FROM t */").is_err());
        assert!(estimated_sql(EngineKind::Mysql, "SELECT 1 /*M!; DELETE FROM t */").is_err());
    }

    #[test]
    fn unsupported_engines_and_oversized_input_fail_closed() {
        for engine in [EngineKind::Snowflake, EngineKind::Redis] {
            assert!(estimated_sql(engine, "SELECT 1").is_err());
        }
        assert!(estimated_sql(EngineKind::Postgres, &" ".repeat(1024 * 1024)).is_err());
    }
}
