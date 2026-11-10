use super::*;
use crate::secrets::MemorySecrets;

#[test]
fn restore_requires_confirmation_current_passphrase_and_an_idle_studio() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let snapshot = studio.create_metadata_backup().unwrap();
    assert!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "", None)
            .is_err()
    );
    studio
        .set_passphrase("fixture-passphrase-123", None)
        .unwrap();
    assert_eq!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "RESTORE", None)
            .unwrap_err()
            .code,
        ErrorCode::PassphraseRequired
    );
    assert_eq!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "RESTORE", Some("incorrect"))
            .unwrap_err()
            .code,
        ErrorCode::InvalidPassphrase
    );
    let connecting = studio.connection_attempts.reserve("fixture", 0, 0).unwrap();
    assert!(
        studio
            .restore_metadata_backup(
                &snapshot.id,
                &snapshot.digest,
                "RESTORE",
                Some("fixture-passphrase-123")
            )
            .is_err()
    );
    studio.cancel_connection_attempt(&connecting);
    let result = studio.results.create("fixture", 0, vec![], None).unwrap();
    assert!(
        studio
            .restore_metadata_backup(
                &snapshot.id,
                &snapshot.digest,
                "RESTORE",
                Some("fixture-passphrase-123")
            )
            .is_err()
    );
    studio.results.release(&result.id);
    studio
        .restore_metadata_backup(
            &snapshot.id,
            &snapshot.digest,
            "RESTORE",
            Some("fixture-passphrase-123"),
        )
        .unwrap();
    assert!(studio.bootstrap().unwrap().passphrase_configured);
    assert!(
        studio
            .verify_passphrase(Some("fixture-passphrase-123"))
            .is_ok()
    );
    assert_eq!(studio.usage().session_count, 0);
}
