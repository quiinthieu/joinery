//! Admission and irreversible history privacy, independent of original receipts.
use super::super::{Persistence, db_error, schema_version};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(in crate::persistence) const SCHEMA: &[(&str, &str)] = &[
    (
        "history_privacy_v5",
        "CREATE TABLE history_privacy_v5(id INTEGER PRIMARY KEY CHECK(id=1),lineage TEXT NOT NULL,current_era TEXT NOT NULL,pristine INTEGER NOT NULL CHECK(pristine IN (0,1)))",
    ),
    (
        "history_eras_v5",
        "CREATE TABLE history_eras_v5(era TEXT PRIMARY KEY NOT NULL,parent TEXT)",
    ),
    (
        "history_profile_privacy_v5",
        "CREATE TABLE history_profile_privacy_v5(connection_id TEXT PRIMARY KEY NOT NULL,generation TEXT NOT NULL)",
    ),
    (
        "history_generations_v5",
        "CREATE TABLE history_generations_v5(generation TEXT PRIMARY KEY NOT NULL,connection_id TEXT NOT NULL,parent TEXT)",
    ),
    (
        "history_tickets_v5",
        "CREATE TABLE history_tickets_v5(id TEXT PRIMARY KEY NOT NULL,seed_json TEXT NOT NULL,lineage TEXT NOT NULL,era TEXT NOT NULL,profile_generation TEXT NOT NULL,sql_digest TEXT,redacted INTEGER NOT NULL CHECK(redacted IN (0,1)),disposition TEXT NOT NULL CHECK(disposition IN ('pending','applied','suppressed')),completion_json TEXT)",
    ),
    (
        "console_lifecycle_v5",
        "CREATE TABLE console_lifecycle_v5(run_id TEXT PRIMARY KEY NOT NULL,ticket_id TEXT,legacy INTEGER NOT NULL CHECK(legacy IN (0,1)),publisher_sealed INTEGER NOT NULL CHECK(publisher_sealed IN (0,1)))",
    ),
];
const MAX_HISTORY_SQL_BYTES: usize = 1_048_576;
const MAX_HISTORY_JSON_BYTES: usize = 6 * MAX_HISTORY_SQL_BYTES + 65_536;

const GUARDS: &[(&str, &str)] = &[
    (
        "console_publisher_run_guard_v5",
        "CREATE TRIGGER console_publisher_run_guard_v5 BEFORE UPDATE OF revision,state,observations_json,terminal_json ON snowflake_console_runs_v5 WHEN EXISTS(SELECT 1 FROM console_lifecycle_v5 WHERE run_id=OLD.run_id AND publisher_sealed=1) BEGIN SELECT RAISE(ABORT,'console publisher sealed'); END",
    ),
    (
        "console_publisher_step_guard_v5",
        "CREATE TRIGGER console_publisher_step_guard_v5 BEFORE UPDATE ON snowflake_console_steps_v5 WHEN EXISTS(SELECT 1 FROM console_lifecycle_v5 WHERE run_id=OLD.run_id AND publisher_sealed=1) BEGIN SELECT RAISE(ABORT,'console publisher sealed'); END",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryDisposition {
    Pending,
    Applied,
    Suppressed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryCompletion {
    pub provenance: HistoryProvenance,
    pub status: Option<RunStatus>,
    pub elapsed_ms: Option<u32>,
    pub execution_summary: Option<ExecutionSummary>,
    pub snowflake_dml: Option<SnowflakeDmlEvidence>,
    pub snowflake_ddl: Option<SnowflakeDdlEvidence>,
}
impl HistoryCompletion {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (serde_json::to_value(self), serde_json::to_value(other)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
    pub(crate) fn from_entry(entry: &HistoryEntry) -> Self {
        Self {
            provenance: entry.provenance,
            status: entry.status,
            elapsed_ms: entry.elapsed_ms,
            execution_summary: entry.execution_summary.clone(),
            snowflake_dml: entry.snowflake_dml.clone(),
            snowflake_ddl: entry.snowflake_ddl.clone(),
        }
    }
    fn apply(&self, entry: &mut HistoryEntry) {
        entry.provenance = self.provenance;
        entry.status = self.status;
        entry.elapsed_ms = self.elapsed_ms;
        entry.execution_summary = self.execution_summary.clone();
        entry.snowflake_dml = self.snowflake_dml.clone();
        entry.snowflake_ddl = self.snowflake_ddl.clone();
    }
}

/// Opaque permission for one already registered identity. It never contains SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryTicket {
    pub(in crate::persistence) id: String,
    pub(in crate::persistence) lineage: String,
    pub(in crate::persistence) era: String,
    pub(in crate::persistence) generation: String,
}
impl HistoryTicket {
    pub fn id(&self) -> &str {
        &self.id
    }
}

fn invalid() -> AppError {
    AppError::invalid("History admission or privacy evidence is invalid")
}
fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "History identity or privacy lineage conflicts",
    )
}
fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| invalid())
}
fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> AppResult<T> {
    serde_json::from_str(raw).map_err(|_| invalid())
}
fn current(connection: &Connection) -> AppResult<()> {
    if schema_version(connection)? != super::super::SCHEMA_VERSION {
        return Err(conflict());
    }
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
fn disposition(value: &str) -> AppResult<HistoryDisposition> {
    match value {
        "pending" => Ok(HistoryDisposition::Pending),
        "applied" => Ok(HistoryDisposition::Applied),
        "suppressed" => Ok(HistoryDisposition::Suppressed),
        _ => Err(invalid()),
    }
}

pub(in crate::persistence) fn migrate(connection: &Connection) -> AppResult<()> {
    for (_, sql) in SCHEMA {
        connection.execute_batch(sql).map_err(db_error)?;
    }
    let era = new_id();
    connection
        .execute("INSERT INTO history_eras_v5 VALUES(?1,NULL)", [&era])
        .map_err(db_error)?;
    let pristine:bool=connection.query_row("SELECT NOT EXISTS(SELECT 1 FROM history_v5) AND NOT EXISTS(SELECT 1 FROM snowflake_console_runs_v5) AND NOT EXISTS(SELECT 1 FROM snowflake_console_rejections_v5)",[],|r|r.get(0)).map_err(db_error)?;
    connection
        .execute(
            "INSERT INTO history_privacy_v5 VALUES(1,?1,?2,?3)",
            params![new_id(), era, pristine],
        )
        .map_err(db_error)?;
    connection.execute("INSERT INTO console_lifecycle_v5 SELECT run_id,NULL,1,0 FROM snowflake_console_runs_v5",[]).map_err(db_error)?;
    // Give legacy history its own explicit privacy identity. Otherwise a later
    // clear could not suppress an older snapshot's unticketed text.
    let (lineage, era, _) = control(connection)?;
    let mut cursor=connection.prepare("SELECT id,started_at,CASE WHEN length(CAST(json AS BLOB))<=6356992 THEN json ELSE '' END FROM history_v5 ORDER BY id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let at: i64 = row.get(1).map_err(db_error)?;
        let raw: String = row.get(2).map_err(db_error)?;
        let value: HistoryEntry = decode(&raw)?;
        value.validate_lifecycle()?;
        if value.id != id
            || i64::try_from(value.started_at).ok() != Some(at)
            || value
                .sql
                .as_ref()
                .is_some_and(|s| s.len() > MAX_HISTORY_SQL_BYTES)
        {
            return Err(invalid());
        }
        let mut seed = value.clone();
        seed.sql = None;
        seed.provenance = HistoryProvenance::Admitted;
        seed.status = None;
        seed.elapsed_ms = None;
        seed.execution_summary = None;
        seed.snowflake_dml = None;
        seed.snowflake_ddl = None;
        let generation = profile_generation(connection, &value.connection_id)?;
        let pending = value.provenance == HistoryProvenance::Admitted;
        connection
            .execute(
                "INSERT INTO history_tickets_v5 VALUES(?1,?2,?3,?4,?5,?6,0,?7,?8)",
                params![
                    id,
                    encode(&seed)?,
                    lineage,
                    era,
                    generation,
                    value
                        .sql
                        .as_ref()
                        .map(|sql| hex::encode(Sha256::digest(sql.as_bytes()))),
                    if pending { "pending" } else { "applied" },
                    if pending {
                        None
                    } else {
                        Some(encode(&HistoryCompletion::from_entry(&value))?)
                    }
                ],
            )
            .map_err(db_error)?;
    }
    for (_, sql) in GUARDS {
        connection.execute_batch(sql).map_err(db_error)?;
    }
    Ok(())
}

pub(in crate::persistence) fn schema_objects(version: i64) -> AppResult<Vec<(String, String)>> {
    let mut objects: Vec<_> = if version >= 5 {
        SCHEMA
            .iter()
            .map(|(name, _)| ("table".into(), (*name).into()))
            .chain(
                GUARDS
                    .iter()
                    .map(|(name, _)| ("trigger".into(), (*name).into())),
            )
            .collect()
    } else {
        vec![]
    };
    if version >= 6 {
        objects.push(("table".into(), super::super::history_owner::TABLE.into()));
    }
    if version >= 7 {
        objects.push(("table".into(), super::super::outcomes::TABLE.into()));
        objects.push(("table".into(), super::super::outcomes::BINDING_TABLE.into()));
    }
    Ok(objects)
}

pub(in crate::persistence) fn validate_records(connection: &Connection) -> AppResult<()> {
    if schema_version(connection)? < 5 {
        return Ok(());
    }
    for (name, sql) in SCHEMA {
        let stored: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if stored.as_deref() != Some(*sql) {
            return Err(invalid());
        }
    }
    for (name, sql) in GUARDS {
        let stored: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if stored.as_deref() != Some(*sql) {
            return Err(invalid());
        }
    }
    let control_count: i64 = connection
        .query_row("SELECT count(*) FROM history_privacy_v5", [], |r| r.get(0))
        .map_err(db_error)?;
    if control_count != 1 {
        return Err(invalid());
    }
    let table = super::super::metadata_table(connection, "history")?;
    let unregistered:bool=connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} h LEFT JOIN history_tickets_v5 t ON t.id=h.id WHERE t.id IS NULL)"),[],|r|r.get(0)).map_err(db_error)?;
    if unregistered {
        return Err(invalid());
    }
    let (lineage, era): (String, String) = connection
        .query_row(
            "SELECT lineage,current_era FROM history_privacy_v5 WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;
    if uuid::Uuid::parse_str(&lineage).is_err() || uuid::Uuid::parse_str(&era).is_err() {
        return Err(invalid());
    }
    let bad:bool=connection.query_row("WITH RECURSIVE reached(era) AS (SELECT era FROM history_eras_v5 WHERE parent IS NULL UNION SELECT e.era FROM history_eras_v5 e JOIN reached r ON e.parent=r.era),generations(generation,connection_id) AS (SELECT generation,connection_id FROM history_generations_v5 WHERE parent IS NULL UNION SELECT g.generation,g.connection_id FROM history_generations_v5 g JOIN generations r ON g.parent=r.generation AND g.connection_id=r.connection_id) SELECT NOT EXISTS(SELECT 1 FROM history_eras_v5 WHERE era=?1) OR EXISTS(SELECT 1 FROM history_eras_v5 WHERE era NOT IN (SELECT era FROM reached)) OR EXISTS(SELECT 1 FROM history_generations_v5 WHERE generation NOT IN (SELECT generation FROM generations)) OR EXISTS(SELECT 1 FROM history_profile_privacy_v5 p LEFT JOIN history_generations_v5 g ON g.generation=p.generation AND g.connection_id=p.connection_id WHERE g.generation IS NULL) OR EXISTS(SELECT 1 FROM history_tickets_v5 t LEFT JOIN history_eras_v5 e ON t.era=e.era LEFT JOIN history_generations_v5 g ON t.profile_generation=g.generation AND json_extract(t.seed_json,'$.connectionId')=g.connection_id WHERE e.era IS NULL OR g.generation IS NULL OR (t.sql_digest IS NOT NULL AND (length(t.sql_digest)!=64 OR t.sql_digest GLOB '*[^0-9a-f]*')))",[&era],|r|r.get(0)).map_err(db_error)?;
    if bad {
        return Err(invalid());
    }
    let mut cursor = connection
        .prepare("SELECT id FROM history_tickets_v5 ORDER BY id")
        .map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        validate_ticket_row(connection, &id)?;
    }
    let bad:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM console_lifecycle_v5 l LEFT JOIN snowflake_console_runs_v5 r USING(run_id) WHERE r.run_id IS NULL OR (l.legacy=0 AND (l.ticket_id IS NULL OR NOT EXISTS(SELECT 1 FROM history_tickets_v5 h WHERE h.id=l.ticket_id)))) OR EXISTS(SELECT 1 FROM snowflake_console_runs_v5 r LEFT JOIN console_lifecycle_v5 l USING(run_id) WHERE l.run_id IS NULL)",[],|r|r.get(0)).map_err(db_error)?;
    if bad {
        return Err(invalid());
    }
    super::super::history_owner::validate_records(connection)?;
    super::super::outcomes::validate_records(connection)
}

type TicketRow = (
    HistoryTicket,
    HistoryEntry,
    String,
    String,
    String,
    bool,
    HistoryDisposition,
    Option<HistoryCompletion>,
);
type StoredTicketRow = (String, String, String, String, bool, String, Option<String>);
pub(in crate::persistence) fn read_ticket(
    connection: &Connection,
    id: &str,
) -> AppResult<Option<TicketRow>> {
    let row:Option<StoredTicketRow>=connection.query_row("SELECT CASE WHEN length(CAST(seed_json AS BLOB))<=65536 THEN seed_json ELSE '' END,lineage,era,profile_generation,redacted,disposition,CASE WHEN length(CAST(completion_json AS BLOB))<=65536 THEN completion_json ELSE CASE WHEN completion_json IS NULL THEN NULL ELSE '' END END FROM history_tickets_v5 WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(db_error)?;
    let Some((raw, lineage, era, generation, redacted, state, completion)) = row else {
        return Ok(None);
    };
    if [&lineage, &era, &generation]
        .iter()
        .any(|v| uuid::Uuid::parse_str(v).is_err())
    {
        return Err(invalid());
    }
    let seed: HistoryEntry = decode(&raw)?;
    seed.validate_lifecycle()?;
    if seed.id != id || seed.sql.is_some() || seed.provenance != HistoryProvenance::Admitted {
        return Err(invalid());
    }
    let ticket = HistoryTicket {
        id: id.into(),
        lineage: lineage.clone(),
        era: era.clone(),
        generation: generation.clone(),
    };
    Ok(Some((
        ticket,
        seed,
        lineage,
        era,
        generation,
        redacted,
        disposition(&state)?,
        completion.map(|s| decode(&s)).transpose()?,
    )))
}

fn profile_generation(connection: &Connection, id: &str) -> AppResult<String> {
    if let Some(value) = connection
        .query_row(
            "SELECT generation FROM history_profile_privacy_v5 WHERE connection_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?
    {
        return Ok(value);
    }
    let generation = new_id();
    connection
        .execute(
            "INSERT INTO history_generations_v5 VALUES(?1,?2,NULL)",
            params![generation, id],
        )
        .map_err(db_error)?;
    connection
        .execute(
            "INSERT INTO history_profile_privacy_v5 VALUES(?1,?2)",
            params![id, generation],
        )
        .map_err(db_error)?;
    Ok(generation)
}

pub(in crate::persistence) fn ensure_profile(connection: &Connection, id: &str) -> AppResult<()> {
    profile_generation(connection, id).map(|_| ())
}

pub(in crate::persistence) fn begin(
    connection: &Connection,
    seed: &HistoryEntry,
) -> AppResult<HistoryTicket> {
    current(connection)?;
    seed.validate_lifecycle()?;
    if seed.provenance != HistoryProvenance::Admitted
        || seed.id.is_empty()
        || seed.id.len() > 256
        || seed.connection_name.len() > 100
        || seed.started_at > 9_007_199_254_740_991
        || seed
            .sql
            .as_ref()
            .is_some_and(|sql| sql.len() > MAX_HISTORY_SQL_BYTES)
    {
        return Err(invalid());
    }
    let mut plain = seed.clone();
    plain.sql = None;
    if encode(&plain)?.len() > 65_536
        || seed.connection_id.len() > 256
        || seed.id.contains('\0')
        || seed.connection_id.contains('\0')
    {
        return Err(invalid());
    }
    if let Some((ticket, old, _, _, _, redacted, _, _)) = read_ticket(connection, &seed.id)? {
        validate_ticket_row(connection, &seed.id)?;
        if encode(&plain)? != encode(&old)? {
            return Err(conflict());
        }
        let fingerprint: Option<String> = connection
            .query_row(
                "SELECT sql_digest FROM history_tickets_v5 WHERE id=?1",
                [&seed.id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if let Some(expected) = fingerprint
            && seed
                .sql
                .as_ref()
                .map(|sql| hex::encode(Sha256::digest(sql.as_bytes())))
                != Some(expected)
        {
            return Err(conflict());
        }
        if !redacted {
            let raw: Option<String> = connection
                .query_row("SELECT json FROM history_v7 WHERE id=?1", [&seed.id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(db_error)?;
            if let Some(raw) = raw {
                let old: HistoryEntry = decode(&raw)?;
                if old.sql.is_some() && old.sql != seed.sql {
                    return Err(conflict());
                }
            }
        }
        return Ok(ticket);
    }
    let (lineage, era): (String, String) = connection
        .query_row(
            "SELECT lineage,current_era FROM history_privacy_v5 WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;
    let generation = profile_generation(connection, &seed.connection_id)?;
    let profile: Option<String> = connection
        .query_row(
            "SELECT json FROM profiles_v7 WHERE id=?1",
            [&seed.connection_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let allowed = profile
        .map(|raw| decode::<ConnectionProfile>(&raw))
        .transpose()?
        .is_some_and(|p| p.store_query_text);
    let mut stored = seed.clone();
    if !allowed {
        stored.sql = None;
    }
    let raw = encode(&stored)?;
    if raw.len() > MAX_HISTORY_JSON_BYTES {
        return Err(invalid());
    }
    connection
        .execute(
            "INSERT INTO history_tickets_v5 VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',NULL)",
            params![
                seed.id,
                encode(&plain)?,
                lineage,
                era,
                generation,
                stored
                    .sql
                    .as_ref()
                    .map(|sql| hex::encode(Sha256::digest(sql.as_bytes()))),
                !allowed
            ],
        )
        .map_err(db_error)?;
    connection
        .execute(
            "INSERT INTO history_v7 VALUES(?1,?2,?3)",
            params![seed.id, seed.started_at as i64, raw],
        )
        .map_err(db_error)?;
    connection
        .execute("UPDATE history_privacy_v5 SET pristine=0 WHERE id=1", [])
        .map_err(db_error)?;
    prune(connection)?;
    Ok(HistoryTicket {
        id: seed.id.clone(),
        lineage,
        era,
        generation,
    })
}

pub(in crate::persistence) fn finish(
    connection: &Connection,
    ticket: &HistoryTicket,
    value: &HistoryCompletion,
) -> AppResult<HistoryDisposition> {
    current(connection)?;
    let (stored, mut seed, _, _, _, redacted, state, old) =
        read_ticket(connection, &ticket.id)?.ok_or_else(conflict)?;
    if stored.lineage != ticket.lineage
        || stored.era != ticket.era
        || stored.generation != ticket.generation
    {
        return Err(conflict());
    }
    let visible = validate_ticket_row(connection, &ticket.id)?;
    value.apply(&mut seed);
    seed.validate_lifecycle()?;
    if value.provenance == HistoryProvenance::Admitted {
        return Err(invalid());
    }
    if let Some(old) = old {
        if encode(&old)? != encode(value)? {
            return Err(conflict());
        }
        if visible.is_none() && state != HistoryDisposition::Suppressed {
            connection
                .execute(
                    "UPDATE history_tickets_v5 SET disposition='suppressed' WHERE id=?1",
                    [&ticket.id],
                )
                .map_err(db_error)?;
            return Ok(HistoryDisposition::Suppressed);
        }
        return Ok(state);
    }
    let applied = if state == HistoryDisposition::Suppressed {
        false
    } else if let Some(mut entry) = visible {
        value.apply(&mut entry);
        if redacted {
            entry.sql = None;
        }
        entry.validate_lifecycle()?;
        connection
            .execute(
                "UPDATE history_v7 SET json=?1 WHERE id=?2",
                params![encode(&entry)?, ticket.id],
            )
            .map_err(db_error)?;
        true
    } else {
        false
    };
    let state = if applied { "applied" } else { "suppressed" };
    connection
        .execute(
            "UPDATE history_tickets_v5 SET completion_json=?1,disposition=?2 WHERE id=?3",
            params![encode(value)?, state, ticket.id],
        )
        .map_err(db_error)?;
    disposition(state)
}

pub(in crate::persistence) fn lookup(
    connection: &Connection,
    id: &str,
) -> AppResult<Option<(HistoryTicket, HistoryDisposition, Option<HistoryCompletion>)>> {
    let row = read_ticket(connection, id)?;
    if row.is_some() {
        validate_ticket_row(connection, id)?;
    }
    Ok(row.map(|(ticket, _, _, _, _, _, state, completion)| (ticket, state, completion)))
}

pub(in crate::persistence) fn prune(connection: &Connection) -> AppResult<()> {
    let cutoff = now_ms().saturating_sub(30 * 24 * 60 * 60 * 1000) as i64;
    connection.execute("UPDATE history_tickets_v5 SET disposition='suppressed' WHERE id IN (SELECT id FROM history_v7 WHERE started_at<?1 OR id IN (SELECT id FROM history_v7 ORDER BY started_at DESC,id DESC LIMIT -1 OFFSET 10000))",[cutoff]).map_err(db_error)?;
    connection.execute("DELETE FROM history_v7 WHERE started_at<?1 OR id IN (SELECT id FROM history_v7 ORDER BY started_at DESC,id DESC LIMIT -1 OFFSET 10000)",[cutoff]).map_err(db_error)?;
    Ok(())
}

pub(in crate::persistence) fn clear(connection: &Connection) -> AppResult<()> {
    let parent: String = connection
        .query_row(
            "SELECT current_era FROM history_privacy_v5 WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let era = new_id();
    connection
        .execute(
            "INSERT INTO history_eras_v5 VALUES(?1,?2)",
            params![era, parent],
        )
        .map_err(db_error)?;
    connection
        .execute(
            "UPDATE history_privacy_v5 SET current_era=?1,pristine=0 WHERE id=1",
            [era],
        )
        .map_err(db_error)?;
    connection
        .execute("UPDATE history_tickets_v5 SET disposition='suppressed'", [])
        .map_err(db_error)?;
    connection
        .execute("DELETE FROM history_v7", [])
        .map_err(db_error)?;
    Ok(())
}

pub(in crate::persistence) fn redact_profile(connection: &Connection, id: &str) -> AppResult<()> {
    let parent = profile_generation(connection, id)?;
    let generation = new_id();
    connection
        .execute(
            "INSERT INTO history_generations_v5 VALUES(?1,?2,?3)",
            params![generation, id, parent],
        )
        .map_err(db_error)?;
    connection
        .execute(
            "UPDATE history_profile_privacy_v5 SET generation=?1 WHERE connection_id=?2",
            params![generation, id],
        )
        .map_err(db_error)?;
    connection.execute("UPDATE history_v7 SET json=json_set(json,'$.sql',NULL) WHERE id IN (SELECT id FROM history_tickets_v5 WHERE json_extract(seed_json,'$.connectionId')=?1 AND disposition='pending')",[id]).map_err(db_error)?;
    connection.execute("UPDATE history_tickets_v5 SET redacted=1 WHERE json_extract(seed_json,'$.connectionId')=?1 AND disposition='pending'",[id]).map_err(db_error)?;
    connection
        .execute("UPDATE history_privacy_v5 SET pristine=0 WHERE id=1", [])
        .map_err(db_error)?;
    Ok(())
}

impl Persistence {
    pub fn begin_history(&self, seed: &HistoryEntry) -> AppResult<HistoryTicket> {
        let _gate = super::super::history_owner::AdmissionGate::claim(&self.root, false)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let value = begin(&tx, seed)?;
        tx.commit().map_err(db_error)?;
        Ok(value)
    }
    #[cfg(test)]
    pub(crate) fn finish_history(
        &self,
        ticket: &HistoryTicket,
        completion: &HistoryCompletion,
    ) -> AppResult<HistoryDisposition> {
        let _gate = super::super::history_owner::AdmissionGate::claim(&self.root, false)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let value = finish(&tx, ticket, completion)?;
        tx.commit().map_err(db_error)?;
        Ok(value)
    }
}

fn ancestor(
    connection: &Connection,
    table: &str,
    key: &str,
    old: &str,
    new: &str,
) -> AppResult<bool> {
    // UNION terminates cycles; SQLite can spill its visited set rather than
    // retaining a privacy-lifetime collection in Rust.
    connection.query_row(&format!("WITH RECURSIVE chain(value) AS (SELECT ?1 UNION SELECT t.parent FROM {table} t JOIN chain c ON t.{key}=c.value WHERE t.parent IS NOT NULL) SELECT EXISTS(SELECT 1 FROM chain WHERE value=?2)"),params![new,old],|r|r.get(0)).map_err(db_error)
}

fn copy_exact(
    connection: &Connection,
    candidate: &Connection,
    table: &str,
    key: &str,
    columns: &str,
    count: usize,
) -> AppResult<()> {
    let mut cursor = connection
        .prepare(&format!("SELECT {columns} FROM {table} ORDER BY {key}"))
        .map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            values.push(row.get::<_, rusqlite::types::Value>(i).map_err(db_error)?);
        }
        let placeholders = (1..=count)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        candidate.execute(&format!("INSERT INTO {table}({columns}) VALUES({placeholders}) ON CONFLICT({key}) DO NOTHING"),rusqlite::params_from_iter(values.iter())).map_err(db_error)?;
        let stored = candidate
            .query_row(
                &format!("SELECT {columns} FROM {table} WHERE {key}=?1"),
                [&values[0]],
                |r| {
                    (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                },
            )
            .map_err(db_error)?;
        if stored != values {
            return Err(conflict());
        }
    }
    Ok(())
}

fn control(connection: &Connection) -> AppResult<(String, String, bool)> {
    connection
        .query_row(
            "SELECT lineage,current_era,pristine FROM history_privacy_v5 WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(db_error)
}

fn has_sql(connection: &Connection) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM history_v7 WHERE json_type(json,'$.sql')='text')",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)
}

fn history_row(connection: &Connection, id: &str) -> AppResult<Option<HistoryEntry>> {
    let table = super::super::metadata_table(connection, "history")?;
    let raw:Option<(i64,String)>=connection.query_row(&format!("SELECT started_at,CASE WHEN length(CAST(json AS BLOB))<=6356992 THEN json ELSE '' END FROM {table} WHERE id=?1"),[id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
    raw.map(|(at, raw)| {
        let value: HistoryEntry = decode(&raw)?;
        value.validate_lifecycle()?;
        if value.id != id || i64::try_from(value.started_at).ok() != Some(at) {
            return Err(invalid());
        }
        Ok(value)
    })
    .transpose()
}

/// Validate the currently visible row against its SQL-free immutable ticket
/// before either reading back an atomic obligation or changing its completion.
fn validate_ticket_row(connection: &Connection, id: &str) -> AppResult<Option<HistoryEntry>> {
    let (_, seed, lineage, _, _, redacted, state, completion) =
        read_ticket(connection, id)?.ok_or_else(invalid)?;
    if let Some(value) = &completion {
        let mut entry = seed.clone();
        value.apply(&mut entry);
        entry.validate_lifecycle()?;
        if value.provenance == HistoryProvenance::Admitted || state == HistoryDisposition::Pending {
            return Err(invalid());
        }
    } else if state == HistoryDisposition::Applied {
        return Err(invalid());
    }
    let Some(row) = history_row(connection, id)? else {
        return Ok(None);
    };
    if lineage != control(connection)?.0
        || row.connection_id != seed.connection_id
        || row.started_at != seed.started_at
        || row.connection_name != seed.connection_name
        || row.statement_count != seed.statement_count
        || state == HistoryDisposition::Suppressed
    {
        return Err(invalid());
    }
    let fingerprint: Option<String> = connection
        .query_row(
            "SELECT sql_digest FROM history_tickets_v5 WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if redacted {
        if row.sql.is_some() {
            return Err(invalid());
        }
    } else if row
        .sql
        .as_ref()
        .map(|sql| hex::encode(Sha256::digest(sql.as_bytes())))
        != fingerprint
    {
        return Err(invalid());
    }
    if let Some(value) = completion {
        if !HistoryCompletion::from_entry(&row).matches(&value) {
            return Err(invalid());
        }
    } else if !matches!(
        row.provenance,
        HistoryProvenance::Admitted | HistoryProvenance::RestoreRecovered
    ) || row.status.is_some()
        || row.elapsed_ms.is_some()
        || row.execution_summary.is_some()
        || row.snowflake_dml.is_some()
        || row.snowflake_ddl.is_some()
    {
        return Err(invalid());
    }
    Ok(Some(row))
}

fn profile_has_sql(connection: &Connection, id: &str) -> AppResult<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_v7 WHERE json_extract(json,'$.connectionId')=?1 AND json_type(json,'$.sql')='text')",
        [id], |r| r.get(0)
    ).map_err(db_error)
}

/// The caller owns the candidate transaction and a consistent live snapshot.
/// Original SQL is copied only as the permitted history row, never a side log.
pub(in crate::persistence) fn merge_for_restore(
    current: &Connection,
    candidate: &Connection,
) -> AppResult<()> {
    let (live_lineage, live_era, live_pristine) = control(current)?;
    let (backup_lineage, backup_era, backup_pristine) = control(candidate)?;
    let same = live_lineage == backup_lineage;
    if !same && !live_pristine && !backup_pristine && (has_sql(current)? || has_sql(candidate)?) {
        return Err(conflict());
    }
    copy_exact(
        current,
        candidate,
        "history_eras_v5",
        "era",
        "era,parent",
        2,
    )?;
    copy_exact(
        current,
        candidate,
        "history_generations_v5",
        "generation",
        "generation,connection_id,parent",
        3,
    )?;
    let (lineage, era) = if live_pristine {
        (backup_lineage.clone(), backup_era.clone())
    } else if backup_pristine
        || !same
        || ancestor(candidate, "history_eras_v5", "era", &backup_era, &live_era)?
    {
        (live_lineage.clone(), live_era.clone())
    } else if ancestor(candidate, "history_eras_v5", "era", &live_era, &backup_era)? {
        (live_lineage.clone(), backup_era.clone())
    } else {
        return Err(conflict());
    };
    candidate
        .execute(
            "UPDATE history_privacy_v5 SET lineage=?1,current_era=?2,pristine=?3 WHERE id=1",
            params![lineage, era, live_pristine && backup_pristine],
        )
        .map_err(db_error)?;
    // Select the monotonic descendant text generation. An incomparable branch
    // has no proof which opt-out occurred later and cannot restore its text.
    let mut cursor=current.prepare("SELECT connection_id,generation FROM history_profile_privacy_v5 ORDER BY connection_id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let live: String = row.get(1).map_err(db_error)?;
        let old: Option<String> = candidate
            .query_row(
                "SELECT generation FROM history_profile_privacy_v5 WHERE connection_id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let selected = if let Some(old) = old {
            if old == live
                || ancestor(
                    candidate,
                    "history_generations_v5",
                    "generation",
                    &old,
                    &live,
                )?
            {
                live
            } else if ancestor(
                candidate,
                "history_generations_v5",
                "generation",
                &live,
                &old,
            )? {
                old
            } else if !profile_has_sql(current, &id)? && !profile_has_sql(candidate, &id)? {
                // Incomparable privacy generations cannot authorize text. With
                // no text on either branch, retain metadata under the live floor.
                live
            } else {
                return Err(conflict());
            }
        } else {
            live
        };
        candidate.execute("INSERT INTO history_profile_privacy_v5 VALUES(?1,?2) ON CONFLICT(connection_id) DO UPDATE SET generation=excluded.generation",params![id,selected]).map_err(db_error)?;
    }
    drop(rows);
    drop(cursor);
    // Merge ticket originals before deciding which history rows remain visible.
    let mut cursor=current.prepare("SELECT id,seed_json,lineage,era,profile_generation,sql_digest,redacted,disposition,completion_json FROM history_tickets_v5 ORDER BY id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let seed: String = row.get(1).map_err(db_error)?;
        let tl: String = row.get(2).map_err(db_error)?;
        let te: String = row.get(3).map_err(db_error)?;
        let generation: String = row.get(4).map_err(db_error)?;
        let digest: Option<String> = row.get(5).map_err(db_error)?;
        let redacted: bool = row.get(6).map_err(db_error)?;
        let state: String = row.get(7).map_err(db_error)?;
        let completion: Option<String> = row.get(8).map_err(db_error)?;
        let before = read_ticket(candidate, &id)?;
        if let Some((_, other, ol, oe, og, or, os, oc)) = before {
            let od: Option<String> = candidate
                .query_row(
                    "SELECT sql_digest FROM history_tickets_v5 WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if seed != encode(&other)? || tl != ol || te != oe || generation != og || digest != od {
                return Err(conflict());
            }
            let old = oc.as_ref().map(encode).transpose()?;
            if old.is_some() && completion.is_some() && old != completion {
                return Err(conflict());
            }
            let suppressed = state == "suppressed"
                || os == HistoryDisposition::Suppressed
                || history_row(current, &id)?.is_none()
                || history_row(candidate, &id)?.is_none();
            candidate.execute("UPDATE history_tickets_v5 SET redacted=?1,disposition=?2,completion_json=?3 WHERE id=?4",params![redacted||or,if suppressed{"suppressed"}else if old.is_some()||completion.is_some(){"applied"}else{"pending"},completion.or(old),id]).map_err(db_error)?;
        } else {
            candidate
                .execute(
                    "INSERT INTO history_tickets_v5 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        id, seed, tl, te, generation, digest, redacted, state, completion
                    ],
                )
                .map_err(db_error)?;
        }
        if let Some(live) = history_row(current, &id)? {
            if let Some(old) = history_row(candidate, &id)?
                && live.sql.is_some()
                && old.sql.is_some()
                && live.sql != old.sql
            {
                return Err(conflict());
            }
            candidate.execute("INSERT INTO history_v7 VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json",params![id,live.started_at as i64,encode(&live)?]).map_err(db_error)?;
        }
    }
    drop(rows);
    drop(cursor);
    // Legacy completed history is preserved without inventing tickets or proof.
    let mut cursor=current.prepare("SELECT id FROM history_v7 WHERE id NOT IN (SELECT id FROM history_tickets_v5) ORDER BY id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let value = history_row(current, &id)?.ok_or_else(invalid)?;
        if let Some(old) = history_row(candidate, &id)? {
            if encode(&old)? != encode(&value)? {
                return Err(conflict());
            }
        } else {
            candidate
                .execute(
                    "INSERT INTO history_v7 VALUES(?1,?2,?3)",
                    params![id, value.started_at as i64, encode(&value)?],
                )
                .map_err(db_error)?;
        }
    }
    drop(rows);
    drop(cursor);
    // Apply the merged privacy floor, including backup-only IDs never observed
    // locally before clear. Re-enabling a profile never revives older text.
    let mut cursor = candidate
        .prepare("SELECT id FROM history_tickets_v5 ORDER BY id")
        .map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let (_, seed, tl, te, generation, redacted, state, completion) =
            read_ticket(candidate, &id)?.ok_or_else(invalid)?;
        let era_ok = tl == lineage && te == era;
        let selected: Option<String> = candidate
            .query_row(
                "SELECT generation FROM history_profile_privacy_v5 WHERE connection_id=?1",
                [&seed.connection_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let redact = redacted || selected.as_ref().is_some_and(|g| g != &generation);
        let visible = history_row(candidate, &id)?;
        if !era_ok || state == HistoryDisposition::Suppressed || visible.is_none() {
            candidate.execute("UPDATE history_tickets_v5 SET disposition='suppressed',redacted=?1 WHERE id=?2",params![redact,id]).map_err(db_error)?;
            candidate
                .execute("DELETE FROM history_v7 WHERE id=?1", [id])
                .map_err(db_error)?;
            continue;
        }
        let mut value = visible.ok_or_else(invalid)?;
        if redact {
            value.sql = None;
        }
        if let Some(completion) = completion {
            completion.apply(&mut value);
        } else if read_ticket(current, &id)?.is_none() {
            value.provenance = HistoryProvenance::RestoreRecovered;
            value.status = None;
            value.elapsed_ms = None;
        }
        value.validate_lifecycle()?;
        candidate
            .execute(
                "UPDATE history_v7 SET json=?1 WHERE id=?2",
                params![encode(&value)?, id],
            )
            .map_err(db_error)?;
        candidate
            .execute(
                "UPDATE history_tickets_v5 SET redacted=?1 WHERE id=?2",
                params![redact, id],
            )
            .map_err(db_error)?;
    }
    // Console originals determine ownership; a restore never creates a publisher.
    let mut cursor=current.prepare("SELECT run_id,ticket_id,legacy,publisher_sealed FROM console_lifecycle_v5 ORDER BY run_id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let ticket: Option<String> = row.get(1).map_err(db_error)?;
        let legacy: bool = row.get(2).map_err(db_error)?;
        let sealed: bool = row.get(3).map_err(db_error)?;
        let old:Option<(Option<String>,bool,bool)>=candidate.query_row("SELECT ticket_id,legacy,publisher_sealed FROM console_lifecycle_v5 WHERE run_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
        if let Some((ot, ol, os)) = old {
            if ot != ticket || ol != legacy {
                return Err(conflict());
            }
            candidate
                .execute(
                    "UPDATE console_lifecycle_v5 SET publisher_sealed=?1 WHERE run_id=?2",
                    params![sealed || os, id],
                )
                .map_err(db_error)?;
        } else {
            candidate
                .execute(
                    "INSERT INTO console_lifecycle_v5 VALUES(?1,?2,?3,?4)",
                    params![id, ticket, legacy, sealed],
                )
                .map_err(db_error)?;
        }
    }
    copy_exact(
        current,
        candidate,
        "audit_v7",
        "id",
        "id,occurred_at,connection_id,action,digest,outcome",
        6,
    )?;
    super::super::history_owner::merge_for_restore(current, candidate)?;
    super::super::outcomes::merge_for_restore(current, candidate)
}
