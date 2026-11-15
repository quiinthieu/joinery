use super::*;
use crate::drivers::{
    snowflake_qualification::QualificationAttachment,
    snowflake_qualification_metadata::MetadataObserver,
    snowflake_qualification_transport::{
        PhysicalRequest, PhysicalTerminal, TransportLimits, TransportPolicy,
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicUsize, mpsc},
    task::Poll,
};

struct HeldQueryTerminal {
    armed: AtomicBool,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    original: Mutex<Option<Report>>,
    controls: AtomicUsize,
    deny: bool,
}

fn callback_denied() -> AppError {
    AppError::new(
        ErrorCode::PolicyBlocked,
        "synthetic finalizing query denial",
    )
}

impl MetadataObserver for HeldQueryTerminal {
    fn check(&self) -> AppResult<()> {
        Ok(())
    }
    fn observe_schemas(&self, _: SchemaObservation<'_>) -> AppResult<()> {
        Ok(())
    }
    fn observe_relations(&self, _: RelationObservation<'_>) -> AppResult<()> {
        Ok(())
    }
}

impl TransportPolicy for HeldQueryTerminal {
    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if request.operation == "credit" {
                self.controls.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        })
    }
    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        if terminal.operation == "query" && self.armed.swap(false, Ordering::SeqCst) {
            *self.original.lock() = Some(terminal.report.clone());
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .expect("bounded original terminal release");
            if self.deny {
                return Err(callback_denied());
            }
        }
        Ok(())
    }
}

struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
impl ReleaseOnDrop {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.release();
    }
}

struct FinalizingSink {
    terminal_entered: mpsc::Receiver<()>,
    rows_returned: Arc<AtomicBool>,
    discard: bool,
}
impl QuerySink for FinalizingSink {
    fn limit(&mut self, _: &str) {
        assert!(self.discard, "one retained synthetic row must not truncate");
    }
    fn describe(&mut self, _: Vec<ColumnInfo>) -> AppResult<()> {
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        assert_eq!(rows.len(), 1);
        // The query has left pending, but its original watch must remain empty
        // until this held observer returns. Force credit into that exact window.
        self.terminal_entered
            .recv_timeout(Duration::from_secs(5))
            .expect("original helper terminal enters observer");
        self.rows_returned.store(true, Ordering::SeqCst);
        Ok(!self.discard)
    }
    fn complete(&mut self, rows_seen: u64) -> AppResult<()> {
        assert_eq!(rows_seen, 1);
        Ok(())
    }
}

async fn finalizing_query(deny: bool, discard: bool) {
    let (entered, terminal_entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let mut release = ReleaseOnDrop(Some(release));
    let policy = Arc::new(HeldQueryTerminal {
        armed: AtomicBool::new(false),
        entered,
        release: Mutex::new(release_rx),
        original: Mutex::new(None),
        controls: AtomicUsize::new(0),
        deny,
    });
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let h = Harness::open_policy_and_attachments(
        SafeMode::ReadOnly,
        false,
        false,
        Some(attachment.metadata.clone()),
        Some(attachment.transport.clone()),
    )
    .await;
    let before = h.queries().len();
    let controls_before = policy.controls.load(Ordering::SeqCst);
    policy.armed.store(true, Ordering::SeqCst);
    let rows_returned = Arc::new(AtomicBool::new(false));
    let mut sink = FinalizingSink {
        terminal_entered,
        rows_returned: rows_returned.clone(),
        discard,
    };
    let spec = metadata::version_read();
    let mut stream = Box::pin(h.driver.stream_bounded(
        spec.sql(),
        vec![],
        &mut sink,
        spec.max_rows(),
        spec.max_bytes(),
    ));
    let at_credit = std::future::poll_fn(|cx| {
        let progress = stream.as_mut().poll(cx);
        if rows_returned.load(Ordering::SeqCst) || progress.is_ready() {
            Poll::Ready(progress)
        } else {
            Poll::Pending
        }
    })
    .await;
    assert!(
        at_credit.is_pending(),
        "a finalizing original must be awaited, not close the session: {at_credit:?}"
    );
    assert!(!h.driver.is_closed());
    let terminal = h.driver.active.lock().as_ref().unwrap().terminal.clone();
    assert!(terminal.borrow().is_none());
    let original = policy.original.lock().clone().unwrap();
    assert!(original.ok && original.retirement.is_none());
    assert_eq!(original.certainty, Certainty::StatementSucceeded);
    release.release();
    let result = stream.await;
    if deny {
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::PolicyBlocked);
        assert_eq!(error.message, callback_denied().message);
        assert!(attachment.transport.check().is_err());
    } else {
        assert_eq!(result.unwrap(), 1);
        assert!(attachment.transport.check().is_ok());
        assert!(!h.driver.is_closed());
    }
    assert_eq!(h.queries().len(), before + 1, "no SQL retry or extra read");
    assert_eq!(
        policy.controls.load(Ordering::SeqCst),
        controls_before,
        "an original already in finalization needs no new physical credit/discard"
    );
    assert_eq!(
        format!("{:?}", terminal.borrow().as_ref().unwrap()),
        format!("{original:?}"),
        "callback denial cannot replace the original helper report"
    );
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_finalizing_query_waits_for_original_publication() {
    finalizing_query(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_finalizing_query_denial_preserves_original_and_blocks_success() {
    finalizing_query(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_finalizing_query_discard_waits_for_original_publication() {
    finalizing_query(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_finalizing_query_discard_denial_preserves_original_and_blocks_success() {
    finalizing_query(true, true).await;
}
