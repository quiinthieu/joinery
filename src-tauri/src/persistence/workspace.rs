use super::{Persistence, db_error};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

const MAX_BYTES: usize = 2 * 1024 * 1024;

fn validate(workspace: &SavedWorkspace) -> AppResult<()> {
    if !["dark", "light", "contrast"].contains(&workspace.theme.as_str())
        || !["comfortable", "compact"].contains(&workspace.density.as_str())
        || workspace.drafts.len() > 32
    {
        return Err(AppError::invalid(
            "Invalid workspace preferences or more than 32 saved query drafts",
        ));
    }
    let mut ids = HashSet::new();
    let mut bytes = 0;
    for draft in &workspace.drafts {
        if uuid::Uuid::parse_str(&draft.id).is_err()
            || uuid::Uuid::parse_str(&draft.connection_id).is_err()
            || !ids.insert(&draft.id)
            || draft.title.len() > 100
            || draft.sql.len() > 1024 * 1024
        {
            return Err(AppError::invalid(
                "A query draft is invalid or larger than 1 MiB",
            ));
        }
        bytes += draft.sql.len() + draft.title.len();
        if bytes > MAX_BYTES {
            return Err(AppError::invalid("Saved query drafts are limited to 2 MiB"));
        }
    }
    if workspace
        .active_draft_id
        .as_ref()
        .is_some_and(|id| !ids.contains(id))
    {
        return Err(AppError::invalid("The selected query draft is unavailable"));
    }
    Ok(())
}

pub(super) fn read_workspace(connection: &Connection) -> AppResult<Option<SavedWorkspace>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key='workspace'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    json.map(|json| {
        if json.len() > MAX_BYTES * 6 + 16384 {
            return Err(AppError::invalid("Saved workspace is too large"));
        }
        let workspace: SavedWorkspace = serde_json::from_str(&json).map_err(|_| {
            AppError::invalid(
                "Saved workspace metadata is incompatible; keep a backup before recovery",
            )
        })?;
        validate(&workspace)?;
        Ok(workspace)
    })
    .transpose()
}

fn write_workspace(connection: &Connection, workspace: &SavedWorkspace) -> AppResult<()> {
    let json = serde_json::to_string(workspace)
        .map_err(|_| AppError::internal("Could not encode workspace preferences"))?;
    connection.execute("INSERT INTO settings VALUES('workspace',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&json]).map_err(db_error)?;
    Ok(())
}

pub(super) fn purge_drafts(connection: &Connection, id: &str) -> AppResult<()> {
    if let Some(mut workspace) = read_workspace(connection)? {
        workspace.drafts.retain(|draft| draft.connection_id != id);
        if !workspace
            .drafts
            .iter()
            .any(|draft| Some(&draft.id) == workspace.active_draft_id.as_ref())
        {
            workspace.active_draft_id = None;
        }
        write_workspace(connection, &workspace)?;
    }
    Ok(())
}

fn enforce_opt_in(connection: &Connection, workspace: &mut SavedWorkspace) -> AppResult<()> {
    let mut statement = connection
        .prepare("SELECT json FROM profiles")
        .map_err(db_error)?;
    let profiles = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?;
    let mut allowed = HashSet::new();
    for profile in profiles {
        let profile: ConnectionProfile = serde_json::from_str(&profile.map_err(db_error)?)
            .map_err(|_| AppError::internal("Application profile data is invalid"))?;
        if profile.store_drafts {
            allowed.insert(profile.id);
        }
    }
    workspace
        .drafts
        .retain(|draft| allowed.contains(&draft.connection_id));
    if !workspace
        .drafts
        .iter()
        .any(|draft| Some(&draft.id) == workspace.active_draft_id.as_ref())
    {
        workspace.active_draft_id = None;
    }
    Ok(())
}

impl Persistence {
    pub fn workspace(&self) -> AppResult<Option<SavedWorkspace>> {
        let connection = self.connection.lock();
        let mut workspace = read_workspace(&connection)?;
        if let Some(workspace) = &mut workspace {
            enforce_opt_in(&connection, workspace)?;
        }
        Ok(workspace)
    }

    pub fn save_workspace(&self, mut workspace: SavedWorkspace) -> AppResult<()> {
        validate(&workspace)?;
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        enforce_opt_in(&tx, &mut workspace)?;
        write_workspace(&tx, &workspace)?;
        tx.commit().map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(enabled: bool) -> ConnectionProfile {
        serde_json::from_value(serde_json::json!({
            "id": new_id(), "name":"Draft fixture", "groupId":"local", "engine":"postgres",
            "host":"127.0.0.1", "port":55432, "database":"joinery", "username":"joinery",
            "environment":"local", "safeMode":"readOnly", "tlsMode":"disable", "caCertPath":null,
            "ssh":null, "rememberPassword":false, "storeQueryText":false, "storeDrafts":enabled
        }))
        .unwrap()
    }
    fn workspace(profile: &ConnectionProfile) -> SavedWorkspace {
        let id = new_id();
        SavedWorkspace {
            theme: "contrast".into(),
            density: "compact".into(),
            sidebar_visible: true,
            inspector_visible: true,
            active_draft_id: Some(id.clone()),
            drafts: vec![QueryDraft {
                id,
                connection_id: profile.id.clone(),
                title: "Draft".into(),
                sql: "SELECT 'synthetic-sensitive-literal'".into(),
            }],
        }
    }

    #[test]
    fn drafts_are_opt_in_and_revocation_purges_text_before_stale_saves() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        let mut profile = profile(false);
        db.save_profile(&profile).unwrap();
        db.save_workspace(workspace(&profile)).unwrap();
        assert!(db.workspace().unwrap().unwrap().drafts.is_empty());
        assert!(
            !db.setting("workspace")
                .unwrap()
                .unwrap()
                .contains("synthetic-sensitive-literal")
        );
        profile.store_drafts = true;
        db.save_profile(&profile).unwrap();
        db.save_workspace(workspace(&profile)).unwrap();
        assert_eq!(db.workspace().unwrap().unwrap().drafts.len(), 1);
        profile.store_drafts = false;
        db.save_profile(&profile).unwrap();
        assert!(
            !db.setting("workspace")
                .unwrap()
                .unwrap()
                .contains("synthetic-sensitive-literal")
        );
        db.save_workspace(workspace(&profile)).unwrap();
        assert!(db.workspace().unwrap().unwrap().drafts.is_empty());
        profile.store_drafts = true;
        db.save_profile(&profile).unwrap();
        db.save_workspace(workspace(&profile)).unwrap();
        db.delete_profile(&profile.id).unwrap();
        assert!(db.workspace().unwrap().unwrap().drafts.is_empty());
        assert!(
            !db.setting("workspace")
                .unwrap()
                .unwrap()
                .contains("synthetic-sensitive-literal")
        );
    }

    #[test]
    fn workspace_round_trips_across_restart_without_sessions_or_results() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        let profile = profile(true);
        db.save_profile(&profile).unwrap();
        let original = workspace(&profile);
        db.save_workspace(original.clone()).unwrap();
        drop(db);
        let db = Persistence::open(root.path()).unwrap();
        let restored = db.workspace().unwrap().unwrap();
        assert_eq!(restored.drafts[0].sql, original.drafts[0].sql);
        assert_eq!(restored.theme, "contrast");
        assert_eq!(restored.density, "compact");
        assert_eq!(restored.active_draft_id, original.active_draft_id);
        let stored = db.setting("workspace").unwrap().unwrap();
        assert!(!stored.contains("result"));
        assert!(!stored.contains("session"));
    }

    #[test]
    fn malformed_and_oversized_workspaces_leave_the_previous_snapshot_intact() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        let profile = profile(true);
        db.save_profile(&profile).unwrap();
        db.save_workspace(workspace(&profile)).unwrap();
        let before = db.setting("workspace").unwrap();
        let mut invalid = workspace(&profile);
        invalid.drafts[0].sql = "x".repeat(1024 * 1024 + 1);
        assert!(db.save_workspace(invalid).is_err());
        let mut invalid = workspace(&profile);
        invalid.drafts.push(invalid.drafts[0].clone());
        assert!(db.save_workspace(invalid).is_err());
        let mut invalid = workspace(&profile);
        invalid.theme = "unknown".into();
        assert!(db.save_workspace(invalid).is_err());
        assert_eq!(db.setting("workspace").unwrap(), before);
        assert!(serde_json::from_value::<SavedWorkspace>(serde_json::json!({"theme":"dark","density":"comfortable","sidebarVisible":true,"inspectorVisible":false,"activeDraftId":null,"drafts":[],"results":["must-not-persist"]})).is_err());
        let mut legacy = serde_json::to_value(profile).unwrap();
        legacy.as_object_mut().unwrap().remove("storeDrafts");
        assert!(
            !serde_json::from_value::<ConnectionProfile>(legacy)
                .unwrap()
                .store_drafts
        );
    }
}
