//! SSH tunnels: a local port that forwards each connection through SSH to the database, like `ssh -L`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tu_core::{Error, ErrorKind, SshAuth, SshSpec};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// An open tunnel. Dropping it stops forwarding and closes the SSH session.
pub(crate) struct Tunnel {
    pub port: u16,
    task: JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn open(ssh: &SshSpec, target_host: &str, target_port: u16, secret: Option<&str>) -> Result<Tunnel, Error> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let problem = Arc::new(Mutex::new(None));
    let checker = HostCheck {
        host: ssh.host.clone(),
        port: ssh.port,
        problem: problem.clone(),
    };
    let connecting = client::connect(config, (ssh.host.as_str(), ssh.port), checker);
    let mut session = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Err(_) => {
            return Err(Error::new(
                ErrorKind::Timeout,
                format!("The SSH server {} didn't answer in time.", ssh.host),
            ));
        }
        Ok(Err(e)) => {
            let reason = problem.lock().ok().and_then(|p| p.clone());
            return Err(Error::new(
                ErrorKind::Ssh,
                reason.unwrap_or_else(|| format!("Couldn't open SSH to {}: {e}.", ssh.host)),
            ));
        }
        Ok(Ok(session)) => session,
    };
    if !authenticate(&mut session, ssh, secret).await? {
        return Err(Error::new(
            ErrorKind::Ssh,
            format!("The SSH server didn't accept the credentials for {}.", ssh.user),
        ));
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| ssh_error("Couldn't open a local port for the tunnel", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| ssh_error("Couldn't open a local port for the tunnel", e))?
        .port();
    let session = Arc::new(session);
    let target_host = target_host.to_string();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, peer)) = listener.accept().await {
            let session = session.clone();
            let host = target_host.clone();
            tokio::spawn(async move {
                match session
                    .channel_open_direct_tcpip(host, u32::from(target_port), peer.ip().to_string(), u32::from(peer.port()))
                    .await
                {
                    Ok(channel) => {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                    }
                    Err(e) => log::warn!("the SSH tunnel couldn't open a channel: {e}"),
                }
            });
        }
    });
    Ok(Tunnel { port, task })
}

/// Trusts a server only when its key is already in `~/.ssh/known_hosts`, as `ssh` does with strict checking.
struct HostCheck {
    host: String,
    port: u16,
    problem: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            self.refuse("The SSH server offered a certificate; only host keys in ~/.ssh/known_hosts are supported.");
            return Ok(false);
        };
        match russh::keys::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                self.refuse(&format!(
                    "{} isn't in ~/.ssh/known_hosts yet. Connect once with ssh to check its key and accept it.",
                    self.host
                ));
                Ok(false)
            }
            Err(e) => {
                self.refuse(&format!(
                    "{}'s SSH key doesn't match ~/.ssh/known_hosts ({e}). Don't connect until you know why.",
                    self.host
                ));
                Ok(false)
            }
        }
    }
}

impl HostCheck {
    fn refuse(&self, reason: &str) {
        if let Ok(mut problem) = self.problem.lock() {
            *problem = Some(reason.to_string());
        }
    }
}

async fn authenticate(session: &mut Handle<HostCheck>, ssh: &SshSpec, secret: Option<&str>) -> Result<bool, Error> {
    let failed = |e: russh::Error| Error::new(ErrorKind::Ssh, format!("SSH sign-in failed: {e}."));
    match &ssh.auth {
        SshAuth::Password => Ok(session
            .authenticate_password(&ssh.user, secret.unwrap_or_default())
            .await
            .map_err(failed)?
            .success()),
        SshAuth::Key { path } => {
            let key = russh::keys::load_secret_key(expand_home(path), secret)
                .map_err(|e| Error::new(ErrorKind::Ssh, format!("Couldn't read the SSH key {path}: {e}.")))?;
            let hash = session.best_supported_rsa_hash().await.map_err(failed)?.flatten();
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            Ok(session.authenticate_publickey(&ssh.user, key).await.map_err(failed)?.success())
        }
        SshAuth::Agent => {
            let no_agent = |e: russh::keys::Error| Error::new(ErrorKind::Ssh, format!("Couldn't reach the SSH agent: {e}."));
            #[cfg(unix)]
            let mut agent = AgentClient::connect_env().await.map_err(no_agent)?;
            #[cfg(windows)]
            let mut agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await.map_err(no_agent)?;
            with_agent(session, &ssh.user, &mut agent).await
        }
    }
}

/// Offers each key the agent holds until the server accepts one.
async fn with_agent<S>(session: &mut Handle<HostCheck>, user: &str, agent: &mut AgentClient<S>) -> Result<bool, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| Error::new(ErrorKind::Ssh, format!("The SSH agent didn't list its keys: {e}.")))?;
    let hash = session.best_supported_rsa_hash().await.ok().flatten().flatten();
    for identity in identities {
        if let AgentIdentity::PublicKey { key, .. } = identity
            && let Ok(result) = session.authenticate_publickey_with(user, key, hash, agent).await
            && result.success()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn expand_home(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

fn ssh_error(what: &str, e: std::io::Error) -> Error {
    Error::new(ErrorKind::Ssh, format!("{what}: {e}."))
}
