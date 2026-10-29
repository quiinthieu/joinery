use joinery_core::domain::*;
use ts_rs::TS;

fn main() {
    let definitions = [
        RedisKeyInfo::decl(),
        RedisScanPage::decl(),
        RedisValuePreview::decl(),
        RedisValueEntry::decl(),
        EngineKind::decl(),
        Environment::decl(),
        SafeMode::decl(),
        TlsMode::decl(),
        ConnectionGroup::decl(),
        SshAuthentication::decl(),
        SshHostKey::decl(),
        SshConfig::decl(),
        ConnectionProfile::decl(),
        SnowflakeConfig::decl(),
        ConnectionSecrets::decl(),
        CompletionCatalog::decl(),
        CompletionIdentity::decl(),
        SessionInfo::decl(),
        EngineCapabilities::decl(),
        StatementKind::decl(),
        StatementPlan::decl(),
        PreparedPlan::decl(),
        ColumnKind::decl(),
        ColumnInfo::decl(),
        TableRef::decl(),
        TableInfo::decl(),
        ForeignKey::decl(),
        TableSchema::decl(),
        RetentionStatus::decl(),
        ResultInfo::decl(),
        TablePage::decl(),
        RunStatus::decl(),
        QueryEvent::decl(),
        CellChange::decl(),
        ResultChanges::decl(),
        ChangePreview::decl(),
        ProcessInfo::decl(),
        HistoryEntry::decl(),
        AuditEntry::decl(),
        Bootstrap::decl(),
        SavedWorkspace::decl(),
        QueryDraft::decl(),
        MetadataBackup::decl(),
        ResourceUsage::decl(),
        ErrorCode::decl(),
        AppError::decl(),
    ];
    let source = format!(
        "// Generated from Rust domain types. Run pnpm contracts.\n\n{}\n",
        definitions
            .iter()
            .map(|d| format!("export {d}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/ipc/generated.ts");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}
