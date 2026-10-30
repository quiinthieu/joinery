use crate::{
    domain::*,
    drivers::{
        Driver, QuerySink, RowMutation,
        mysql::Mysql,
        postgres::{Postgres, mutation_sql},
        redis::Redis,
        snowflake::Snowflake,
    },
    persistence::Persistence,
    results::{RESULT_BUDGET, ROW_LIMIT, ResultStore},
    safety,
    secrets::{OsSecrets, SecretStore},
    ssh::{SshManager, Tunnel, host_setting},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[cfg(test)]
mod recovery_tests;

pub type EventHandler = Arc<dyn Fn(QueryEvent) -> bool + Send + Sync>;

struct Session {
    info: SessionInfo,
    profile: ConnectionProfile,
    driver: Arc<dyn Driver>,
    lane: tokio::sync::Mutex<()>,
    cancellation_gate: tokio::sync::Mutex<()>,
    active_run: Mutex<Option<String>>,
    closed: AtomicBool,
    tunnel: Option<Arc<Tunnel>>,
    completion: Mutex<Option<CompletionCatalog>>,
}

enum Action {
    Sql,
    Browse {
        table: TableRef,
        query: BrowseQuery,
    },
    Changes {
        schema: TableSchema,
        rows: Vec<RowMutation>,
    },
    CancelProcess {
        id: String,
        identity: String,
    },
}

struct StoredPlan {
    plan: PreparedPlan,
    action: Action,
    deadline: Instant,
    approval: Option<String>,
    digest: String,
    bytes: usize,
}

struct Job {
    session_id: String,
    token: CancellationToken,
    cancellation_sent: Arc<AtomicBool>,
}

pub struct Studio {
    persistence: Persistence,
    secrets: Arc<dyn SecretStore>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    plans: Mutex<HashMap<String, StoredPlan>>,
    jobs: Mutex<HashMap<String, Job>>,
    result_sessions: Mutex<HashMap<String, String>>,
    pub results: ResultStore,
    connect_gate: tokio::sync::Mutex<()>,
    configuration_gate: Mutex<()>,
    tunnels: SshManager,
}

impl Studio {
    pub fn open(root: &Path) -> AppResult<Arc<Self>> {
        Self::with_secrets(root, OsSecrets::initialize())
    }
    pub fn with_secrets(root: &Path, secrets: Arc<dyn SecretStore>) -> AppResult<Arc<Self>> {
        Ok(Arc::new(Self {
            persistence: Persistence::open(root)?,
            secrets,
            sessions: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            result_sessions: Mutex::new(HashMap::new()),
            results: ResultStore::default(),
            connect_gate: tokio::sync::Mutex::new(()),
            configuration_gate: Mutex::new(()),
            tunnels: SshManager::default(),
        }))
    }

    pub fn bootstrap(&self) -> AppResult<Bootstrap> {
        Ok(Bootstrap {
            profiles: self.persistence.profiles()?,
            groups: self.persistence.groups()?,
            passphrase_configured: self
                .persistence
                .setting("passphrase-configured")?
                .as_deref()
                == Some("true"),
            supported_engines: vec![
                EngineKind::Postgres,
                EngineKind::Redis,
                EngineKind::Mysql,
                EngineKind::Snowflake,
            ],
            result_budget_bytes: RESULT_BUDGET,
            row_limit: ROW_LIMIT,
            workspace: self.persistence.workspace()?,
        })
    }

    pub fn metadata_backups(&self) -> AppResult<Vec<MetadataBackup>> {
        self.persistence.metadata_backups()
    }

    pub fn save_workspace(&self, workspace: SavedWorkspace) -> AppResult<()> {
        let _configuration = self.configuration_gate.lock();
        self.persistence.save_workspace(workspace)
    }

    pub fn create_metadata_backup(&self) -> AppResult<MetadataBackup> {
        self.persistence.create_metadata_backup()
    }

    pub fn delete_metadata_backup(&self, id: &str, digest: &str) -> AppResult<()> {
        self.persistence.delete_metadata_backup(id, digest)
    }

    pub fn restore_metadata_backup(
        &self,
        id: &str,
        digest: &str,
        confirmation: &str,
        passphrase: Option<&str>,
    ) -> AppResult<MetadataBackup> {
        if confirmation != "RESTORE" {
            return Err(AppError::invalid("Type RESTORE to replace local metadata"));
        }
        let _connections = self.connect_gate.try_lock().map_err(|_| {
            AppError::invalid("Wait for connection setup to finish before restoring")
        })?;
        let _configuration = self.configuration_gate.lock();
        if !self.sessions.lock().is_empty()
            || !self.jobs.lock().is_empty()
            || self.results.usage().1 > 0
        {
            return Err(AppError::invalid(
                "Disconnect every connection and close all result tabs before restoring metadata",
            ));
        }
        if self
            .persistence
            .setting("passphrase-configured")?
            .as_deref()
            == Some("true")
        {
            self.verify_passphrase(passphrase)?;
        }
        let undo = self.persistence.restore_metadata_backup(id, digest)?;
        self.plans.lock().clear();
        Ok(undo)
    }

    pub fn save_profile(
        &self,
        mut profile: ConnectionProfile,
        passphrase: Option<&str>,
    ) -> AppResult<ConnectionProfile> {
        let _configuration = self.configuration_gate.lock();
        if profile.id.is_empty() {
            profile.id = new_id();
        }
        uuid::Uuid::parse_str(&profile.id)
            .map_err(|_| AppError::invalid("Invalid connection identifier"))?;
        if profile.name.trim().is_empty()
            || profile.name.len() > 100
            || profile.host.trim().is_empty()
            || profile.host.len() > 253
            || profile.port == 0
            || profile.username.is_empty()
            || profile.username.len() > 1024
            || profile.database.is_empty()
            || profile.database.len() > 1024
            || profile
                .ca_cert_path
                .as_ref()
                .is_some_and(|p| p.len() > 4096)
        {
            return Err(AppError::invalid(
                "Provide a name, host, port, database, and username",
            ));
        }
        let profiles = self.persistence.profiles()?;
        if !profiles.iter().any(|p| p.id == profile.id) && profiles.len() >= 256 {
            return Err(AppError::invalid(
                "At most 256 connection profiles may be stored",
            ));
        }
        if let Some(ssh) = &profile.ssh
            && (ssh.host.trim().is_empty()
                || ssh.host.len() > 253
                || ssh.port == 0
                || ssh.username.is_empty()
                || ssh.username.len() > 255
                || ssh
                    .private_key_path
                    .as_ref()
                    .is_some_and(|p| p.len() > 4096))
        {
            return Err(AppError::invalid(
                "Provide a valid SSH host, port, username, and key path",
            ));
        }
        let groups = self.persistence.groups()?;
        let group = groups
            .iter()
            .find(|g| g.id == profile.group_id)
            .ok_or_else(|| AppError::invalid("Choose an existing connection group"))?;
        profile.environment = group.environment;
        if profile.environment == Environment::Production && profile.safe_mode != SafeMode::ReadOnly
        {
            profile.safe_mode = SafeMode::PasswordGated;
        }
        if profile.tls_mode == TlsMode::Disable && profile.environment == Environment::Production {
            return Err(AppError::invalid(
                "Production connections require verified TLS",
            ));
        }
        if profile.engine == EngineKind::Redis {
            Redis::validate_profile(&profile)?;
        }
        if profile.engine == EngineKind::Snowflake {
            Snowflake::validate_profile(&profile)?;
        }
        if let Some(previous) = profiles
            .iter()
            .find(|p| p.id == profile.id && p.environment == Environment::Production)
        {
            let previous_json = serde_json::to_value(previous)
                .map_err(|_| AppError::internal("Could not compare connection settings"))?;
            let proposed_json = serde_json::to_value(&profile)
                .map_err(|_| AppError::internal("Could not compare connection settings"))?;
            if previous_json != proposed_json {
                self.verify_passphrase(passphrase)?;
            }
        }
        self.close_profile_sessions(&profile.id);
        if profiles
            .iter()
            .any(|p| p.id == profile.id && p.remember_password)
            && !profile.remember_password
        {
            self.secrets.delete(&format!("database:{}", profile.id))?;
        }
        self.persistence.save_profile(&profile)?;
        Ok(profile)
    }

    pub fn save_group(&self, mut group: ConnectionGroup) -> AppResult<ConnectionGroup> {
        let _configuration = self.configuration_gate.lock();
        let groups = self.persistence.groups()?;
        let previous = groups.iter().find(|g| g.id == group.id);
        if previous.is_none() && (uuid::Uuid::parse_str(&group.id).is_err() || groups.len() >= 256)
        {
            return Err(AppError::invalid(
                "Use a valid group identifier; at most 256 groups may be stored",
            ));
        }
        group.name = group.name.trim().to_string();
        if group.name.is_empty() || group.name.len() > 100 {
            return Err(AppError::invalid("Use a group name of 1 to 100 bytes"));
        }
        let mut current = group.parent_id.as_deref();
        let mut depth = 0;
        while let Some(id) = current {
            if id == group.id || depth >= 7 {
                return Err(AppError::invalid(
                    "Groups cannot form a cycle or exceed eight levels",
                ));
            }
            let parent = groups
                .iter()
                .find(|g| g.id == id)
                .ok_or_else(|| AppError::invalid("Choose an existing parent group"))?;
            if depth == 0 {
                group.environment = parent.environment;
            }
            current = parent.parent_id.as_deref();
            depth += 1;
        }
        if previous.is_some_and(|old| old.environment != group.environment) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "A group's environment cannot change. Create a new group in the desired environment, then move profiles individually.",
            ));
        }
        // Count the deepest descendant too when moving an existing subtree.
        for descendant in &groups {
            let mut id = descendant.parent_id.as_deref();
            let mut below = 0;
            while let Some(parent) = id {
                below += 1;
                if parent == group.id && depth + below >= 8 {
                    return Err(AppError::invalid(
                        "This move would exceed eight group levels",
                    ));
                }
                if below > 256 {
                    return Err(AppError::invalid("The stored group hierarchy is invalid"));
                }
                id = groups
                    .iter()
                    .find(|g| g.id == parent)
                    .and_then(|g| g.parent_id.as_deref());
            }
        }
        self.persistence.save_group(&group)?;
        Ok(group)
    }

    pub fn delete_group(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_gate.lock();
        let groups = self.persistence.groups()?;
        if ["production", "staging", "local"].contains(&id) {
            return Err(AppError::invalid(
                "The default environment groups cannot be removed",
            ));
        }
        if !groups.iter().any(|g| g.id == id) {
            return Err(AppError::new(ErrorCode::NotFound, "Group no longer exists"));
        }
        if groups.iter().any(|g| g.parent_id.as_deref() == Some(id))
            || self
                .persistence
                .profiles()?
                .iter()
                .any(|p| p.group_id == id)
        {
            return Err(AppError::invalid(
                "Move this group's connections and child groups before removing it",
            ));
        }
        self.persistence.delete_group(id)
    }

    pub fn delete_profile(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_gate.lock();
        let profile = self.persistence.profile(id)?;
        self.close_profile_sessions(id);
        if profile.remember_password {
            self.secrets.delete(&format!("database:{id}"))?;
        }
        self.persistence.delete_profile(id)
    }

    pub async fn connect(&self, id: &str, secrets: ConnectionSecrets) -> AppResult<SessionInfo> {
        let _gate = self.connect_gate.lock().await;
        let profile = self.persistence.profile(id)?;
        if self
            .sessions
            .lock()
            .values()
            .filter(|s| s.profile.id == id)
            .count()
            >= 4
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "This connection already has four open sessions",
            ));
        }
        let password = match secrets.password {
            Some(value) => Some(Zeroizing::new(value)),
            None if profile.remember_password => self
                .secrets
                .get(&format!("database:{id}"))?
                .map(Zeroizing::new),
            None => None,
        };
        let session_id = new_id();
        let tunnel = if let Some(ssh) = &profile.ssh {
            Some(
                self.tunnels
                    .acquire(
                        &profile,
                        self.persistence.setting(&host_setting(ssh))?,
                        secrets.ssh_password.map(Zeroizing::new),
                        secrets.key_passphrase.map(Zeroizing::new),
                    )
                    .await?,
            )
        } else {
            None
        };
        let (driver, info): (Arc<dyn Driver>, SessionInfo) = match profile.engine {
            EngineKind::Redis => {
                let (driver, info) = Redis::open(
                    &profile,
                    password.as_deref().map(String::as_str),
                    &session_id,
                    tunnel.as_ref().map(|t| t.port),
                )
                .await?;
                (driver, info)
            }
            EngineKind::Mysql => {
                let (driver, info) = Mysql::open(
                    &profile,
                    password.as_deref().map(String::as_str),
                    &session_id,
                    tunnel.as_ref().map(|t| t.port),
                )
                .await?;
                (driver, info)
            }
            EngineKind::Postgres => {
                let (d, i) = Postgres::open(
                    &profile,
                    password.as_deref().map(String::as_str),
                    &session_id,
                    tunnel.as_ref().map(|t| t.port),
                )
                .await?;
                (d, i)
            }
            EngineKind::Snowflake => {
                if password.is_some() {
                    return Err(AppError::invalid(
                        "Snowflake browser sign-in does not use a password",
                    ));
                }
                let (d, i) = Snowflake::open(&profile, &session_id).await?;
                (d, i)
            }
        };
        let _configuration = self.configuration_gate.lock();
        if self
            .persistence
            .profile(id)
            .ok()
            .and_then(|current| serde_json::to_string(&current).ok())
            != serde_json::to_string(&profile).ok()
        {
            driver.close();
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection profile changed while connecting. Connect again using its current settings.",
            ));
        }
        if profile.remember_password
            && let Some(password) = password.as_ref()
            && let Err(error) = self.secrets.set(&format!("database:{id}"), password)
        {
            driver.close();
            return Err(error);
        }
        self.sessions.lock().insert(
            session_id,
            Arc::new(Session {
                info: info.clone(),
                profile,
                driver,
                lane: tokio::sync::Mutex::new(()),
                completion: Mutex::new(None),
                cancellation_gate: tokio::sync::Mutex::new(()),
                active_run: Mutex::new(None),
                closed: AtomicBool::new(false),
                tunnel,
            }),
        );
        Ok(info)
    }

    fn session(&self, id: &str) -> AppResult<Arc<Session>> {
        let session = self.sessions.lock().get(id).cloned().ok_or_else(|| {
            AppError::new(ErrorCode::ConnectionClosed, "Connect to a database first")
        })?;
        if session.closed.load(Ordering::Acquire)
            || session.driver.is_closed()
            || session.tunnel.as_ref().is_some_and(|t| t.is_closed())
        {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        Ok(session)
    }

    pub async fn probe_ssh(&self, profile_id: &str) -> AppResult<SshHostKey> {
        let profile = self.persistence.profile(profile_id)?;
        let ssh = profile
            .ssh
            .as_ref()
            .ok_or_else(|| AppError::invalid("SSH is not configured"))?;
        self.tunnels
            .probe(ssh, self.persistence.setting(&host_setting(ssh))?)
            .await
    }

    pub fn trust_ssh_key(&self, challenge_id: &str, fingerprint: &str) -> AppResult<()> {
        let (host, line) = self.tunnels.accept(challenge_id, fingerprint)?;
        let setting = host_setting(&host);
        if self
            .persistence
            .setting(&setting)?
            .is_some_and(|previous| previous != line)
        {
            return Err(AppError::new(
                ErrorCode::SshHostKeyChanged,
                "A different host key is already trusted. Connection blocked.",
            ));
        }
        self.persistence.set_setting(&setting, &line)?;
        self.persistence
            .audit("ssh", "trust-host", fingerprint, "accepted")
    }

    pub fn disconnect(&self, id: &str) {
        if let Some(session) = self.sessions.lock().remove(id) {
            session.closed.store(true, Ordering::Release);
            session.driver.close();
        }
        self.plans.lock().retain(|_, p| p.plan.session_id != id);
        for job in self.jobs.lock().values().filter(|j| j.session_id == id) {
            job.token.cancel();
        }
        let results = self
            .result_sessions
            .lock()
            .iter()
            .filter(|(_, s)| s.as_str() == id)
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>();
        for result in results {
            self.release_result(&result);
        }
    }

    fn close_profile_sessions(&self, id: &str) {
        let ids = self
            .sessions
            .lock()
            .iter()
            .filter(|(_, s)| s.profile.id == id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.disconnect(&id);
        }
    }

    pub fn set_passphrase(&self, passphrase: &str, previous: Option<&str>) -> AppResult<()> {
        let _configuration = self.configuration_gate.lock();
        if self
            .persistence
            .setting("passphrase-configured")?
            .as_deref()
            == Some("true")
        {
            self.verify_passphrase(previous)?;
        }
        if passphrase.chars().count() < 12 || passphrase.len() > 1024 {
            return Err(AppError::invalid(
                "Use a passphrase of 12 to 1,024 characters",
            ));
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(passphrase.as_bytes(), &salt)
            .map_err(|_| AppError::internal("Could not create passphrase verifier"))?
            .to_string();
        self.secrets.set("write-passphrase", &hash)?;
        self.persistence
            .set_setting("passphrase-configured", "true")?;
        self.plans.lock().clear();
        Ok(())
    }

    fn verify_passphrase(&self, passphrase: Option<&str>) -> AppResult<()> {
        let passphrase = passphrase.filter(|p| p.len() <= 1024).ok_or_else(|| {
            AppError::new(
                ErrorCode::PassphraseRequired,
                "Enter your application write passphrase",
            )
        })?;
        let hash=self.secrets.get("write-passphrase")?.ok_or_else(||AppError::new(ErrorCode::PassphraseRequired,"Configure a write passphrase in Settings before changing protected connections or running production writes"))?;
        let hash = PasswordHash::new(&hash)
            .map_err(|_| AppError::internal("The passphrase verifier is invalid"))?;
        Argon2::default()
            .verify_password(passphrase.as_bytes(), &hash)
            .map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidPassphrase,
                    "The write passphrase is incorrect",
                )
            })
    }

    pub fn prepare_execution(&self, session_id: &str, sql: &str) -> AppResult<PreparedPlan> {
        let session = self.session(session_id)?;
        let plan = safety::prepare(session_id, &session.profile, sql)?;
        self.store_plan(plan, Action::Sql, sql.len())
    }

    fn store_plan(
        &self,
        plan: PreparedPlan,
        action: Action,
        bytes: usize,
    ) -> AppResult<PreparedPlan> {
        let mut plans = self.plans.lock();
        plans.retain(|_, p| p.deadline > Instant::now());
        // A newer preview replaces earlier previews for the same session.
        plans.retain(|_, p| p.plan.session_id != plan.session_id);
        if plans.len() >= 32
            || plans
                .values()
                .map(|p| p.bytes)
                .sum::<usize>()
                .saturating_add(bytes)
                > 8 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Prepared changes exceed the 8 MiB review budget",
            ));
        }
        let mut fingerprint = Sha256::new();
        fingerprint.update(
            serde_json::to_vec(&plan)
                .map_err(|_| AppError::internal("Could not fingerprint query plan"))?,
        );
        match &action {
            Action::Browse { table, query } => fingerprint.update(
                serde_json::to_vec(&(table, query))
                    .map_err(|_| AppError::internal("Could not fingerprint table filter"))?,
            ),
            Action::Changes { schema, rows } => fingerprint.update(
                serde_json::to_vec(&(schema, rows))
                    .map_err(|_| AppError::internal("Could not fingerprint bound changes"))?,
            ),
            Action::CancelProcess { id, identity } => fingerprint.update(
                serde_json::to_vec(&(id, identity))
                    .map_err(|_| AppError::internal("Could not fingerprint process target"))?,
            ),
            Action::Sql => {}
        }
        let digest = hex::encode(fingerprint.finalize());
        plans.insert(
            plan.id.clone(),
            StoredPlan {
                plan: plan.clone(),
                action,
                deadline: Instant::now() + Duration::from_secs(60),
                approval: None,
                digest,
                bytes,
            },
        );
        Ok(plan)
    }

    pub fn authorize(
        &self,
        plan_id: &str,
        passphrase: Option<&str>,
        confirmation: Option<&str>,
    ) -> AppResult<String> {
        let plan = {
            self.plans
                .lock()
                .get(plan_id)
                .map(|p| p.plan.clone())
                .ok_or_else(expired)?
        };
        if plan.requires_passphrase {
            self.verify_passphrase(passphrase)?;
        }
        if plan
            .confirmation_text
            .as_deref()
            .is_some_and(|expected| Some(expected) != confirmation)
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Type the exact connection name to confirm this operation",
            ));
        }
        self.session(&plan.session_id)?;
        let mut plans = self.plans.lock();
        let stored = plans.get_mut(plan_id).ok_or_else(expired)?;
        if stored.deadline <= Instant::now() {
            return Err(expired());
        }
        let id = new_id();
        stored.approval = Some(id.clone());
        Ok(id)
    }

    pub fn execute(
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
        events: EventHandler,
    ) -> AppResult<String> {
        let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
        if stored.deadline <= Instant::now() {
            return Err(expired());
        }
        if matches!(stored.action, Action::Browse { .. }) {
            return Err(AppError::invalid(
                "Use browse_table to execute an approved table filter",
            ));
        }
        if stored.plan.requires_approval
            && (stored.approval.is_none() || stored.approval.as_deref() != approval)
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Review and authorize this exact query plan before running it",
            ));
        }
        let session = self.session(&stored.plan.session_id)?;
        let run_id = new_id();
        let token = CancellationToken::new();
        if self.jobs.lock().len() >= 32 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Too many query jobs are already running",
            ));
        }
        self.persistence
            .audit(&session.profile.id, "execute", &stored.digest, "started")?;
        let cancellation_sent = Arc::new(AtomicBool::new(false));
        self.jobs.lock().insert(
            run_id.clone(),
            Job {
                session_id: session.info.id.clone(),
                token: token.clone(),
                cancellation_sent: cancellation_sent.clone(),
            },
        );
        let studio = self.clone();
        let run = run_id.clone();
        tokio::spawn(async move {
            studio
                .run_plan(run, session, stored, token, cancellation_sent, events)
                .await;
        });
        Ok(run_id)
    }

    async fn run_plan(
        self: Arc<Self>,
        run_id: String,
        session: Arc<Session>,
        stored: StoredPlan,
        token: CancellationToken,
        cancellation_sent: Arc<AtomicBool>,
        events: EventHandler,
    ) {
        let started = Instant::now();
        let mut status = RunStatus::Completed;
        let serialized = !matches!(&stored.action, Action::CancelProcess { .. });
        let _lane = if serialized {
            Some(session.lane.lock().await)
        } else {
            None
        };
        if serialized {
            *session.completion.lock() = None;
            let _gate = session.cancellation_gate.lock().await;
            *session.active_run.lock() = Some(run_id.clone());
        }
        if !events(QueryEvent::Started {
            run_id: run_id.clone(),
        }) {
            token.cancel();
        }
        match &stored.action {
            Action::Sql => {
                for (index, statement) in stored.plan.statements.iter().enumerate() {
                    if token.is_cancelled() || session.closed.load(Ordering::Acquire) {
                        status = RunStatus::Cancelled;
                        break;
                    }
                    if !events(QueryEvent::StatementStarted {
                        run_id: run_id.clone(),
                        index,
                    }) {
                        status = RunStatus::Cancelled;
                        break;
                    }
                    let clock = Instant::now();
                    let mut sink = Output {
                        studio: &self,
                        session_id: &session.info.id,
                        run_id: &run_id,
                        index,
                        current: None,
                        truncated: false,
                        viewer_closed: false,
                        events: &events,
                    };
                    let outcome = session
                        .driver
                        .execute(&statement.sql, statement.kind, &mut sink)
                        .await;
                    match outcome {
                        Ok(affected) => {
                            events(QueryEvent::StatementCompleted {
                                run_id: run_id.clone(),
                                index,
                                affected_rows: affected.to_string(),
                                elapsed_ms: clock.elapsed().as_millis() as u32,
                            });
                        }
                        Err(error) => {
                            sink.fail();
                            if matches!(
                                error.code,
                                ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed
                            ) && safety::needs_authorization(statement)
                            {
                                status = RunStatus::OutcomeUnknown;
                                events(QueryEvent::Error {
                                    error: AppError::new(
                                        ErrorCode::OutcomeUnknown,
                                        "The connection was lost during a write. Verify its outcome before retrying.",
                                    ),
                                    statement_index: index,
                                });
                            } else {
                                status = if error.code == ErrorCode::OutcomeUnknown {
                                    RunStatus::OutcomeUnknown
                                } else if error.code == ErrorCode::Cancelled {
                                    RunStatus::Cancelled
                                } else {
                                    RunStatus::Failed
                                };
                                events(QueryEvent::Error {
                                    error,
                                    statement_index: index,
                                });
                            }
                            break;
                        }
                    }
                    if sink.truncated && statement.kind == StatementKind::Read {
                        break;
                    }
                }
            }
            Action::Changes { schema, rows } => {
                if token.is_cancelled() {
                    status = RunStatus::Cancelled;
                } else if let Err(error) = session.driver.apply(schema, rows).await {
                    status = if error.code == ErrorCode::Cancelled {
                        RunStatus::Cancelled
                    } else if error.code == ErrorCode::OutcomeUnknown {
                        RunStatus::OutcomeUnknown
                    } else {
                        RunStatus::Failed
                    };
                    events(QueryEvent::Error {
                        error,
                        statement_index: 0,
                    });
                }
            }
            Action::CancelProcess { id, identity } => {
                if let Err(error) = session.driver.cancel_process(id, identity).await {
                    status = RunStatus::Failed;
                    events(QueryEvent::Error {
                        error,
                        statement_index: 0,
                    });
                }
            }
            Action::Browse { .. } => {}
        }
        if serialized {
            let _gate = session.cancellation_gate.lock().await;
            // PostgreSQL's cancel protocol has no acknowledgement. Discard a session
            // after an explicit cancellation so a late request cannot hit the next run.
            let mut reason = None;
            if cancellation_sent.load(Ordering::Acquire) {
                session.closed.store(true, Ordering::Release);
                session.driver.close();
                reason = Some(if status == RunStatus::Completed {
                    "The operation completed before cancellation took effect. Its reported outcome still applies. Reconnect before running another query."
                } else {
                    "Cancellation was requested. Review the reported statement outcomes before retrying; reconnect before running another query."
                });
            } else if session.driver.is_closed() {
                session.closed.store(true, Ordering::Release);
                reason = Some("This database session closed. Reconnect to run another query.");
            }
            if let Some(reason) = reason {
                self.sessions.lock().remove(&session.info.id);
                self.plans
                    .lock()
                    .retain(|_, p| p.plan.session_id != session.info.id);
                events(QueryEvent::SessionClosed {
                    session_id: session.info.id.clone(),
                    reason: reason.into(),
                });
            }
            *session.active_run.lock() = None;
        }
        let elapsed_ms = started.elapsed().as_millis() as u32;
        let sql = if session.profile.store_query_text && matches!(stored.action, Action::Sql) {
            Some(
                stored
                    .plan
                    .statements
                    .iter()
                    .map(|s| s.sql.as_str())
                    .collect::<Vec<_>>()
                    .join(";\n"),
            )
        } else {
            None
        };
        let history = HistoryEntry {
            id: run_id.clone(),
            connection_id: session.profile.id.clone(),
            connection_name: session.profile.name.clone(),
            started_at: now_ms().saturating_sub(elapsed_ms as u64),
            sql,
            statement_count: stored.plan.statements.len(),
            status,
            elapsed_ms,
        };
        if self.persistence.record_history(&history).is_err()
            || self
                .persistence
                .audit(
                    &session.profile.id,
                    "execute",
                    &stored.digest,
                    &format!("{status:?}"),
                )
                .is_err()
        {
            events(QueryEvent::Notice {
                message: "The execution finished, but its local history could not be recorded."
                    .into(),
            });
        }
        self.jobs.lock().remove(&run_id);
        events(QueryEvent::Finished {
            run_id,
            status,
            elapsed_ms,
        });
    }

    pub async fn cancel(&self, run_id: &str) -> AppResult<()> {
        let (session_id, cancellation_sent) = {
            let jobs = self.jobs.lock();
            let job = jobs.get(run_id).ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "This query has already finished")
            })?;
            job.token.cancel();
            (job.session_id.clone(), job.cancellation_sent.clone())
        };
        let session = self.session(&session_id)?;
        let _gate = session.cancellation_gate.lock().await;
        if session.active_run.lock().as_deref() != Some(run_id) {
            return Ok(());
        }
        cancellation_sent.store(true, Ordering::Release);
        session.driver.cancel().await
    }

    /// Read a bounded context snapshot on the existing editor execution lane.
    pub async fn completion_catalog(&self, session_id: &str) -> AppResult<CompletionCatalog> {
        let session = self.session(session_id)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session is busy; retry after execution finishes",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "The session is busy; retry suggestions",
                )
            })?;
        let previous = session.completion.lock().take();
        let identity = session.driver.completion_identity().await?;
        let relations = session.driver.completion_catalog(&identity).await?;
        let tables = relations.tables;
        let unqualified_tables = relations.unqualified_tables;
        let revision = previous
            .filter(|old| {
                old.identity == identity
                    && old.unqualified_tables.len() == unqualified_tables.len()
                    && old
                        .unqualified_tables
                        .iter()
                        .zip(&unqualified_tables)
                        .all(|(a, b)| a.schema == b.schema && a.name == b.name)
                    && old.tables.len() == tables.len()
                    && old.tables.iter().zip(&tables).all(|(a, b)| {
                        a.table.schema == b.table.schema
                            && a.table.name == b.table.name
                            && a.kind == b.kind
                    })
            })
            .map(|old| old.revision)
            .unwrap_or_else(new_id);
        let snapshot = CompletionCatalog {
            revision,
            identity,
            tables,
            unqualified_tables,
        };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| AppError::internal("Cannot encode completion metadata"))?
            .len()
            > 2 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Completion metadata exceeds its byte budget",
            ));
        }
        if session.closed.load(Ordering::Acquire) {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The session closed",
            ));
        }
        *session.completion.lock() = Some(snapshot.clone());
        Ok(snapshot)
    }
    pub async fn completion_schema(
        &self,
        session_id: &str,
        revision: &str,
        table: &TableRef,
    ) -> AppResult<TableSchema> {
        let session = self.session(session_id)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session is busy; retry after execution finishes",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "The session is busy; retry suggestions",
                )
            })?;
        let snapshot = session
            .completion
            .lock()
            .clone()
            .filter(|s| s.revision == revision)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Conflict,
                    "Completion context changed; refresh suggestions",
                )
            })?;
        if session.driver.completion_identity().await? != snapshot.identity {
            *session.completion.lock() = None;
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Completion context changed; refresh suggestions",
            ));
        }
        if !snapshot
            .tables
            .iter()
            .any(|t| t.table.schema == table.schema && t.table.name == table.name)
        {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "The table is not visible in this completion context",
            ));
        }
        let schema = session.driver.completion_schema(table).await?;
        if schema.columns.len() > 4096
            || serde_json::to_vec(&schema)
                .map_err(|_| AppError::internal("Cannot encode completion columns"))?
                .len()
                > 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Completion columns exceed their budget",
            ));
        }
        if session.closed.load(Ordering::Acquire) {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The session closed",
            ));
        }
        Ok(schema)
    }
    pub async fn catalog(&self, session_id: &str) -> AppResult<Vec<TableInfo>> {
        self.session(session_id)?.driver.catalog().await
    }
    pub async fn schema(&self, session_id: &str, table: &TableRef) -> AppResult<TableSchema> {
        let session = self.session(session_id)?;
        self.schema_for_session(&session, table).await
    }
    pub async fn referencing_foreign_keys(
        &self,
        session_id: &str,
        table: &TableRef,
    ) -> AppResult<Vec<InboundForeignKey>> {
        self.session(session_id)?
            .driver
            .referencing_keys(table)
            .await
    }
    async fn schema_for_session(
        &self,
        session: &Session,
        table: &TableRef,
    ) -> AppResult<TableSchema> {
        let mut schema = session.driver.schema(table).await?;
        if safety::effective_mode(&session.profile) == SafeMode::ReadOnly {
            schema.editable = false;
            for c in &mut schema.columns {
                c.editable = false;
                c.insertable = false;
            }
        }
        Ok(schema)
    }
    pub async fn redis_scan(
        &self,
        session_id: &str,
        pattern: &str,
        cursor: Option<&str>,
    ) -> AppResult<RedisScanPage> {
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        tokio::time::timeout(
            Duration::from_secs(15),
            session.driver.redis_scan(pattern, cursor),
        )
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "The Redis browser timed out. Narrow the pattern and restart the scan.",
            )
        })?
    }
    pub async fn redis_inspect(
        &self,
        session_id: &str,
        argument: &str,
        cursor: Option<&str>,
    ) -> AppResult<RedisValuePreview> {
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        tokio::time::timeout(
            Duration::from_secs(15),
            session.driver.redis_inspect(argument, cursor),
        )
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "The Redis value preview timed out. Retry with a smaller range in the console.",
            )
        })?
    }
    pub async fn health(&self, session_id: &str) -> AppResult<u32> {
        self.session(session_id)?.driver.health().await
    }
    pub async fn processes(&self, session_id: &str) -> AppResult<Vec<ProcessInfo>> {
        self.session(session_id)?.driver.processes().await
    }

    pub async fn browse(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<TablePage> {
        self.browse_with(session_id, table, after, &BrowseQuery::default())
            .await
    }

    pub async fn prepare_browse(
        &self,
        session_id: &str,
        table: &TableRef,
        query: &BrowseQuery,
    ) -> AppResult<Option<PreparedPlan>> {
        let query = query.normalized();
        if !query.has_raw_clauses() {
            return Ok(None);
        }
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        let schema = self.schema_for_session(&session, table).await?;
        let sql = browse_sql_for_engine(session.profile.engine, &schema, &query, None)?;
        let (mut plan, needs_review) = classify_browse_sql(session_id, &session.profile, &sql)?;
        if !needs_review {
            return Ok(None);
        }
        plan.requires_approval = true;
        plan.warnings.push(
            "This table filter or order clause uses an unclassified or side-effecting expression and executes a single bounded page."
                .into(),
        );
        let bytes = sql.len();
        let stored = self.store_plan(
            plan,
            Action::Browse {
                table: table.clone(),
                query,
            },
            bytes,
        )?;
        Ok(Some(stored))
    }

    /// Table page with server-side sort/filter. Typed values stay bound; raw
    /// `WHERE` / `ORDER BY` clauses are classified through `safety::prepare`.
    pub async fn browse_with(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
        query: &BrowseQuery,
    ) -> AppResult<TablePage> {
        self.browse_with_approval(session_id, table, after, query, None)
            .await
    }

    pub async fn browse_with_approval(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
        query: &BrowseQuery,
        approval: Option<&BrowseApproval>,
    ) -> AppResult<TablePage> {
        let query = query.normalized();
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        let schema = self.schema_for_session(&session, table).await?;
        let mut approved_digest: Option<String> = None;
        if query.has_raw_clauses() {
            let sql =
                browse_sql_for_engine(session.profile.engine, &schema, &query, after.as_deref())?;
            let (_, needs_review) = classify_browse_sql(session_id, &session.profile, &sql)?;
            if needs_review {
                if after.is_some() {
                    return Err(AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Approved table clauses execute a single bounded page",
                    ));
                }
                let token = approval.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Review and authorize this table filter or order clause before running it",
                    )
                })?;
                let stored = self
                    .plans
                    .lock()
                    .remove(&token.plan_id)
                    .ok_or_else(expired)?;
                if stored.deadline <= Instant::now() {
                    return Err(expired());
                }
                if stored.plan.session_id != session_id
                    || stored.approval.as_deref() != Some(token.approval.as_str())
                    || !matches!(
                        &stored.action,
                        Action::Browse { table: t, query: q } if t == table && q == &query
                    )
                {
                    return Err(AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Review and authorize this exact table filter or order clause before running it",
                    ));
                }
                self.persistence
                    .audit(&session.profile.id, "browse", &stored.digest, "started")?;
                approved_digest = Some(stored.digest);
            }
        }
        let mut page = match session.driver.browse_query(&schema, &query, after).await {
            Ok(page) => {
                if let Some(digest) = &approved_digest {
                    let _ =
                        self.persistence
                            .audit(&session.profile.id, "browse", digest, "Completed");
                }
                page
            }
            Err(error) => {
                if let Some(digest) = &approved_digest {
                    let _ = self
                        .persistence
                        .audit(&session.profile.id, "browse", digest, "Failed");
                }
                return Err(error);
            }
        };
        if approved_digest.is_some() {
            page.has_more = false;
            page.next_key = None;
        }
        let result = self.results.create(
            &format!("table:{session_id}"),
            0,
            schema.columns.clone(),
            Some(schema),
        )?;
        self.result_sessions
            .lock()
            .insert(result.id.clone(), session_id.into());
        let retained = self.results.append(&result.id, page.rows)?;
        if !retained {
            self.release_result(&result.id);
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Close a result tab to free memory before loading this page",
            ));
        }
        let result = self.results.finish(&result.id, page.truncated)?;
        Ok(TablePage {
            result,
            next_key: page.next_key,
            has_more: page.has_more,
        })
    }

    pub fn prepare_changes(
        &self,
        session_id: &str,
        result_id: &str,
        changes: Vec<CellChange>,
    ) -> AppResult<ChangePreview> {
        self.prepare_changes_batch(
            session_id,
            vec![ResultChanges {
                result_id: result_id.into(),
                changes,
            }],
        )
    }

    pub fn prepare_changes_batch(
        &self,
        session_id: &str,
        inputs: Vec<ResultChanges>,
    ) -> AppResult<ChangePreview> {
        self.prepare_table_changes(
            session_id,
            TableChanges {
                result_id: inputs.first().map(|input| input.result_id.clone()),
                updates: inputs,
                deletes: Vec::new(),
                inserts: Vec::new(),
            },
        )
    }

    fn verify_result_provenance(
        &self,
        session_id: &str,
        result_id: &str,
        expected: &TableSchema,
    ) -> AppResult<()> {
        if self
            .result_sessions
            .lock()
            .get(result_id)
            .map(String::as_str)
            != Some(session_id)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This result belongs to a different session",
            ));
        }
        let source = self.results.provenance(result_id)?;
        if source.table != expected.table
            || serde_json::to_string(&source.columns).ok()
                != serde_json::to_string(&expected.columns).ok()
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "All edits must belong to the same unchanged table schema",
            ));
        }
        Ok(())
    }

    pub fn prepare_table_changes(
        &self,
        session_id: &str,
        changes: TableChanges,
    ) -> AppResult<ChangePreview> {
        let session = self.session(session_id)?;
        let anchor_id = changes
            .result_id
            .as_deref()
            .or_else(|| changes.updates.first().map(|u| u.result_id.as_str()))
            .or_else(|| changes.deletes.first().map(|d| d.result_id.as_str()))
            .ok_or_else(|| AppError::invalid("There are no pending edits"))?;
        if self
            .result_sessions
            .lock()
            .get(anchor_id)
            .map(String::as_str)
            != Some(session_id)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This result belongs to a different session",
            ));
        }
        let schema = self.results.provenance(anchor_id)?;
        let update_cells = changes
            .updates
            .iter()
            .map(|i| i.changes.len())
            .sum::<usize>();
        let delete_rows = changes.deletes.iter().map(|d| d.rows.len()).sum::<usize>();
        let insert_cells = changes
            .inserts
            .iter()
            .map(|r| r.values.len())
            .sum::<usize>();
        let total_cells = update_cells + insert_cells;
        if !schema.editable
            || (update_cells == 0 && delete_rows == 0 && changes.inserts.is_empty())
            || total_cells > 5000
        {
            return Err(AppError::invalid(
                "Select editable cells in a table with a primary key",
            ));
        }

        let mut keyed_mutations: Vec<(TextRow, RowMutation)> = Vec::new();
        let mut bytes = 0usize;

        for input in changes.deletes {
            self.verify_result_provenance(session_id, &input.result_id, &schema)?;
            let mut seen_rows = std::collections::HashSet::new();
            for row_index in input.rows {
                if !seen_rows.insert(row_index) {
                    return Err(AppError::invalid(
                        "A row was marked for deletion twice in one change set",
                    ));
                }
                let original = self.results.row(&input.result_id, row_index)?;
                bytes += original.iter().flatten().map(String::len).sum::<usize>();
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                keyed_mutations.push((original.clone(), RowMutation::Delete { original }));
            }
        }

        for input in changes.updates {
            self.verify_result_provenance(session_id, &input.result_id, &schema)?;
            let mut rows: HashMap<usize, Vec<(usize, Option<String>)>> = HashMap::new();
            for change in input.changes {
                if change.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
                    return Err(AppError::invalid(
                        "An edited value must be smaller than 1 MiB",
                    ));
                }
                let index = schema
                    .columns
                    .iter()
                    .position(|c| c.id == change.column)
                    .ok_or_else(|| AppError::invalid("Unknown edited column"))?;
                let column = &schema.columns[index];
                if !column.editable || column.primary_key {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This column is read-only",
                    ));
                }
                if !column.nullable && change.value.is_none() {
                    return Err(AppError::invalid("This column does not allow NULL"));
                }
                if column.kind == ColumnKind::Json
                    && let Some(value) = &change.value
                {
                    serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
                        AppError::invalid("Enter valid JSON before applying this edit")
                    })?;
                }
                let cells = rows.entry(change.row).or_default();
                if cells.iter().any(|(i, _)| *i == index) {
                    return Err(AppError::invalid(
                        "A cell was included twice in one change set",
                    ));
                }
                cells.push((index, change.value));
            }
            for (index, row_changes) in rows {
                let original = self.results.row(&input.result_id, index)?;
                bytes += original.iter().flatten().map(String::len).sum::<usize>()
                    + row_changes
                        .iter()
                        .filter_map(|(_, v)| v.as_ref())
                        .map(String::len)
                        .sum::<usize>();
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                keyed_mutations.push((
                    original.clone(),
                    RowMutation::Update {
                        original,
                        changes: row_changes,
                    },
                ));
            }
        }

        let key_indices = schema
            .primary_key
            .iter()
            .filter_map(|key| schema.columns.iter().position(|c| &c.name == key))
            .collect::<Vec<_>>();
        keyed_mutations.sort_by(|a, b| {
            key_indices
                .iter()
                .map(|i| &a.0[*i])
                .cmp(key_indices.iter().map(|i| &b.0[*i]))
        });
        for pair in keyed_mutations.windows(2) {
            if key_indices.iter().all(|i| pair[0].0[*i] == pair[1].0[*i]) {
                return Err(AppError::invalid(
                    "A primary key was included twice in this change set",
                ));
            }
        }

        let mut mutations: Vec<RowMutation> = keyed_mutations.into_iter().map(|(_, m)| m).collect();

        for row in changes.inserts {
            let mut cols: Vec<(usize, Option<String>)> = Vec::new();
            for entry in row.values {
                if entry.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
                    return Err(AppError::invalid(
                        "An edited value must be smaller than 1 MiB",
                    ));
                }
                let index = schema
                    .columns
                    .iter()
                    .position(|c| c.id == entry.column)
                    .ok_or_else(|| AppError::invalid("Unknown inserted column"))?;
                let column = &schema.columns[index];
                if !column.insertable {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        format!("Column \"{}\" cannot be set on insert", column.name),
                    ));
                }
                if !column.nullable && entry.value.is_none() {
                    return Err(AppError::invalid(format!(
                        "Column \"{}\" does not allow NULL",
                        column.name
                    )));
                }
                if column.kind == ColumnKind::Json
                    && let Some(value) = &entry.value
                {
                    serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
                        AppError::invalid("Enter valid JSON before applying this insert")
                    })?;
                }
                if cols.iter().any(|(i, _)| *i == index) {
                    return Err(AppError::invalid(
                        "A column was included twice in one inserted row",
                    ));
                }
                bytes += entry.value.as_ref().map_or(0, String::len);
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                cols.push((index, entry.value));
            }
            cols.sort_by_key(|(i, _)| *i);
            for (index, column) in schema.columns.iter().enumerate() {
                if !column.nullable && !column.has_default && !cols.iter().any(|(i, _)| *i == index)
                {
                    return Err(AppError::invalid(format!(
                        "Column \"{}\" requires a value (NOT NULL without a default)",
                        column.name
                    )));
                }
            }
            mutations.push(RowMutation::Insert { values: cols });
        }

        if mutations.len() > 500 {
            return Err(AppError::invalid("Apply at most 500 edited rows at a time"));
        }

        let generated = mutations
            .iter()
            .map(|m| {
                if session.profile.engine == EngineKind::Mysql {
                    crate::drivers::mysql::mutation_sql(&schema, m)
                } else {
                    mutation_sql(&schema, m)
                }
            })
            .collect::<AppResult<Vec<_>>>()?;
        let sql = generated
            .iter()
            .map(|(sql, _)| sql.clone())
            .collect::<Vec<_>>();
        let bindings = generated
            .into_iter()
            .map(|(_, values)| values)
            .collect::<Vec<_>>();
        let statements = sql
            .iter()
            .map(|sql| StatementPlan {
                sql: sql.clone(),
                kind: StatementKind::Write,
                missing_where: false,
            })
            .collect();
        let mut plan = safety::plan_for_statements(session_id, &session.profile, statements)?;
        plan.requires_approval = true;
        plan.warnings.push("Grid edits use the login role in a separate transaction. SQL-editor transactions and session settings are independent.".into());
        bytes += sql.iter().map(String::len).sum::<usize>();
        let row_count = mutations.len();
        let plan = self.store_plan(
            plan,
            Action::Changes {
                schema,
                rows: mutations,
            },
            bytes,
        )?;
        Ok(ChangePreview {
            plan,
            row_count,
            sql,
            bindings,
        })
    }
    pub fn prepare_process_cancel(
        &self,
        session_id: &str,
        id: &str,
        identity: &str,
    ) -> AppResult<PreparedPlan> {
        let session = self.session(session_id)?;
        let mut plan = safety::prepare(
            session_id,
            &session.profile,
            "SELECT pg_catalog.pg_cancel_backend($1)",
        )?;
        plan.requires_approval = true;
        self.store_plan(
            plan,
            Action::CancelProcess {
                id: id.into(),
                identity: identity.into(),
            },
            id.len() + identity.len(),
        )
    }

    pub fn range(&self, result_id: &str, start: usize, count: usize) -> AppResult<Vec<u8>> {
        self.results.range(result_id, start, count)
    }
    pub fn release_result(&self, id: &str) {
        self.result_sessions.lock().remove(id);
        self.results.release(id);
    }
    pub fn history(&self, search: &str) -> AppResult<Vec<HistoryEntry>> {
        self.persistence.history(search)
    }
    pub fn clear_history(&self) -> AppResult<()> {
        self.persistence.clear_history()
    }
    pub fn audit_log(&self) -> AppResult<Vec<AuditEntry>> {
        self.persistence.audit_log()
    }
    pub fn usage(&self) -> ResourceUsage {
        let (retained_bytes, result_count) = self.results.usage();
        ResourceUsage {
            retained_bytes,
            result_count,
            session_count: self.sessions.lock().len(),
            active_runs: self.jobs.lock().len(),
        }
    }
    pub fn shutdown(&self) {
        let ids = self.sessions.lock().keys().cloned().collect::<Vec<_>>();
        for id in ids {
            self.disconnect(&id);
        }
    }
}

fn browse_sql_for_engine(
    engine: EngineKind,
    schema: &TableSchema,
    query: &BrowseQuery,
    after: Option<&[Option<String>]>,
) -> AppResult<String> {
    match engine {
        EngineKind::Postgres => {
            crate::drivers::postgres::browse_sql(schema, query, after).map(|(sql, _)| sql)
        }
        EngineKind::Mysql => {
            crate::drivers::mysql::browse_sql(schema, query, after).map(|(sql, _)| sql)
        }
        EngineKind::Snowflake | EngineKind::Redis => Err(AppError::new(
            ErrorCode::Unsupported,
            "Sorting and filtering table data is unavailable for this engine",
        )),
    }
}

fn classify_browse_sql(
    session_id: &str,
    profile: &ConnectionProfile,
    sql: &str,
) -> AppResult<(PreparedPlan, bool)> {
    let plan = safety::prepare(session_id, profile, sql)?;
    if plan.statements.len() != 1 {
        return Err(AppError::invalid(
            "Table filter and order clauses must form a single SELECT query",
        ));
    }
    let non_read = plan.statements[0].kind != StatementKind::Read;
    if safety::effective_mode(profile) == SafeMode::ReadOnly && non_read {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This connection is read-only. Table filter and order clauses must be pure read expressions.",
        ));
    }
    let needs_review = non_read || plan.requires_approval;
    Ok((plan, needs_review))
}

fn expired() -> AppError {
    AppError::new(
        ErrorCode::ApprovalExpired,
        "The query preview expired or was replaced. Review it again before running.",
    )
}

struct Output<'a> {
    studio: &'a Studio,
    session_id: &'a str,
    run_id: &'a str,
    index: usize,
    current: Option<String>,
    truncated: bool,
    viewer_closed: bool,
    events: &'a EventHandler,
}
impl Output<'_> {
    fn discard(&mut self) {
        self.viewer_closed = true;
        if let Some(id) = self.current.take() {
            self.studio.release_result(&id);
        }
    }
    fn fail(&mut self) {
        if let Some(id) = self.current.take()
            && let Ok(result) = self.studio.results.finish(&id, true)
        {
            (self.events)(QueryEvent::Result { result });
        }
    }
}
impl QuerySink for Output<'_> {
    fn limit(&mut self, reason: &str) {
        self.truncated = true;
        (self.events)(QueryEvent::Notice {
            message: reason.into(),
        });
    }
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        if self.viewer_closed {
            return Ok(());
        }
        let info = match self
            .studio
            .results
            .create(self.run_id, self.index, columns, None)
        {
            Ok(info) => info,
            Err(error) => {
                self.limit(&error.message);
                self.discard();
                return Ok(());
            }
        };
        self.studio
            .result_sessions
            .lock()
            .insert(info.id.clone(), self.session_id.into());
        self.current = Some(info.id.clone());
        self.truncated = false;
        if !(self.events)(QueryEvent::Result { result: info }) {
            self.discard();
        }
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        if self.viewer_closed {
            return Ok(false);
        }
        let id = self
            .current
            .as_deref()
            .ok_or_else(|| AppError::internal("Rows arrived before column metadata"))?;
        let retained = match self.studio.results.append(id, rows) {
            Ok(value) => value,
            Err(error) => {
                self.limit(&error.message);
                self.discard();
                return Ok(false);
            }
        };
        self.truncated |= !retained;
        let info = self.studio.results.info(id)?;
        if !(self.events)(QueryEvent::Progress {
            result_id: id.into(),
            retained_rows: info.retained_rows,
        }) {
            self.discard();
            return Ok(false);
        }
        Ok(retained)
    }
    fn complete(&mut self, _affected: u64) -> AppResult<()> {
        if self.viewer_closed {
            return Ok(());
        }
        if self.current.is_none() {
            self.describe(Vec::new())?;
        }
        if self.viewer_closed {
            return Ok(());
        }
        let id = self.current.take().unwrap();
        let result = self.studio.results.finish(&id, self.truncated)?;
        (self.events)(QueryEvent::Result { result });
        Ok(())
    }
}

#[cfg(test)]
mod browse_classification_tests {
    use super::*;

    fn profile(engine: EngineKind, safe_mode: SafeMode) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "BrowseTest".into(),
            group_id: "local".into(),
            engine,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "joinery".into(),
            username: "joinery".into(),
            environment: Environment::Local,
            safe_mode,
            tls_mode: TlsMode::Disable,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text: false,
            store_drafts: false,
            snowflake: None,
        }
    }

    fn schema(engine: EngineKind) -> TableSchema {
        let (int_type, text_type) = if engine == EngineKind::Mysql {
            ("bigint", "varchar(64)")
        } else {
            ("bigint", "text")
        };
        TableSchema {
            table: TableRef {
                schema: if engine == EngineKind::Mysql {
                    "joinery".into()
                } else {
                    "public".into()
                },
                name: "orders".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "id".into(),
                    database_type: int_type.into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: false,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c1".into(),
                    name: "bucket".into(),
                    database_type: int_type.into(),
                    kind: ColumnKind::Integer,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "label".into(),
                    database_type: text_type.into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: vec![],
            editable: true,
        }
    }

    #[test]
    fn pure_read_raw_clauses_with_typed_filters_need_no_approval_on_pg_and_mysql() {
        for engine in [EngineKind::Postgres, EngineKind::Mysql] {
            let schema = schema(engine);
            let query = BrowseQuery {
                sort: vec![SortKey {
                    column: "bucket".into(),
                    descending: true,
                }],
                filters: vec![
                    ColumnFilter {
                        column: "label".into(),
                        op: FilterOp::Contains,
                        value: Some("ok".into()),
                    },
                    ColumnFilter {
                        column: "bucket".into(),
                        op: FilterOp::Gte,
                        value: Some("1".into()),
                    },
                ],
                where_clause: Some("bucket IS NOT NULL AND LOWER(label) <> 'skip'".into()),
                order_clause: None,
            };
            let sql = browse_sql_for_engine(engine, &schema, &query, None).unwrap();
            for mode in [SafeMode::ReadOnly, SafeMode::Confirm, SafeMode::Silent] {
                let (_, needs_review) =
                    classify_browse_sql("s", &profile(engine, mode), &sql).unwrap();
                assert!(!needs_review, "{engine:?} {mode:?}: {sql}");
            }

            let custom_order = BrowseQuery {
                where_clause: Some("id > 0".into()),
                order_clause: Some("COALESCE(bucket, 0) DESC, id ASC".into()),
                ..Default::default()
            };
            let sql = browse_sql_for_engine(engine, &schema, &custom_order, None).unwrap();
            let (_, needs_review) =
                classify_browse_sql("s", &profile(engine, SafeMode::ReadOnly), &sql).unwrap();
            assert!(!needs_review, "{engine:?} custom order: {sql}");
        }
    }

    #[test]
    fn non_read_raw_clauses_are_blocked_in_read_only_and_reviewed_in_writable_modes() {
        for (engine, expr) in [
            (EngineKind::Postgres, "pg_advisory_lock(1) IS NULL"),
            (EngineKind::Mysql, "GET_LOCK('joinery', 0) = 1"),
        ] {
            let schema = schema(engine);
            let query = BrowseQuery {
                where_clause: Some(expr.into()),
                ..Default::default()
            };
            let sql = browse_sql_for_engine(engine, &schema, &query, None).unwrap();
            assert_eq!(
                classify_browse_sql("s", &profile(engine, SafeMode::ReadOnly), &sql)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
            for mode in [SafeMode::Silent, SafeMode::Confirm, SafeMode::PasswordGated] {
                let (_, needs_review) =
                    classify_browse_sql("s", &profile(engine, mode), &sql).unwrap();
                assert!(needs_review, "{engine:?} {mode:?} must require review");
            }
        }
    }
}
