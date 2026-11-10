//! Local synthetic protocol child; never connects to an external service.
#[cfg(unix)]
mod fixture {
    use joinery_core::drivers::snowflake_process::{MAX_BATCH, MAX_FRAME, read_frame, write_frame};
    use serde_json::{Value, json};
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, stdin, stdout};

    async fn event(output: &mut tokio::io::Stdout, request: &Value, kind: &str, extra: Value) {
        let mut frame =
            json!({"v":1,"generation":request["generation"],"id":request["id"],"type":kind});
        frame
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        write_frame(output, &frame).await.unwrap();
    }
    async fn done(output: &mut tokio::io::Stdout, request: &Value, result: Value) {
        event(
            output,
            request,
            "terminal",
            json!({"ok":true,"result":result}),
        )
        .await;
    }
    #[tokio::main]
    pub async fn run() {
        let root =
            std::path::PathBuf::from(std::env::var_os("JOINERY_SNOWFLAKE_STATE_DIR").unwrap());
        // Publish the marker only after the PID is complete: existence is the
        // parent fixture's readiness signal, including during runtime teardown.
        std::fs::write(root.join("child.starting"), std::process::id().to_string()).unwrap();
        std::fs::rename(root.join("child.starting"), root.join("child.started")).unwrap();
        if std::path::Path::new("hold-hello").exists() {
            std::future::pending::<()>().await;
        }
        let mut input = stdin();
        let mut output = stdout();
        write_frame(&mut output,&json!({"v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1",
        "limits":{"frameBytes":MAX_FRAME,"batchRows":128,"batchBytes":MAX_BATCH,"queued":8,"credits":32}})).await.unwrap();
        let mut scenario = String::new();
        let mut active: Option<Value> = None;
        let mut executions = 0;
        let mut last_id = 0u64;
        let mut emitted = 0u64;
        while let Some(request) = read_frame(&mut input, Duration::from_secs(2))
            .await
            .unwrap()
        {
            if last_id == 0 {
                assert_eq!(request["op"], "connect");
            }
            let wire_id = request["id"].as_str().unwrap().parse::<u64>().unwrap();
            assert!(
                wire_id > last_id,
                "wire sequence must increase at transmission"
            );
            last_id = wire_id;
            event(&mut output, &request, "accepted", json!({})).await;
            match request["op"].as_str().unwrap() {
                "connect" => {
                    scenario = request["payload"]["scenario"]
                        .as_str()
                        .unwrap_or("healthy")
                        .into();
                    if scenario == "auth_hang" {
                        active = Some(request);
                        continue;
                    }
                    done(&mut output, &request, json!({"connected":true})).await;
                    if scenario == "stop_reading" {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                }
                "query" => {
                    executions += 1;
                    match scenario.as_str() {
                        "crash_no_phase" => std::process::exit(0),
                        "wrong_generation" => {
                            write_frame(&mut output,&json!({"v":1,"generation":"00000000-0000-4000-8000-000000000099","id":request["id"],"type":"terminal","ok":true,"result":{}})).await.unwrap();
                        }
                        "wrong_id" => {
                            let mut forged = request.clone();
                            forged["id"] = json!("not-pending");
                            done(&mut output, &forged, json!({})).await;
                        }
                        "oversize" => {
                            output
                                .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
                                .await
                                .unwrap();
                            output.flush().await.unwrap();
                        }
                        "partial" => {
                            output.write_all(&[0]).await.unwrap();
                            output.flush().await.unwrap();
                        }
                        "invalid_json" => {
                            output.write_all(&[0, 0, 0, 1, b'{']).await.unwrap();
                            output.flush().await.unwrap();
                        }
                        "secret_error" => {
                            event(&mut output,&request,"terminal",json!({"ok":false,"error":{"code":"database","message":request["payload"]["sql"]}})).await;
                        }
                        "crash_dispatch" => {
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"dispatching"}),
                            )
                            .await;
                            std::process::exit(0);
                        }
                        "crash_ack" => {
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"dispatching"}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"acknowledged"}),
                            )
                            .await;
                            std::process::exit(0);
                        }
                        "early_columns" => {
                            event(&mut output, &request, "columns", json!({"columns":[]})).await;
                        }
                        "duplicate_accepted" => {
                            event(&mut output, &request, "accepted", json!({})).await;
                        }
                        "statement_ids" => {
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"dispatching"}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "statement",
                                json!({"sdkRequestId":"00000000-0000-4000-8000-000000000003"}),
                            )
                            .await;
                            event(&mut output,&request,"statement",json!({"sdkRequestId":"00000000-0000-4000-8000-000000000003","queryId":"00000000-0000-4000-8000-000000000004"})).await;
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"acknowledged"}),
                            )
                            .await;
                            std::process::exit(0);
                        }
                        "hang" => {
                            active = Some(request);
                        }
                        "late_error" => {
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"dispatching"}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"acknowledged"}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "columns",
                                json!({"columns":[{"name":"x","type":"TEXT"}]}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "rows",
                                json!({"rows":[["retained"]]}),
                            )
                            .await;
                            event(&mut output,&request,"terminal",json!({"ok":false,"error":{"code":"database","message":"static error"}})).await;
                        }
                        _ => {
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"dispatching"}),
                            )
                            .await;
                            event(
                                &mut output,
                                &request,
                                "phase",
                                json!({"phase":"acknowledged"}),
                            )
                            .await;
                            event(&mut output,&request,"columns",json!({"columns":[{"name":"same","type":"NUMBER"},{"name":"same","type":"TEXT"}]})).await;
                            if scenario == "without_credit" {
                                event(&mut output, &request, "rows", json!({"rows":[["1",null]]}))
                                    .await;
                            } else if matches!(
                                scenario.as_str(),
                                "backpressure" | "cancel_ack_only" | "million"
                            ) {
                                active = Some(request);
                                emitted = 0;
                            } else {
                                if request["payload"]["credits"].as_u64().unwrap_or(0) > 0 {
                                    event(
                                        &mut output,
                                        &request,
                                        "rows",
                                        json!({"rows":[["9007199254740993","exact"]]}),
                                    )
                                    .await;
                                }
                                done(
                                    &mut output,
                                    &request,
                                    json!({"rowsSeen":1,"rowsRetained":1,"truncated":false}),
                                )
                                .await;
                            }
                        }
                    }
                }
                "context" => {
                    event(
                        &mut output,
                        &request,
                        "phase",
                        json!({"phase":"dispatching"}),
                    )
                    .await;
                    event(
                        &mut output,
                        &request,
                        "phase",
                        json!({"phase":"acknowledged"}),
                    )
                    .await;
                    done(&mut output,&request,json!({"executions":executions,"args":std::env::args().skip(1).collect::<Vec<_>>(),"env":std::env::vars().collect::<Vec<_>>()})).await;
                }
                "credit" => {
                    done(&mut output, &request, json!({})).await;
                    if let Some(query) = &active {
                        if request["payload"]["discard"] == true {
                            done(
                                &mut output,
                                query,
                                json!({"rowsSeen":999,"rowsRetained":0,"truncated":true}),
                            )
                            .await;
                            active = None;
                        } else if scenario == "million" {
                            for _ in 0..request["payload"]["batches"].as_u64().unwrap_or(0) {
                                let count = 128.min(1_000_000 - emitted);
                                let rows: Vec<_> = (emitted..emitted + count)
                                    .map(|n| vec![Some(n.to_string()), Some("exact".to_owned())])
                                    .collect();
                                event(&mut output, query, "rows", json!({"rows":rows})).await;
                                emitted += count;
                                if emitted == 1_000_000 {
                                    done(&mut output,query,json!({"rowsSeen":emitted,"rowsRetained":emitted,"truncated":false})).await;
                                    active = None;
                                    break;
                                }
                            }
                        } else {
                            for _ in 0..request["payload"]["batches"].as_u64().unwrap_or(0) {
                                event(&mut output, query, "rows", json!({"rows":[["1",null]]}))
                                    .await;
                            }
                        }
                    }
                }
                "cancel" => {
                    done(&mut output, &request, json!({"cancelAcknowledged":true})).await;
                    if scenario != "cancel_ack_only"
                        && let Some(query) = active.take()
                    {
                        event(
                        &mut output,
                        &query,
                        "terminal",
                        json!({"ok":false,"error":{"code":"cancelled","message":"static cancel"}}),
                    )
                    .await;
                    }
                    if scenario == "auth_hang" {
                        break;
                    }
                }
                "close" => {
                    if scenario == "wedged_close" {
                        continue;
                    }
                    done(&mut output, &request, json!({"closed":true})).await;
                    break;
                }
                _ => unreachable!(),
            }
        }
    }
}
#[cfg(unix)]
fn main() {
    fixture::run();
}
#[cfg(not(unix))]
fn main() {}
