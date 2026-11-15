//! Fixed SQL compiled from the same bundled contract read by the Node helper.
//! Invalid contracts fail the build; no runtime override is accepted.
include!(concat!(env!("OUT_DIR"), "/snowflake_sql_contract.rs"));

#[cfg(test)]
mod tests {
    #[test]
    fn generated_constants_match_the_bundled_helper_contract() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../snowflake-helper/src/sql-contract.json"
        ))
        .unwrap();
        for (key, sql) in [
            ("bootstrapContext", super::BOOTSTRAP_CONTEXT_SQL),
            ("version", super::VERSION_SQL),
            ("begin", super::BEGIN_SQL),
            ("commit", super::COMMIT_SQL),
            ("rollback", super::ROLLBACK_SQL),
        ] {
            assert_eq!(contract[key].as_str(), Some(sql), "{key}");
        }
    }
}
