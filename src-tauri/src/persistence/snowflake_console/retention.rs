//! Browsing-only indexes. Originals remain in the journal; archive is not erasure.
use super::*;
use crate::console_contract::*;
use std::collections::HashSet;

const RUNS: &str = "snowflake_console_runs_v4";
const REJECTIONS: &str = "snowflake_console_rejections_v4";
const CONTROL: &str = "snowflake_console_retention_v4";
const CONTROL_SQL: &str = "CREATE TABLE snowflake_console_retention_v4(id INTEGER PRIMARY KEY CHECK(id=1),inventory_epoch TEXT NOT NULL,next_sequence INTEGER NOT NULL CHECK(next_sequence>0),inventory_revision INTEGER NOT NULL CHECK(inventory_revision>=0),positive_count INTEGER NOT NULL CHECK(positive_count>=0),refusal_count INTEGER NOT NULL CHECK(refusal_count>=0),charged_bytes INTEGER NOT NULL CHECK(charged_bytes>=0))";

fn objects() -> Vec<(&'static str, String, String)> {
    let mut result = Vec::new();
    for (name, sql) in DECISION_TRIGGERS {
        result.push(("trigger", name.into(), sql.replace("_v3", "_v4")));
    }
    for (table, tag, key, count, charge) in [
        (
            RUNS,
            "run",
            "run_id",
            "positive_count",
            "512+length(CAST(NEW.run_id||NEW.client_request_id||NEW.session_id||NEW.connection_id||NEW.plan_digest||NEW.state||NEW.observations_json||coalesce(NEW.terminal_json,'') AS BLOB))",
        ),
        (
            REJECTIONS,
            "refusal",
            "client_request_id",
            "refusal_count",
            "256+length(CAST(NEW.client_request_id||NEW.session_id||NEW.plan_digest||NEW.code AS BLOB))",
        ),
    ] {
        for (suffix, columns, unique) in [
            ("sequence", "receipt_sequence", "UNIQUE "),
            ("browse", "(archived_at IS NULL),receipt_sequence", ""),
        ] {
            let name = format!("console_{tag}_{suffix}_v4");
            result.push((
                "index",
                name.clone(),
                format!("CREATE {unique}INDEX {name} ON {table}({columns})"),
            ));
        }
        let name = format!("console_{tag}_allocate_v4");
        result.push(("trigger",name.clone(),format!("CREATE TRIGGER {name} AFTER INSERT ON {table} BEGIN SELECT CASE WHEN NEW.receipt_sequence!=0 OR (SELECT next_sequence FROM {CONTROL} WHERE id=1)>=9223372036854775807 THEN RAISE(ABORT,'invalid receipt sequence') END; UPDATE {table} SET receipt_sequence=(SELECT next_sequence FROM {CONTROL} WHERE id=1) WHERE {key}=NEW.{key}; UPDATE {CONTROL} SET next_sequence=next_sequence+1,{count}={count}+1,charged_bytes=charged_bytes+({charge}) WHERE id=1; END")));
        let name = format!("console_{tag}_sequence_guard_v4");
        result.push(("trigger",name.clone(),format!("CREATE TRIGGER {name} BEFORE UPDATE OF receipt_sequence ON {table} WHEN (OLD.receipt_sequence!=0 AND NEW.receipt_sequence!=OLD.receipt_sequence) OR NEW.receipt_sequence<=0 OR EXISTS(SELECT 1 FROM {} WHERE receipt_sequence=NEW.receipt_sequence) BEGIN SELECT RAISE(ABORT,'conflicting receipt sequence'); END",if table==RUNS {REJECTIONS}else{RUNS})));
        let name = format!("console_{tag}_charge_update_v4");
        result.push(("trigger",name.clone(),format!("CREATE TRIGGER {name} AFTER UPDATE ON {table} BEGIN UPDATE {CONTROL} SET charged_bytes=charged_bytes+({charge})-({old}) WHERE id=1; END",old=charge.replace("NEW.","OLD."))));
        let name = format!("console_{tag}_charge_delete_v4");
        result.push(("trigger",name.clone(),format!("CREATE TRIGGER {name} AFTER DELETE ON {table} BEGIN UPDATE {CONTROL} SET {count}={count}-1,charged_bytes=charged_bytes-({old}) WHERE id=1; END",old=charge.replace("NEW.","OLD."))));
    }
    let charge = "256+length(CAST(coalesce(NEW.outcome_json,'')||NEW.observations_json AS BLOB))";
    for event in ["INSERT", "UPDATE", "DELETE"] {
        let name = format!("console_step_charge_{}_v4", event.to_ascii_lowercase());
        let delta = match event {
            "INSERT" => format!("+({charge})"),
            "DELETE" => format!("-({})", charge.replace("NEW.", "OLD.")),
            _ => format!("+({charge})-({})", charge.replace("NEW.", "OLD.")),
        };
        result.push(("trigger",name.clone(),format!("CREATE TRIGGER {name} AFTER {event} ON snowflake_console_steps_v4 BEGIN UPDATE {CONTROL} SET charged_bytes=charged_bytes{delta} WHERE id=1; END")));
    }
    result
}

pub(crate) fn schema_objects(version: i64) -> AppResult<Vec<(String, String)>> {
    let tables: &[&str] = match version {
        2 => &["snowflake_console_runs", "snowflake_console_steps"],
        3 => &[
            "snowflake_console_runs_v3",
            "snowflake_console_steps_v3",
            "snowflake_console_rejections_v3",
        ],
        4 => &[RUNS, "snowflake_console_steps_v4", REJECTIONS, CONTROL],
        _ => return Err(invalid()),
    };
    let mut names: Vec<_> = tables
        .iter()
        .map(|name| ("table".into(), (*name).into()))
        .collect();
    if version == 3 {
        names.extend(
            DECISION_TRIGGERS
                .iter()
                .map(|(name, _)| ("trigger".into(), (*name).into())),
        );
    }
    if version == 4 {
        names.extend(
            objects()
                .into_iter()
                .map(|(kind, name, _)| (kind.into(), name)),
        );
    }
    Ok(names)
}

fn totals(connection: &Connection) -> AppResult<(i64, i64, i64)> {
    connection.query_row("SELECT (SELECT count(*) FROM snowflake_console_runs_v4),(SELECT count(*) FROM snowflake_console_rejections_v4),(SELECT coalesce(sum(512+length(CAST(run_id||client_request_id||session_id||connection_id||plan_digest||state||observations_json||coalesce(terminal_json,'') AS BLOB))),0) FROM snowflake_console_runs_v4)+(SELECT coalesce(sum(256+length(CAST(client_request_id||session_id||plan_digest||code AS BLOB))),0) FROM snowflake_console_rejections_v4)+(SELECT coalesce(sum(256+length(CAST(coalesce(outcome_json,'')||observations_json AS BLOB))),0) FROM snowflake_console_steps_v4)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)
}

/// Called in the parent's IMMEDIATE migration transaction under the raw lease.
pub(in crate::persistence) fn migrate_v4(connection: &Connection) -> AppResult<()> {
    for (name, _) in DECISION_TRIGGERS {
        connection
            .execute_batch(&format!("DROP TRIGGER {name}"))
            .map_err(db_error)?;
    }
    connection.execute_batch("ALTER TABLE snowflake_console_runs_v3 RENAME TO snowflake_console_runs_v4; ALTER TABLE snowflake_console_steps_v3 RENAME TO snowflake_console_steps_v4; ALTER TABLE snowflake_console_rejections_v3 RENAME TO snowflake_console_rejections_v4;").map_err(db_error)?;
    for table in [RUNS, REJECTIONS] {
        connection.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN receipt_sequence INTEGER NOT NULL DEFAULT 0 CHECK(receipt_sequence>=0); ALTER TABLE {table} ADD COLUMN archived_at INTEGER CHECK(archived_at>=0); ALTER TABLE {table} ADD COLUMN browsing_revision INTEGER NOT NULL DEFAULT 0 CHECK(browsing_revision>=0)")).map_err(db_error)?;
    }
    connection.execute_batch(CONTROL_SQL).map_err(db_error)?;
    connection
        .execute(
            &format!("INSERT INTO {CONTROL} VALUES(1,?1,1,0,0,0,0)"),
            [crate::domain::new_id()],
        )
        .map_err(db_error)?;
    resequence(connection)?;
    let (positive, refusal, charged) = totals(connection)?;
    connection.execute(&format!("UPDATE {CONTROL} SET positive_count=?1,refusal_count=?2,charged_bytes=?3 WHERE id=1"),params![positive,refusal,charged]).map_err(db_error)?;
    for (_, _, sql) in objects() {
        connection.execute_batch(&sql).map_err(db_error)?;
    }
    Ok(())
}

/// A restore creates a new ordering epoch, never transfers branch sequence identity.
pub(super) fn resequence(connection: &Connection) -> AppResult<()> {
    // The ordered cursor selects only stable original time/identity fields;
    // updates touch browsing fields. Keep one bounded row in Rust at a time.
    let mut sequence = 1i64;
    let mut statement=connection.prepare("SELECT client_request_id,positive FROM (SELECT created_at recorded_at,client_request_id,1 positive FROM snowflake_console_runs_v4 UNION ALL SELECT recorded_at,client_request_id,0 positive FROM snowflake_console_rejections_v4) ORDER BY recorded_at,client_request_id").map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let client: String = row.get(0).map_err(db_error)?;
        let positive: bool = row.get(1).map_err(db_error)?;
        if !valid_app_uuid(&client) || sequence > i64::from(MAX_RUNS + MAX_REJECTIONS) {
            return Err(invalid());
        }
        connection
            .execute(
                &format!(
                    "UPDATE {} SET receipt_sequence=?1 WHERE client_request_id=?2",
                    if positive { RUNS } else { REJECTIONS }
                ),
                params![sequence, client],
            )
            .map_err(db_error)?;
        sequence = sequence.checked_add(1).ok_or_else(invalid)?;
    }
    drop(rows);
    drop(statement);
    connection.execute(&format!("UPDATE {CONTROL} SET inventory_epoch=?1,next_sequence=?2,inventory_revision=0 WHERE id=1"),params![crate::domain::new_id(),sequence]).map_err(db_error)?;
    Ok(())
}

pub(super) fn validate_schema(connection: &Connection) -> AppResult<()> {
    let stored: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name=?1 AND type='table'",
            [CONTROL],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if stored != CONTROL_SQL {
        return Err(conflict());
    }
    for (kind, name, expected) in objects() {
        let stored: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1 AND type=?2",
                params![name, kind],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if stored.as_deref() != Some(expected.as_str()) {
            return Err(conflict());
        }
    }
    let known: HashSet<_> = schema_objects(4)?.into_iter().collect();
    let mut statement = connection.prepare("SELECT type,name FROM sqlite_schema WHERE (tbl_name IN ('snowflake_console_runs_v4','snowflake_console_steps_v4','snowflake_console_rejections_v4','snowflake_console_retention_v4') OR name LIKE 'snowflake_console_%') AND name NOT LIKE 'sqlite_autoindex_%'").map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        if !known.contains(&(
            row.get::<_, String>(0).map_err(db_error)?,
            row.get::<_, String>(1).map_err(db_error)?,
        )) {
            return Err(conflict());
        }
    }
    Ok(())
}

fn control(connection: &Connection) -> AppResult<(String, i64, i64, i64, i64, i64)> {
    let row=connection.query_row(&format!("SELECT CASE WHEN length(CAST(inventory_epoch AS BLOB))=36 THEN inventory_epoch ELSE '' END,next_sequence,inventory_revision,positive_count,refusal_count,charged_bytes FROM {CONTROL} WHERE id=1"),[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).map_err(db_error)?;
    if !valid_app_uuid(&row.0) || row.1 <= 0 || row.2 < 0 || row.3 < 0 || row.4 < 0 || row.5 < 0 {
        return Err(invalid());
    }
    Ok(row)
}

pub(super) fn validate_records(connection: &Connection) -> AppResult<()> {
    let (_, next, _, positive, refusal, charged) = control(connection)?;
    let bad:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM (SELECT receipt_sequence,archived_at,browsing_revision FROM snowflake_console_runs_v4 UNION ALL SELECT receipt_sequence,archived_at,browsing_revision FROM snowflake_console_rejections_v4) WHERE typeof(receipt_sequence)!='integer' OR receipt_sequence<=0 OR receipt_sequence>=?1 OR typeof(browsing_revision)!='integer' OR browsing_revision<0 OR (archived_at IS NOT NULL AND (typeof(archived_at)!='integer' OR archived_at<0))) OR EXISTS(SELECT receipt_sequence FROM (SELECT receipt_sequence FROM snowflake_console_runs_v4 UNION ALL SELECT receipt_sequence FROM snowflake_console_rejections_v4) GROUP BY receipt_sequence HAVING count(*)!=1) OR (SELECT count(*) FROM snowflake_console_retention_v4)!=1",[next],|r|r.get(0)).map_err(db_error)?;
    if bad || totals(connection)? != (positive, refusal, charged) {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn copy_browsing(
    current: &Connection,
    pinned: &Connection,
    client: &str,
) -> AppResult<()> {
    for table in [RUNS, REJECTIONS] {
        let prior: Option<(Option<i64>, i64)> = current
            .query_row(
                &format!(
                    "SELECT archived_at,browsing_revision FROM {table} WHERE client_request_id=?1"
                ),
                [client],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((archived, revision)) = prior {
            pinned.execute(&format!("UPDATE {table} SET archived_at=?1,browsing_revision=?2 WHERE client_request_id=?3"),params![archived,revision,client]).map_err(db_error)?;
        }
    }
    Ok(())
}

pub(super) fn restore_epoch(connection: &Connection) -> AppResult<()> {
    // Sequences are local to an epoch. Temporarily remove only sequence guards
    // and indexes while the private restore transaction deterministically orders
    // its complete validated union. No other connection can see this interval.
    for (kind, name, _) in objects()
        .into_iter()
        .filter(|(_, name, _)| name.contains("_sequence"))
    {
        connection
            .execute_batch(&format!("DROP {kind} {name}"))
            .map_err(db_error)?;
    }
    resequence(connection)?;
    for (_, name, sql) in objects()
        .into_iter()
        .filter(|(_, name, _)| name.contains("_sequence"))
    {
        let _ = name;
        connection.execute_batch(&sql).map_err(db_error)?;
    }
    Ok(())
}

fn summary(
    connection: &Connection,
    client: &str,
    epoch: &str,
) -> AppResult<Option<SnowflakeConsoleReceiptSummary>> {
    if !valid_app_uuid(client) {
        return Err(invalid());
    }
    ensure_single_decision(connection, client)?;
    let run_id:Option<String>=connection.query_row("SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id ELSE '' END FROM snowflake_console_runs_v4 WHERE client_request_id=?1",[client],|r|r.get(0)).optional().map_err(db_error)?;
    let (table, mut result) = if let Some(id) = run_id {
        // Inventory loads a bounded header, never full original step JSON.
        // Startup/detail/archive validate the complete original independently.
        let oversized:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM snowflake_console_runs_v4 WHERE run_id=?1 AND (length(CAST(session_id AS BLOB))!=36 OR length(CAST(connection_id AS BLOB))!=36 OR length(CAST(plan_digest AS BLOB))!=64 OR length(CAST(state AS BLOB))>8 OR length(CAST(observations_json AS BLOB))>16384 OR length(CAST(terminal_json AS BLOB))>16384))",[&id],|r|r.get(0)).map_err(db_error)?;
        if oversized {
            return Err(invalid());
        }
        let (metadata,revision,state,observations,terminal)=connection.query_row("SELECT session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json,terminal_json FROM snowflake_console_runs_v4 WHERE run_id=?1",[&id],|r|Ok((ConsoleRunMetadata{run_id:id.clone(),client_request_id:client.into(),session_id:r.get(0)?,connection_id:r.get(1)?,plan_digest:r.get(2)?,step_count:r.get(3)?,created_at:sqlite_nonnegative(r,4)?},sqlite_nonnegative(r,5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<String>>(8)?))).map_err(db_error)?;
        validate_metadata(&metadata)?;
        if revision > MAX_REVISION {
            return Err(invalid());
        }
        let state = match state.as_str() {
            "prepared" => ConsoleRunState::Prepared,
            "running" => ConsoleRunState::Running,
            "terminal" => ConsoleRunState::Terminal,
            _ => return Err(invalid()),
        };
        let observations: ConsoleObservations = decode(&observations)?;
        let terminal: Option<ConsoleTerminal> = terminal.as_deref().map(decode).transpose()?;
        if (state == ConsoleRunState::Terminal) != terminal.is_some() {
            return Err(invalid());
        }
        if let Some(terminal) = &terminal {
            let c = &terminal.accounting;
            if !valid_time(terminal.finished_at)
                || terminal.finished_at < metadata.created_at
                || c.planned != metadata.step_count
                || c.intended > c.planned
                || c.acknowledged
                    .checked_add(c.rejected)
                    .and_then(|n| n.checked_add(c.unknown))
                    .and_then(|n| n.checked_add(c.not_dispatched))
                    != Some(c.intended)
                || c.not_intended != c.planned - c.intended
                || c.not_intended.checked_add(c.not_dispatched) != Some(c.not_attempted)
                || !observations_cover(&observations, &terminal.observations)
            {
                return Err(invalid());
            }
        }
        let (invalid_step,unknown):(bool,bool)=connection.query_row("SELECT EXISTS(SELECT 1 FROM snowflake_console_steps_v4 WHERE run_id=?1 AND (length(CAST(outcome_json AS BLOB))>16384 OR (outcome_json IS NOT NULL AND CASE WHEN json_valid(outcome_json) THEN coalesce(json_extract(outcome_json,'$.kind'),'') NOT IN ('acknowledged','rejected','unknown','notDispatched') ELSE 1 END))),EXISTS(SELECT 1 FROM snowflake_console_steps_v4 WHERE run_id=?1 AND (outcome_json IS NULL OR CASE WHEN json_valid(outcome_json) THEN json_extract(outcome_json,'$.kind')='unknown' ELSE 0 END))",[&id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
        if invalid_step {
            return Err(invalid());
        }
        let eligible = terminal.is_some();
        (
            RUNS,
            SnowflakeConsoleReceiptSummary {
                inventory_epoch: epoch.into(),
                sequence: String::new(),
                client_request_id: client.into(),
                session_id: metadata.session_id,
                connection_id: Some(metadata.connection_id),
                run_id: Some(id),
                plan_digest: metadata.plan_digest,
                recorded_at: metadata.created_at.to_string(),
                original_revision: revision.to_string(),
                browsing_revision: String::new(),
                kind: SnowflakeConsoleReceiptKind::Positive,
                state: Some(state),
                stop_reason: terminal.as_ref().map(|t| t.reason),
                unknown,
                refusal_code: None,
                archived: false,
                archive_eligible: eligible,
                archive_block_reason: (!eligible)
                    .then(|| "The original run is not terminal".into()),
            },
        )
    } else if let Some(refusal) = read_rejection(connection, client)? {
        (
            REJECTIONS,
            SnowflakeConsoleReceiptSummary {
                inventory_epoch: epoch.into(),
                sequence: String::new(),
                client_request_id: client.into(),
                session_id: refusal.session_id,
                connection_id: None,
                run_id: None,
                plan_digest: refusal.plan_digest,
                recorded_at: refusal.recorded_at.to_string(),
                original_revision: "0".into(),
                browsing_revision: String::new(),
                kind: SnowflakeConsoleReceiptKind::Refusal,
                state: None,
                stop_reason: None,
                unknown: false,
                refusal_code: Some(refusal.code),
                archived: false,
                archive_eligible: true,
                archive_block_reason: None,
            },
        )
    } else {
        return Ok(None);
    };
    let (sequence,archived,revision):(i64,Option<i64>,i64)=connection.query_row(&format!("SELECT receipt_sequence,archived_at,browsing_revision FROM {table} WHERE client_request_id=?1"),[client],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    if sequence <= 0 || revision < 0 || archived.is_some_and(|v| v < 0) {
        return Err(invalid());
    }
    result.sequence = sequence.to_string();
    result.browsing_revision = revision.to_string();
    result.archived = archived.is_some();
    Ok(Some(result))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    epoch: String,
    revision: i64,
    high: i64,
    last: i64,
    filter: SnowflakeConsoleReceiptFilter,
}

fn validate_filter(filter: &SnowflakeConsoleReceiptFilter) -> AppResult<()> {
    if filter
        .connection_id
        .as_ref()
        .is_some_and(|id| !valid_app_uuid(id))
    {
        return Err(invalid());
    }
    Ok(())
}

impl Persistence {
    pub(crate) fn console_receipts(
        &self,
        request: &SnowflakeConsoleReceiptPageRequest,
    ) -> AppResult<SnowflakeConsoleReceiptPage> {
        if !(1..=100).contains(&request.limit) {
            return Err(invalid());
        }
        validate_filter(&request.filter)?;
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let (epoch, next, revision, _, _, _) = control(&tx)?;
        let cursor = if let Some(raw) = &request.cursor {
            if raw.len() > 2048 {
                return Err(invalid());
            }
            let cursor: Cursor = serde_json::from_str(raw).map_err(|_| invalid())?;
            if cursor.epoch != epoch
                || cursor.revision != revision
                || cursor.filter != request.filter
            {
                return Err(conflict());
            }
            if cursor.high < 0
                || cursor.high >= next
                || cursor.last <= 0
                || cursor.last > cursor.high.checked_add(1).ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            cursor
        } else {
            Cursor {
                epoch: epoch.clone(),
                revision,
                high: next - 1,
                last: next,
                filter: request.filter.clone(),
            }
        };
        let visibility = match request.filter.visibility {
            SnowflakeConsoleReceiptVisibility::All => 0,
            SnowflakeConsoleReceiptVisibility::Visible => 1,
            SnowflakeConsoleReceiptVisibility::Archived => 2,
        };
        let kind = match request.filter.kind {
            None => 0,
            Some(SnowflakeConsoleReceiptKind::Positive) => 1,
            Some(SnowflakeConsoleReceiptKind::Refusal) => 2,
        };
        let mut statement=tx.prepare("SELECT CASE WHEN length(CAST(client_request_id AS BLOB))=36 THEN client_request_id ELSE '' END FROM (SELECT client_request_id,receipt_sequence,archived_at,connection_id,1 kind FROM snowflake_console_runs_v4 UNION ALL SELECT client_request_id,receipt_sequence,archived_at,NULL connection_id,2 kind FROM snowflake_console_rejections_v4) WHERE receipt_sequence<=?1 AND receipt_sequence<?2 AND (?3=0 OR (?3=1 AND archived_at IS NULL) OR (?3=2 AND archived_at IS NOT NULL)) AND (?4=0 OR kind=?4) AND (?5 IS NULL OR connection_id=?5) ORDER BY receipt_sequence DESC LIMIT ?6").map_err(db_error)?;
        let clients = statement
            .query_map(
                params![
                    cursor.high,
                    cursor.last,
                    visibility,
                    kind,
                    request.filter.connection_id,
                    request.limit + 1
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        drop(statement);
        let more = clients.len() > request.limit as usize;
        let mut receipts = Vec::with_capacity(request.limit as usize);
        for client in clients.iter().take(request.limit as usize) {
            receipts.push(summary(&tx, client, &epoch)?.ok_or_else(invalid)?);
        }
        let next_cursor = if more {
            let last = receipts
                .last()
                .ok_or_else(invalid)?
                .sequence
                .parse()
                .map_err(|_| invalid())?;
            Some(serde_json::to_string(&Cursor { last, ..cursor }).map_err(|_| invalid())?)
        } else {
            None
        };
        tx.commit().map_err(db_error)?;
        Ok(SnowflakeConsoleReceiptPage {
            inventory_epoch: epoch,
            receipts,
            next_cursor,
        })
    }

    pub(crate) fn console_receipt_summary(
        &self,
        client: &str,
    ) -> AppResult<Option<SnowflakeConsoleReceiptSummary>> {
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let (epoch, _, _, _, _, _) = control(&tx)?;
        let result = summary(&tx, client, &epoch)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }

    /// Caller additionally holds configuration/journal gates and excludes active
    /// owners, publishers and every pending outbox for these exact identities.
    pub(crate) fn console_archive_receipts(
        &self,
        owner: &ConsoleJournalOwner,
        request: &SnowflakeConsoleReceiptArchiveRequest,
        at: u64,
    ) -> AppResult<SnowflakeConsoleReceiptArchiveResult> {
        owner.validate(self)?;
        if !valid_time(at) || request.items.is_empty() || request.items.len() > 100 {
            return Err(invalid());
        }
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        let (epoch, _, revision, _, _, _) = control(&tx)?;
        if request.inventory_epoch != epoch || revision >= i64::MAX - 1 {
            return Err(conflict());
        }
        let mut seen = HashSet::new();
        let mut receipts = Vec::with_capacity(request.items.len());
        for item in &request.items {
            if !seen.insert(&item.client_request_id) {
                return Err(conflict());
            }
            let current = summary(&tx, &item.client_request_id, &epoch)?.ok_or_else(conflict)?;
            if let Some(id) = &current.run_id {
                read_run(&tx, id)?.ok_or_else(invalid)?;
            }
            if !current.archive_eligible
                || current.archived == request.archived
                || item.session_id != current.session_id
                || item.plan_digest != current.plan_digest
                || item.kind != current.kind
                || item.run_id != current.run_id
                || item.recorded_at != current.recorded_at
                || item.original_revision != current.original_revision
                || item.browsing_revision != current.browsing_revision
                || item.refusal_code != current.refusal_code
            {
                return Err(conflict());
            }
            let browsing: i64 = current.browsing_revision.parse().map_err(|_| invalid())?;
            if browsing >= i64::MAX - 1
                || at < current.recorded_at.parse::<u64>().map_err(|_| invalid())?
            {
                return Err(conflict());
            }
            let table = if current.kind == SnowflakeConsoleReceiptKind::Positive {
                RUNS
            } else {
                REJECTIONS
            };
            tx.execute(&format!("UPDATE {table} SET archived_at=?1,browsing_revision=browsing_revision+1 WHERE client_request_id=?2"),params![if request.archived{Some(sqlite_integer(at)?)}else{None},item.client_request_id]).map_err(db_error)?;
            receipts.push(summary(&tx, &item.client_request_id, &epoch)?.ok_or_else(invalid)?);
        }
        tx.execute(
            &format!("UPDATE {CONTROL} SET inventory_revision=inventory_revision+1 WHERE id=1"),
            [],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(SnowflakeConsoleReceiptArchiveResult { receipts })
    }

    pub(crate) fn console_storage(&self) -> AppResult<SnowflakeConsoleStorage> {
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let (epoch, _, _, positive, refusal, charged) = control(&tx)?;
        tx.commit().map_err(db_error)?;
        Ok(SnowflakeConsoleStorage {
            inventory_epoch: epoch,
            positive_count: positive.to_string(),
            refusal_count: refusal.to_string(),
            charged_bytes: Some(charged.to_string()),
            reserved_bytes: None,
            admission_budget_bytes: None,
            free_bytes: None,
            retention_enabled: false,
        })
    }
}
