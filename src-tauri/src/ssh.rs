use crate::domain::*;
use parking_lot::Mutex;
use russh::{
    client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, task::JoinSet};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[derive(Clone)]
struct ObservedKey {
    line: String,
    fingerprint: String,
}
struct CheckKey {
    expected: Option<String>,
    observed: Arc<Mutex<Option<ObservedKey>>>,
}
impl client::Handler for CheckKey {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        offered: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Pin raw host keys. SSH host certificate CA validation is a separate feature.
        if offered.certificate().is_some() {
            return Ok(false);
        }
        let key = offered.public_key();
        let line = key.to_openssh().map_err(russh::Error::from)?;
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let accepted = self.expected.as_deref() == Some(line.as_str());
        *self.observed.lock() = Some(ObservedKey { line, fingerprint });
        Ok(accepted)
    }
}

struct Challenge {
    host: SshConfig,
    observed: ObservedKey,
    deadline: Instant,
}
#[derive(Default)]
pub struct SshManager {
    challenges: Mutex<HashMap<String, Challenge>>,
    tunnels: Mutex<HashMap<String, Weak<Tunnel>>>,
}

pub struct Tunnel {
    pub port: u16,
    failed: Arc<AtomicBool>,
    stop: CancellationToken,
    task: tokio::task::AbortHandle,
}
impl Tunnel {
    pub fn is_closed(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.task.is_finished()
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

pub fn host_setting(ssh: &SshConfig) -> String {
    format!(
        "ssh-host:{}",
        hex::encode(Sha256::digest(format!(
            "{}\0{}",
            ssh.host.to_lowercase(),
            ssh.port
        )))
    )
}
fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}
fn ssh_error(_: impl std::fmt::Display) -> AppError {
    // Authentication errors can contain remote banners. Keep untrusted text out of logs/UI.
    AppError::new(
        ErrorCode::ConnectionFailed,
        "SSH connection or authentication failed. Check the bastion, credentials, and forwarding permissions.",
    )
}

impl SshManager {
    pub async fn probe(&self, ssh: &SshConfig, known: Option<String>) -> AppResult<SshHostKey> {
        let observed = Arc::new(Mutex::new(None));
        let handler = CheckKey {
            expected: known.clone(),
            observed: observed.clone(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            client::connect(config(), (ssh.host.as_str(), ssh.port), handler),
        )
        .await;
        if let Ok(Ok(handle)) = result {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "Host key verified", "en")
                .await;
        }
        let observed = observed.lock().clone().ok_or_else(|| AppError::new(ErrorCode::ConnectionFailed, "Could not verify the SSH host key. Check the host and port. SSH host certificates are not supported."))?;
        let previous_fingerprint = known
            .as_deref()
            .and_then(|s| keys::PublicKey::from_openssh(s).ok())
            .map(|key| key.fingerprint(HashAlg::Sha256).to_string());
        let challenge_id = if known.is_none() {
            let mut challenges = self.challenges.lock();
            challenges.retain(|_, c| c.deadline > Instant::now());
            if challenges.len() >= 16 {
                return Err(AppError::invalid("Too many pending SSH host checks"));
            }
            let id = new_id();
            challenges.insert(
                id.clone(),
                Challenge {
                    host: ssh.clone(),
                    observed: observed.clone(),
                    deadline: Instant::now() + Duration::from_secs(120),
                },
            );
            Some(id)
        } else {
            None
        };
        Ok(SshHostKey {
            host: ssh.host.clone(),
            port: ssh.port,
            fingerprint: observed.fingerprint,
            previous_fingerprint,
            challenge_id,
        })
    }

    pub fn accept(&self, id: &str, fingerprint: &str) -> AppResult<(SshConfig, String)> {
        let challenge = self.challenges.lock().remove(id).ok_or_else(|| {
            AppError::new(
                ErrorCode::ApprovalExpired,
                "The SSH host check expired. Check the host again.",
            )
        })?;
        if challenge.deadline <= Instant::now() || challenge.observed.fingerprint != fingerprint {
            return Err(AppError::new(
                ErrorCode::ApprovalExpired,
                "The SSH host check expired or changed.",
            ));
        }
        Ok((challenge.host, challenge.observed.line))
    }

    pub async fn acquire(
        &self,
        profile: &ConnectionProfile,
        known: Option<String>,
        password: Option<Zeroizing<String>>,
        key_passphrase: Option<Zeroizing<String>>,
    ) -> AppResult<Arc<Tunnel>> {
        let ssh = profile
            .ssh
            .as_ref()
            .ok_or_else(|| AppError::invalid("SSH is not configured"))?;
        if known.is_none() {
            return Err(AppError::new(
                ErrorCode::SshHostKeyUnknown,
                "Verify and trust the SSH host fingerprint before connecting.",
            ));
        }
        let reuse_key =
            serde_json::to_string(&(profile.id.as_str(), ssh, &profile.host, profile.port))
                .map_err(|_| AppError::internal("Invalid SSH profile"))?;
        if let Some(tunnel) = self
            .tunnels
            .lock()
            .get(&reuse_key)
            .and_then(Weak::upgrade)
            .filter(|t| !t.is_closed())
        {
            return Ok(tunnel);
        }
        let observed = Arc::new(Mutex::new(None));
        let handler = CheckKey {
            expected: known.clone(),
            observed: observed.clone(),
        };
        let mut handle = match tokio::time::timeout(
            Duration::from_secs(10),
            client::connect(config(), (ssh.host.as_str(), ssh.port), handler),
        )
        .await
        {
            Ok(Ok(handle)) => handle,
            _ => {
                if observed
                    .lock()
                    .as_ref()
                    .is_some_and(|key| Some(&key.line) != known.as_ref())
                {
                    return Err(AppError::new(
                        ErrorCode::SshHostKeyChanged,
                        "The SSH host key changed. Connection blocked. Verify the server identity before changing its trusted key.",
                    ));
                }
                return Err(ssh_error("connect"));
            }
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            let authenticated = match ssh.authentication {
                SshAuthentication::Password => handle
                    .authenticate_password(
                        &ssh.username,
                        password.as_deref().map(String::as_str).unwrap_or(""),
                    )
                    .await
                    .map_err(ssh_error)?
                    .success(),
                SshAuthentication::PrivateKey => {
                    let path = ssh
                        .private_key_path
                        .clone()
                        .ok_or_else(|| AppError::invalid("Choose an SSH private key path"))?;
                    let key = tokio::task::spawn_blocking(move || {
                        let bytes = crate::files::read_bounded(&path, 1024 * 1024)?;
                        let text = std::str::from_utf8(&bytes).map_err(ssh_error)?;
                        keys::decode_secret_key(text, key_passphrase.as_deref().map(String::as_str))
                            .map_err(ssh_error)
                    })
                    .await
                    .map_err(ssh_error)??;
                    let hash = handle
                        .best_supported_rsa_hash()
                        .await
                        .map_err(ssh_error)?
                        .flatten();
                    handle
                        .authenticate_publickey(
                            &ssh.username,
                            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                        )
                        .await
                        .map_err(ssh_error)?
                        .success()
                }
                SshAuthentication::Agent => {
                    authenticate_agent(&mut handle, &ssh.username, ssh.agent_socket_path.as_deref())
                        .await?
                }
            };
            if !authenticated {
                return Err(ssh_error("auth"));
            }
            Ok::<_, AppError>(())
        })
        .await
        .map_err(ssh_error)??;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(ssh_error)?;
        let port = listener.local_addr().map_err(ssh_error)?.port();
        let handle = Arc::new(handle);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let target = profile.host.clone();
        let target_port = profile.port;
        let task = tokio::spawn(async move {
            let mut channels = JoinSet::new();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = stopped.cancelled() => break,
                    _ = heartbeat.tick() => { if handle.is_closed() { failure.store(true, Ordering::Release); break; } },
                    _ = channels.join_next(), if !channels.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((mut socket, origin)) = accepted else { failure.store(true, Ordering::Release); break; };
                        if channels.len() >= 8 { continue; }
                        let handle = handle.clone(); let host = target.clone();
                        channels.spawn(async move {
                            if let Ok(Ok(channel)) = tokio::time::timeout(Duration::from_secs(10), handle.channel_open_direct_tcpip(host, target_port as u32, origin.ip().to_string(), origin.port() as u32)).await {
                                let mut stream = channel.into_stream();
                                let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                            }
                        });
                    }
                }
            }
            channels.abort_all();
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "Tunnel closed", "en")
                .await;
        });
        let tunnel = Arc::new(Tunnel {
            port,
            stop,
            failed,
            task: task.abort_handle(),
        });
        let mut tunnels = self.tunnels.lock();
        tunnels.retain(|_, t| t.strong_count() > 0);
        tunnels.insert(reuse_key, Arc::downgrade(&tunnel));
        Ok(tunnel)
    }
}

async fn authenticate_agent(
    handle: &mut client::Handle<CheckKey>,
    user: &str,
    socket: Option<&str>,
) -> AppResult<bool> {
    #[cfg(unix)]
    let mut agent = match socket {
        Some(path) => keys::agent::client::AgentClient::connect_uds(path).await,
        None => keys::agent::client::AgentClient::connect_env().await,
    }
    .map_err(ssh_error)?;
    #[cfg(windows)]
    let mut agent = keys::agent::client::AgentClient::connect_named_pipe(
        socket.unwrap_or(r"\\.\pipe\openssh-ssh-agent"),
    )
    .await
    .map_err(ssh_error)?;
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(ssh_error)?
        .flatten();
    for identity in agent
        .request_identities()
        .await
        .map_err(ssh_error)?
        .into_iter()
        .take(16)
    {
        let result = match identity {
            keys::agent::AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(user, key, hash, &mut agent)
                    .await
            }
            keys::agent::AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user, certificate, hash, &mut agent)
                    .await
            }
        }
        .map_err(ssh_error)?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}
