use super::*;

const READ_LIMIT: Duration = Duration::from_secs(5);

// Real owned Node child, production framing/Helper/submit guard, synthetic SDK.
// The SDK never calls HTTP; these tests do not qualify a Snowflake account or
// prove remote transaction behavior. The ledger records every SDK execute.
fn read_launch(scenario: &str) -> (Launch, tempfile::TempDir) {
    let node = PathBuf::from(
        std::env::var_os("JOINERY_TEST_NODE")
            .expect("JOINERY_TEST_NODE must name an explicit absolute Node executable"),
    );
    assert!(node.is_absolute() && node.is_file());
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("mutation-read-fixture.mjs");
    // Keep this test-owned ledger beyond state-root cleanup, so assertions can
    // inspect the final execute list after actual child reaping.
    let ledger = tempfile::tempdir().unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../snowflake-helper/src");
    let body = r#"
import { Readable, PassThrough } from 'node:stream';
import { writeFileSync, renameSync, existsSync } from 'node:fs';
const { Helper } = await import(HELPER_PATH);
const { Decoder, writer } = await import(FRAMING_PATH);
const { installQuerySubmitGuard } = await import(GUARD_PATH);
const { MUTATION_CONTEXT_SQL, MUTATION_SETTINGS_SQL } = await import(BATCH_PATH);
const { MUTATION_READ_CONTEXT_SQL, MUTATION_READ_TRANSACTION_SQL } = await import(READ_PATH);
const { SQL_CONTRACT } = await import(CONTRACT_PATH);
const scenario=SCENARIO, root=process.env.JOINERY_SNOWFLAKE_STATE_DIR, ledger=LEDGER_PATH;
const calls=[], records=new Map();
let serial=0, transaction=null, connections=0, stopping=false;
const sdk={createConnection(){connections++;return {
  connectAsync(cb){queueMicrotask(()=>cb(null));return Promise.resolve();},
  destroy(cb){cb(null);},
  execute(options){
    const n=++serial, sql=options.sqlText;
    calls.push({sql,binds:options.binds,parameters:options.parameters??null});
    writeFileSync(ledger+'/calls-next',JSON.stringify({connections,calls}));renameSync(ledger+'/calls-next',ledger+'/calls');
    let names=[], rows=[], read=false, acknowledged=false;
    if(sql===SQL_CONTRACT.bootstrapContext||sql===MUTATION_CONTEXT_SQL){
      names=['user','role','database','schema','warehouse','session'];rows=[['TEST','ROLE','DB','PUBLIC','WH','123']];
      if(sql===MUTATION_CONTEXT_SQL){names.push('transaction');rows[0].push(transaction);}
    }else if(sql===MUTATION_SETTINGS_SQL){names=['key','value'];rows=[['QUOTED_IDENTIFIERS_IGNORE_CASE','false']];}
    else if(sql==='BEGIN TRANSACTION'){transaction=String(1000+n);records.set(transaction,'running');}
    else if(sql==='COMMIT'||sql==='ROLLBACK'){records.set(transaction,sql==='COMMIT'?'committed':'rolled back');transaction=null;}
    else if(sql.startsWith('DESCRIBE TRANSACTION ')){const tid=sql.slice(21);names=['id','session','state'];rows=[[tid,'123',records.get(tid)]];}
    else if(sql.startsWith('UPDATE synthetic ')){names=['number of rows updated'];rows=[['1']];}
    else if(sql===MUTATION_READ_CONTEXT_SQL){read=true;names=['PRINCIPAL','ROLE_NAME','DATABASE_NAME','SCHEMA_NAME','WAREHOUSE_NAME','SESSION_ID','SEARCH_PATH','SECONDARY_ROLES'];rows=[['TEST','ROLE','DB','PUBLIC','WH','123','["DB.PUBLIC"]','{"roles":"ROLE","value":"ALL"}']];}
    else if(sql===MUTATION_READ_TRANSACTION_SQL){read=true;names=['TRANSACTION_ID'];rows=[[transaction]];}
    else if(sql.includes('.INFORMATION_SCHEMA.COLUMNS AS C ')){read=true;names=['ORDINAL','DEFAULT','EXPRESSION','EXACT'];rows=Array.from({length:257},(_,i)=>[String(i),null,'','9'.repeat(38)]);}
    else if(sql.includes('.INFORMATION_SCHEMA.TABLES ')){read=true;names=['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME'];rows=[['DB','PUBLIC','T']];}
    else if(sql==='DESCRIBE TABLE "DB"."PUBLIC"."T" TYPE = COLUMNS'){read=true;names=['name','type'];rows=[['N','NUMBER(38,0)']];}
    else if(sql==='SELECT * FROM "DB"."PUBLIC"."T" LIMIT 0'){read=true;names=['N'];rows=[];}
    else throw Error('unexpected synthetic SQL');
    const statement={
      getRequestId:()=>`11111111-1111-4111-8111-${String(n).padStart(12,'0')}`,
      getQueryId:()=>acknowledged?`22222222-2222-4222-8222-${String(n).padStart(12,'0')}`:undefined,
      getColumns:()=>names.map(name=>({getName:()=>name,getType:()=>name.startsWith('number of ')||sql.startsWith('DESCRIBE TRANSACTION ')&&['id','session'].includes(name)?'FIXED':'TEXT'})),
      streamRows:()=>read&&scenario==='cancel'?new PassThrough({objectMode:true}):Readable.from(rows),
      cancel(cb){cb(null);}
    };
    const complete=()=>{acknowledged=true;options.complete(null,statement);};
    if(scenario==='finish'&&sql==='COMMIT'){
      const timer=setInterval(()=>{if(existsSync(root+'/release-finish')){clearInterval(timer);complete();}},5);
    }else queueMicrotask(complete);
    return statement;
  }
};}};
const send=writer(process.stdout.write.bind(process.stdout),()=>process.exit(2));
const stop=()=>{if(!stopping){stopping=true;send.flush().then(()=>process.exit(0));}};
const guard=installQuerySubmitGuard({onViolation:stop});
const emit=event=>{
  // Deliberately corrupt only owner-read frames from the production helper.
  // Batch admission and its retained original evidence stay genuine locally.
  if(event.type==='rows'&&event.rows?.[0]?.length===8){
    if(scenario==='shape')return send({...event,rows:[['wrong width']]});
    if(scenario==='row-bound')return send({...event,rows:[event.rows[0],event.rows[0]]});
  }
  if(scenario==='terminal-count'&&event.type==='terminal'&&event.ok&&event.result?.rowsRetained===1&&event.result?.rowsSeen===1&&readIds.has(event.id))
    return send({...event,result:{...event.result,rowsRetained:2}});
  return send(event);
};
const readIds=new Set();
const helper=new Helper(sdk,emit,stop,{submitGuard:guard,isFileTransfer:()=>false,closeTimeoutMs:20});
const decoder=new Decoder();
process.stdin.on('data',chunk=>{try{for(const frame of decoder.push(chunk)){if(frame.op==='mutationRead')readIds.add(frame.id);helper.receive(frame);}}catch{stop();}});
process.stdin.on('end',stop);
"#;
    let body = [
        ("HELPER_PATH", "helper.mjs"),
        ("FRAMING_PATH", "framing.mjs"),
        ("GUARD_PATH", "query-submit-guard.mjs"),
        ("BATCH_PATH", "mutation-batch.mjs"),
        ("READ_PATH", "mutation-read.mjs"),
        ("CONTRACT_PATH", "sql-contract.mjs"),
    ]
    .into_iter()
    .fold(body.to_owned(), |body, (token, file)| {
        body.replace(token, &serde_json::to_string(&source.join(file)).unwrap())
    })
    .replace("SCENARIO", &serde_json::to_string(scenario).unwrap())
    .replace(
        "LEDGER_PATH",
        &serde_json::to_string(ledger.path()).unwrap(),
    );
    std::fs::write(&script, body).unwrap();
    (
        Launch::new(
            node,
            Some(script),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            root,
        )
        .unwrap(),
        ledger,
    )
}

async fn pending_helper(
    scenario: &str,
) -> (Supervisor, PathBuf, Value, BatchEvidence, tempfile::TempDir) {
    let (launch, ledger) = read_launch(scenario);
    let root = launch.state_directory().to_owned();
    let supervisor = Supervisor::spawn(
        launch,
        "00000000-0000-4000-8000-000000000001",
        Bounds::default(),
    )
    .await
    .unwrap();
    let mut connect = supervisor.request("connect", "connect", json!({"account":"fixture","host":"fixture.snowflakecomputing.com","username":"TEST","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"}), Impact::Control).unwrap();
    let report = connect.wait_bounded(READ_LIMIT).await;
    assert!(report.ok, "{report:?}");
    let mut context = supervisor
        .request("context", "context", json!({}), Impact::Read)
        .unwrap();
    let report = context.wait_bounded(READ_LIMIT).await;
    assert!(report.ok, "{report:?}");
    let mut payload = batch_payload();
    payload["mode"] = json!("manual");
    let mut batch = supervisor
        .request("pending", "mutationBatch", payload, Impact::Mutating)
        .unwrap();
    let report = batch.wait_bounded(READ_LIMIT).await;
    assert!(report.ok, "{report:?}");
    let evidence = report.batch.unwrap();
    let summary = evidence.summary.as_ref().unwrap();
    assert_eq!(
        summary.outcome,
        crate::drivers::snowflake_batch::Outcome::Pending
    );
    let owner = serde_json::to_value(summary.owner.as_ref().unwrap()).unwrap();
    (supervisor, root, owner, evidence, ledger)
}

fn owner_read(owner: &Value, kind: &str, credits: u32) -> Value {
    let target = if matches!(
        kind,
        "context" | "quotedIdentifierPolicy" | "currentTransaction"
    ) {
        Value::Null
    } else {
        json!({"database":"DB","schema":"PUBLIC","name":"T","tableType":"BASE TABLE"})
    };
    json!({"owner":owner,"expected":batch_payload()["expected"],"kind":kind,"target":target,"credits":credits})
}
fn ledger(root: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("calls")).unwrap()).unwrap()
}
fn executed(root: &std::path::Path) -> Vec<String> {
    ledger(root)["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["sql"].as_str().unwrap().to_owned())
        .collect()
}
async fn drain_read(operation: &mut Operation) -> (Report, Rows) {
    timeout(READ_LIMIT, async {
        let mut rows = vec![];
        while let Some(batch) = operation.rows.recv().await {
            rows.extend(batch.rows.iter().cloned());
        }
        (operation.wait().await, rows)
    })
    .await
    .unwrap()
}
async fn close_helper(supervisor: &Supervisor, root: &std::path::Path) {
    assert!(supervisor.close().await.unwrap().ok);
    assert!(supervisor.wait_exited().await);
    assert!(!root.exists());
}

struct ReadPublicationHook;
impl Drop for ReadPublicationHook {
    fn drop(&mut self) {
        let _ = TERMINAL_PUBLICATION_OBSERVER.with(|slot| slot.borrow_mut().take());
    }
}

fn observe_read_terminal_seal(
    terminal: watch::Receiver<Option<Report>>,
    observed: Arc<std::sync::atomic::AtomicBool>,
) {
    TERMINAL_PUBLICATION_OBSERVER.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move |inner| {
            let published = terminal.borrow().is_some();
            if published {
                // Runs synchronously after terminal publication and before the
                // legacy deferred retirement path or a later EOF can seal it.
                assert!(
                    inner.state.lock().unwrap().retired.is_some(),
                    "active owner-read cancellation published before sealing admission"
                );
                observed.store(true, Ordering::SeqCst);
            } else {
                // The cancel control can publish first. Follow only this exact
                // read watch rather than mistaking that ACK for its terminal.
                observe_read_terminal_seal(terminal, observed);
            }
        }));
    });
}

#[tokio::test]
#[ignore = "requires explicit JOINERY_TEST_NODE; local production-helper/native owner-read bridge only"]
async fn actual_owner_metadata_streams_with_credits_and_retains_manual_original_until_finish() {
    let (supervisor, root, owner, original, calls_root) = pending_helper("reads").await;
    let original_bytes = BatchCapture::encode(&original).unwrap();
    let mut read = supervisor
        .request(
            "columns",
            "mutationRead",
            owner_read(&owner, "columns", 1),
            Impact::Read,
        )
        .unwrap();
    let first = timeout(READ_LIMIT, read.rows.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.rows.len(), 128);
    let mut rows: Rows = first.rows.clone();
    drop(first);
    assert!(
        read.report().is_none(),
        "one initial credit cannot publish 257 rows"
    );
    let mut credit = supervisor.credit(&read.token(), 2).unwrap();
    assert!(credit.wait_bounded(READ_LIMIT).await.ok);
    let (report, rest) = drain_read(&mut read).await;
    rows.extend(rest);
    assert!(report.ok, "{report:?}");
    assert_eq!(rows.len(), 257);
    assert_eq!(
        rows[256],
        vec![
            Some("256".into()),
            None,
            Some("".into()),
            Some("9".repeat(38))
        ]
    );
    assert_eq!(report.result.as_ref().unwrap()["rowsRetained"], 257);
    assert_eq!(report.result.as_ref().unwrap()["truncated"], false);
    assert_eq!(report.statements.len(), 1);
    assert!(report.batch.is_none());
    for kind in [
        "context",
        "quotedIdentifierPolicy",
        "currentTransaction",
        "exactRelation",
        "describe",
        "zeroRowResolution",
    ] {
        let mut operation = supervisor
            .request(
                kind,
                "mutationRead",
                owner_read(&owner, kind, 1),
                Impact::Read,
            )
            .unwrap();
        let (report, rows) = drain_read(&mut operation).await;
        assert!(report.ok, "{kind}: {report:?}");
        assert_eq!(report.statements.len(), 1);
        if kind == "currentTransaction" {
            assert_eq!(
                rows[0][0].as_deref(),
                original.summary.as_ref().unwrap().transaction_id.as_deref()
            );
        }
        if kind == "context" {
            assert_eq!(rows[0].len(), 8);
        }
        if kind == "zeroRowResolution" {
            assert!(rows.is_empty());
        }
    }
    let before = executed(calls_root.path());
    let mut ordinary = supervisor
        .request("ordinary", "context", json!({}), Impact::Read)
        .unwrap();
    let rejected = ordinary.wait_bounded(READ_LIMIT).await;
    assert!(!rejected.ok);
    assert_eq!(executed(calls_root.path()), before);
    let calls = ledger(calls_root.path());
    assert_eq!(calls["connections"], 1);
    assert!(
        calls["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["sql"]
                .as_str()
                .unwrap()
                .contains("INFORMATION_SCHEMA.COLUMNS"))
            .all(|entry| entry["binds"] == json!([])
                && entry["parameters"]["MULTI_STATEMENT_COUNT"] == 1)
    );
    let mut finish = supervisor
        .request(
            "finish",
            "mutationFinish",
            json!({"owner":owner,"action":"rollback","expected":batch_payload()["expected"]}),
            Impact::Mutating,
        )
        .unwrap();
    let report = finish.wait_bounded(READ_LIMIT).await;
    assert!(report.ok, "{report:?}");
    let summary = report.batch.unwrap().summary.unwrap();
    assert_eq!(
        summary.outcome,
        crate::drivers::snowflake_batch::Outcome::RolledBack
    );
    assert_eq!(summary.prior_batches, 1);
    assert_eq!(BatchCapture::encode(&original).unwrap(), original_bytes);
    close_helper(&supervisor, &root).await;
}

#[tokio::test]
#[ignore = "requires explicit JOINERY_TEST_NODE; local production-helper/native owner-read bridge only"]
async fn actual_owner_read_queued_behind_finish_rejects_without_sdk_dispatch() {
    let (supervisor, root, owner, _, calls_root) = pending_helper("finish").await;
    let mut finish = supervisor
        .request(
            "finish",
            "mutationFinish",
            json!({"owner":owner,"action":"commit","expected":batch_payload()["expected"]}),
            Impact::Mutating,
        )
        .unwrap();
    timeout(READ_LIMIT, async {
        while !executed(calls_root.path())
            .iter()
            .any(|sql| sql == "COMMIT")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut read = supervisor
        .request(
            "queued",
            "mutationRead",
            owner_read(&owner, "columns", 1),
            Impact::Read,
        )
        .unwrap();
    timeout(READ_LIMIT, async {
        while read.phase() == Phase::Queued {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        read.report().is_none(),
        "read must remain queued behind held finish"
    );
    assert_eq!(read.phase(), Phase::Sent);
    std::fs::write(
        root.join("release-finish"),
        b"release only this owned fixture",
    )
    .unwrap();
    let report = finish.wait_bounded(READ_LIMIT).await;
    assert!(report.ok, "{report:?}");
    assert_eq!(
        report.batch.unwrap().summary.unwrap().outcome,
        crate::drivers::snowflake_batch::Outcome::Committed
    );
    let report = read.wait_bounded(READ_LIMIT).await;
    assert!(!report.ok, "{report:?}");
    assert!(report.statements.is_empty());
    assert!(
        !executed(calls_root.path())
            .iter()
            .any(|sql| sql.contains("INFORMATION_SCHEMA.COLUMNS"))
    );
    assert_eq!(ledger(calls_root.path())["connections"], 1);
    close_helper(&supervisor, &root).await;
}

#[tokio::test]
#[ignore = "requires explicit JOINERY_TEST_NODE; local production-helper/native owner-read bridge only"]
async fn actual_active_owner_read_cancel_reaps_without_boundary_or_replay() {
    let (supervisor, root, owner, original, calls_root) = pending_helper("cancel").await;
    let observer = supervisor.retirement_observer();
    let helper = supervisor.helper_generation();
    let mut read = supervisor
        .request(
            "held",
            "mutationRead",
            owner_read(&owner, "context", 1),
            Impact::Read,
        )
        .unwrap();
    timeout(READ_LIMIT, async {
        while read.columns.borrow().is_none() {
            read.columns.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let calls = executed(calls_root.path());
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _hook = ReadPublicationHook;
    observe_read_terminal_seal(read.terminal_receiver(), observed.clone());
    let mut cancel = supervisor.cancel(&read.token()).unwrap();
    assert!(cancel.wait_bounded(READ_LIMIT).await.ok);
    let report = read.wait_bounded(READ_LIMIT).await;
    assert!(
        observed.load(Ordering::SeqCst),
        "must inspect admission at the exact read terminal publication"
    );
    assert!(!report.ok, "{report:?}");
    assert_eq!(report.statements.len(), 1);
    assert!(report.batch.is_none());
    assert!(calls.iter().all(|sql| sql != "COMMIT" && sql != "ROLLBACK"));
    assert!(
        supervisor
            .request(
                "replay",
                "mutationRead",
                owner_read(&owner, "context", 1),
                Impact::Read
            )
            .is_err()
    );
    assert!(supervisor.wait_exited().await);
    let proof = observer
        .wait(READ_LIMIT)
        .await
        .expect("actual owned child reaped");
    assert!(helper.matches_reaped(&proof));
    assert_eq!(
        executed(calls_root.path()),
        calls,
        "cancel must not dispatch cleanup or suffix SQL"
    );
    assert_eq!(ledger(calls_root.path())["connections"], 1);
    assert_eq!(
        original.summary.unwrap().outcome,
        crate::drivers::snowflake_batch::Outcome::Pending
    );
    assert!(!root.exists());
}

#[tokio::test]
#[ignore = "requires explicit JOINERY_TEST_NODE; local production-helper/native owner-read bridge only"]
async fn actual_owner_read_malformed_rows_bounds_and_terminal_counts_retire_protocol() {
    for scenario in ["shape", "row-bound", "terminal-count"] {
        let (supervisor, root, owner, original, calls_root) = pending_helper(scenario).await;
        let mut read = supervisor
            .request(
                "hostile",
                "mutationRead",
                owner_read(&owner, "context", 1),
                Impact::Read,
            )
            .unwrap();
        let (report, _) = drain_read(&mut read).await;
        assert!(!report.ok, "{scenario}: {report:?}");
        assert_eq!(
            report.retirement,
            Some(Retirement::Protocol),
            "{scenario}: {report:?}"
        );
        assert!(report.batch.is_none());
        assert!(
            supervisor
                .request(
                    "replay",
                    "mutationRead",
                    owner_read(&owner, "context", 1),
                    Impact::Read
                )
                .is_err()
        );
        assert_eq!(
            original.summary.unwrap().outcome,
            crate::drivers::snowflake_batch::Outcome::Pending
        );
        assert!(supervisor.wait_exited().await);
        assert!(
            executed(calls_root.path())
                .iter()
                .all(|sql| sql != "COMMIT" && sql != "ROLLBACK")
        );
        assert_eq!(ledger(calls_root.path())["connections"], 1);
        assert!(!root.exists());
    }
}
