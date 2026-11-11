//! Internal, harness-only attachment for one qualification connection.
//! This binds transport and metadata failure handling; it is not an execution
//! envelope, closed SQL ledger, journal, account approval or desktop capability.
use super::{
    snowflake_qualification_metadata::{
        MetadataAttachment, MetadataObserver, RelationObservation, SchemaObservation,
    },
    snowflake_qualification_transport::{
        PhysicalRequest, PhysicalTerminal, TransportAttachment, TransportLimits, TransportPolicy,
    },
};
use crate::domain::{AppError, AppResult};
use parking_lot::Mutex;
use std::{future::Future, pin::Pin, sync::Arc};

/// Consumed by Studio's Rust-only qualification connect route. Both attachment
/// interfaces use the same policy and failure latch. No profile or IPC can set it.
pub(crate) struct QualificationAttachment {
    pub(crate) transport: TransportAttachment,
    pub(crate) metadata: MetadataAttachment,
    /// Deterministic connection-publication race seam; absent from released builds.
    #[cfg(test)]
    pub(crate) before_publication: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Drop for QualificationAttachment {
    fn drop(&mut self) {
        // Launch owns settlement once claimed. Before that point, disposal is
        // the only proof that no future launch can start counter tasks.
        let _ = self.transport.settle_unstarted();
    }
}

impl QualificationAttachment {
    pub(crate) fn new<T>(policy: Arc<T>, limits: TransportLimits) -> AppResult<Self>
    where
        T: TransportPolicy + MetadataObserver + 'static,
    {
        let shared = Arc::new(SharedPolicy {
            policy,
            failure: Mutex::new(None),
        });
        Ok(Self {
            transport: TransportAttachment::new(shared.clone(), limits)?,
            metadata: MetadataAttachment::new(shared),
            #[cfg(test)]
            before_publication: None,
        })
    }
}

struct SharedPolicy<T> {
    policy: Arc<T>,
    failure: Mutex<Option<AppError>>,
}

impl<T: TransportPolicy + MetadataObserver> SharedPolicy<T> {
    fn record(&self, result: AppResult<()>) -> AppResult<()> {
        let mut failure = self.failure.lock();
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
        failure.clone().map_or(Ok(()), Err)
    }

    fn check(&self) -> AppResult<()> {
        if let Some(error) = self.failure.lock().clone() {
            return Err(error);
        }
        self.record(TransportPolicy::check(self.policy.as_ref()))?;
        self.record(MetadataObserver::check(self.policy.as_ref()))
    }
}

impl<T: TransportPolicy + MetadataObserver> TransportPolicy for SharedPolicy<T> {
    fn check(&self) -> AppResult<()> {
        SharedPolicy::check(self)
    }

    fn failed(&self, error: AppError) {
        let _ = self.record(Err(error));
    }

    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        // Preserve observation of original ACKs even after cross-domain failure.
        self.record(self.policy.terminal(terminal))
    }

    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.check()?;
            self.record(self.policy.admit(request).await)?;
            self.check()
        })
    }
}

impl<T: TransportPolicy + MetadataObserver> MetadataObserver for SharedPolicy<T> {
    fn check(&self) -> AppResult<()> {
        SharedPolicy::check(self)
    }

    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.record(self.policy.observe_schemas(observation))?;
        self.check()
    }

    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.record(self.policy.observe_relations(observation))?;
        self.check()
    }
}
