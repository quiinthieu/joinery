//! Concrete consumed-envelope revocation around the closed owning policy.
//! This is a component, not an account entry or an additional execution grant.
//!
//! Synchronous publication callbacks perform only memory/time checks. Full
//! filesystem scans run in a retained blocking worker, outside all policy and
//! process locks. A timeout/drop revokes the attempt; it cannot stop an already
//! running OS read. The single permit stays with that worker until it returns,
//! and the worker can never admit SQL. Cleanup must retain a false settlement
//! result rather than claim that an overdue scanner has finished.
use super::{Plan, envelope::ConsumedEnvelope, policy::Policy};
use crate::{
    domain::{AppError, AppResult, ErrorCode, SessionRuntimeState},
    drivers::{
        snowflake_qualification_metadata::{
            MetadataObserver, RelationObservation, SchemaObservation,
        },
        snowflake_qualification_transport::{
            LocalTerminal, PhysicalRequest, PhysicalTerminal, TransportPolicy,
        },
    },
};
use parking_lot::Mutex;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const CHECK_LIMIT: Duration = Duration::from_secs(30);

struct Shared {
    consumed: Arc<ConsumedEnvelope>,
    policy: Arc<Policy>,
    failure: Mutex<Option<AppError>>,
    slot: Arc<Semaphore>,
}

impl Shared {
    fn fail(&self, error: AppError) -> AppError {
        let first = {
            let mut failure = self.failure.lock();
            failure.get_or_insert(error).clone()
        };
        // Never hold the guard latch across another component's lock/callback.
        self.consumed.fail(first.clone());
        TransportPolicy::failed(&*self.policy, first.clone());
        first
    }

    fn check(&self) -> AppResult<()> {
        if let Some(error) = self.failure.lock().clone() {
            return Err(error);
        }
        self.consumed.check().map_err(|error| self.fail(error))?;
        TransportPolicy::check(&*self.policy).map_err(|error| self.fail(error))?;
        match self.failure.lock().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

struct Interrupted {
    shared: Arc<Shared>,
    armed: bool,
}

impl Drop for Interrupted {
    fn drop(&mut self) {
        if self.armed {
            self.shared.fail(AppError::new(
                ErrorCode::Cancelled,
                "Qualification envelope check or admission was interrupted",
            ));
        }
    }
}

pub(super) struct Guard {
    shared: Arc<Shared>,
    limit: Duration,
    #[cfg(test)]
    pause: Option<Arc<tests::ScanPause>>,
}

impl Guard {
    /// Takes the non-cloneable consumed capability by value. There is no second
    /// constructor accepting an arbitrary policy or a caller-clonable capability.
    pub(super) fn new(consumed: ConsumedEnvelope, policy: Arc<Policy>) -> AppResult<Arc<Self>> {
        let shared = Arc::new(Shared {
            consumed: Arc::new(consumed),
            policy,
            failure: Mutex::new(None),
            slot: Arc::new(Semaphore::new(1)),
        });
        let identity = (|| {
            let plan = shared.consumed.plan();
            let policy_plan = shared.policy.plan();
            if plan.canonical_json()? != policy_plan.canonical_json()?
                || plan.sha256()? != policy_plan.sha256()?
                || shared.consumed.frozen_profile()? != shared.policy.frozen_profile()
            {
                return Err(AppError::invalid(
                    "Consumed qualification envelope does not match its owning policy",
                ));
            }
            shared.check()
        })();
        if let Err(error) = identity {
            return Err(shared.fail(error));
        }
        Ok(Arc::new(Self {
            shared,
            limit: CHECK_LIMIT,
            #[cfg(test)]
            pause: None,
        }))
    }

    pub(super) fn plan(&self) -> Plan {
        self.shared.consumed.plan()
    }

    /// Strictly memory/time only; safe for final publication under Studio locks.
    pub(super) fn check(&self) -> AppResult<()> {
        self.shared.check()
    }

    fn interrupted(&self) -> Interrupted {
        Interrupted {
            shared: self.shared.clone(),
            armed: true,
        }
    }

    async fn scan(&self) -> AppResult<OwnedSemaphorePermit> {
        let permit = self
            .shared
            .slot
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::internal("Qualification check ownership is unavailable"))?;
        self.check()?;
        let shared = self.shared.clone();
        #[cfg(test)]
        let pause = self.pause.clone();
        let worker = tokio::task::spawn_blocking(move || {
            // The worker owns the permit, including after its async waiter drops.
            #[cfg(test)]
            if let Some(pause) = pause {
                pause.wait();
            }
            let result = shared
                .consumed
                .revalidate()
                .map_err(|error| shared.fail(error));
            (permit, result)
        });
        let (permit, result) = worker
            .await
            .map_err(|_| AppError::internal("Qualification envelope check worker failed"))?;
        result?;
        self.check()?;
        Ok(permit)
    }

    fn complete(
        &self,
        result: Result<AppResult<()>, tokio::time::error::Elapsed>,
        interrupted: &mut Interrupted,
    ) -> AppResult<()> {
        let result = match result {
            Ok(result) => result,
            Err(_) => Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Qualification envelope check or admission exceeded its deadline",
            )),
        };
        let result = result.map_err(|error| self.shared.fail(error));
        interrupted.armed = false;
        result
    }

    /// Arming outside the returned future also revokes an unpolled dropped check.
    pub(super) fn revalidate(&self) -> impl Future<Output = AppResult<()>> + Send + '_ {
        let mut interrupted = self.interrupted();
        async move {
            let result = tokio::time::timeout(self.limit, async {
                self.check()?;
                let _permit = self.scan().await?;
                self.check()
            })
            .await;
            self.complete(result, &mut interrupted)
        }
    }

    /// No scan or admission. A false result means a retained blocking scanner or
    /// admission owner has not settled; dropping this waiter never releases it.
    pub(super) async fn wait_for_settlement(&self) -> bool {
        tokio::time::timeout(self.limit, self.shared.slot.clone().acquire_owned())
            .await
            .is_ok_and(|result| result.is_ok())
    }
}

impl TransportPolicy for Guard {
    fn check(&self) -> AppResult<()> {
        Guard::check(self)
    }

    fn failed(&self, error: AppError) {
        self.shared.fail(error);
    }

    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        let _ = self.check();
        // Always delegate the original after failure, including any mutation ACK.
        if let Err(error) = self.shared.policy.terminal(terminal) {
            self.shared.fail(error);
        }
        self.check()
    }

    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        let _ = self.check();
        if let Err(error) = self.shared.policy.local_terminal(terminal) {
            self.shared.fail(error);
        }
        self.check()
    }

    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        let mut interrupted = self.interrupted();
        Box::pin(async move {
            let result = tokio::time::timeout(self.limit, async {
                self.check()?;
                let _permit = self.scan().await?;
                self.check()?;
                self.shared.policy.admit(request).await?;
                self.check()
            })
            .await;
            self.complete(result, &mut interrupted)
        })
    }
}

impl MetadataObserver for Guard {
    fn check(&self) -> AppResult<()> {
        Guard::check(self)
    }

    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.shared
            .policy
            .observe_schemas(observation)
            .map_err(|error| self.shared.fail(error))?;
        self.check()
    }

    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        self.check()?;
        self.shared
            .policy
            .observe_relations(observation)
            .map_err(|error| self.shared.fail(error))?;
        self.check()
    }

    fn observe_runtime(&self, state: &SessionRuntimeState) -> AppResult<()> {
        self.check()?;
        self.shared
            .policy
            .observe_runtime(state)
            .map_err(|error| self.shared.fail(error))?;
        self.check()
    }
}

#[cfg(test)]
#[path = "snowflake_table_qualification_guard_tests.rs"]
mod tests;
