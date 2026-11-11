use super::*;
use crate::domain::ErrorCode;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Observer {
    calls: AtomicUsize,
    deny: AtomicUsize,
}

impl MetadataObserver for Observer {
    fn check(&self) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.deny.load(Ordering::SeqCst) != 0 {
            Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "private first failure",
            ))
        } else {
            Ok(())
        }
    }
    fn observe_schemas(&self, _: SchemaObservation<'_>) -> AppResult<()> {
        unreachable!()
    }
    fn observe_relations(&self, _: RelationObservation<'_>) -> AppResult<()> {
        unreachable!()
    }
}

#[test]
fn qualification_metadata_attachment_clones_keep_first_failure_and_do_not_reenter_policy() {
    let observer = Arc::new(Observer::default());
    let attachment = MetadataAttachment::new(observer.clone());
    attachment.check().unwrap();
    observer.deny.store(1, Ordering::SeqCst);
    let original = attachment.check().unwrap_err();
    let count = observer.calls.load(Ordering::SeqCst);
    observer.deny.store(0, Ordering::SeqCst);
    let retained = attachment.clone().check().unwrap_err();
    assert_eq!(retained.code, original.code);
    assert_eq!(retained.message, original.message);
    assert_eq!(observer.calls.load(Ordering::SeqCst), count);
}
