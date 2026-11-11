//! Harness-only, per-attempt observations of production-validated metadata.
//! No SQL, object selection, account policy, or qualification runner lives here.

use super::{snowflake_browsing::BrowseRelation, snowflake_metadata::LiveContext};
use crate::domain::{AppError, AppResult};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataSite {
    CreateTargetCheck,
    SessionContextOptions,
    RelationCatalogBatch,
    RelationCatalogSnapshot,
    ExactRelationCheck,
    SchemaSwitch,
    CompletionCatalog,
}

pub(crate) struct SchemaObservation<'a> {
    pub site: MetadataSite,
    pub context: &'a LiveContext,
    /// Original row order, recovered only after the production parser succeeds.
    pub wire_names: &'a [String],
    /// The production parser's sorted, validated names.
    pub parsed_names: &'a [String],
}

pub(crate) struct RelationObservation<'a> {
    pub site: MetadataSite,
    pub context: &'a LiveContext,
    /// None only for the complete, aggregate-validated catalog snapshot.
    pub schema: Option<&'a str>,
    /// Retains CREATED/LAST_DDL and traits omitted by public target summaries.
    pub relations: &'a [BrowseRelation],
}

/// Callbacks are synchronous and must not reenter the adapter, Studio, or its
/// connection registry. A composite
/// policy may implement this and TransportPolicy on one Arc to retain the first
/// private failure across both seams. `check` also blocks publication after a
/// failure observed by that other seam. No default policy grants any authority.
pub(crate) trait MetadataObserver: Send + Sync {
    /// Synchronous, in-memory check: no I/O and no adapter, Studio, or registry
    /// reentry. Publication may call this while holding their state locks.
    fn check(&self) -> AppResult<()>;
    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()>;
    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()>;
}

#[derive(Clone)]
pub(crate) struct MetadataAttachment {
    observer: Arc<dyn MetadataObserver>,
    failure: Arc<Mutex<Option<AppError>>>,
}

impl MetadataAttachment {
    pub(crate) fn new(observer: Arc<dyn MetadataObserver>) -> Self {
        Self {
            observer,
            failure: Arc::new(Mutex::new(None)),
        }
    }

    fn retain<T>(&self, result: AppResult<T>) -> AppResult<T> {
        let mut failure = self.failure.lock();
        if let Err(error) = &result
            && failure.is_none()
        {
            *failure = Some(error.clone());
        }
        match failure.as_ref() {
            Some(error) => Err(error.clone()),
            None => result,
        }
    }

    pub(crate) fn check(&self) -> AppResult<()> {
        if let Some(error) = self.failure.lock().clone() {
            return Err(error);
        }
        self.retain(self.observer.check())
    }

    pub(crate) fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.retain(self.observer.observe_schemas(observation))?;
        self.check()
    }

    pub(crate) fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.retain(self.observer.observe_relations(observation))?;
        self.check()
    }
}

#[cfg(test)]
#[path = "snowflake_qualification_metadata_tests.rs"]
mod tests;
