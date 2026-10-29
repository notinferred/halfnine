//! Where the running app listens for `halfnine mcp`, and how the two recognise each other. On each launch the app
//! writes the address and a fresh token to `mcp.json` in its config folder, which only the person's own account can
//! read; a bridge must send that token before anything else.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hn_core::Error;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::files;

/// The app's identifier, as `tauri.conf.json` names it, which names its config folder too.
pub const IDENTIFIER: &str = "com.notinferred.halfnine";
const FILE: &str = "mcp.json";
const VERSION: u32 = 1;
const ACCEPTED: &[u8] = b"ok\n";

#[derive(Serialize, Deserialize, Default)]
pub struct Endpoint {
    pub address: String,
    pub token: String,
    pub pid: u32,
}

/// The app's config folder, worked out the way Tauri does, for the bridge, which doesn't start Tauri.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(IDENTIFIER))
}

pub fn read(dir: &Path) -> Result<Endpoint, Error> {
    files::load(&dir.join(FILE), VERSION, "where Halfnine listens")
}

pub fn write(dir: &Path, endpoint: &Endpoint) -> Result<(), Error> {
    files::save_private(&dir.join(FILE), VERSION, endpoint, "where Halfnine listens")
}

pub fn remove(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

/// A token nobody can guess: two random UUIDs, 244 random bits.
pub fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The bridge's side: the token on one line, then the app's answer on one line.
pub async fn introduce<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, token: &str) -> Result<(), String> {
    stream.write_all(format!("{token}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let answer = line(stream).await.map_err(|e| e.to_string())?;
    if answer.as_bytes() == &ACCEPTED[..ACCEPTED.len() - 1] {
        Ok(())
    } else {
        Err(answer)
    }
}

/// The app's side: reads the first line a byte at a time, so nothing after it is read early, and answers it.
pub async fn admit<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, token: &str) -> bool {
    let sent = match tokio::time::timeout(Duration::from_secs(5), line(stream)).await {
        Ok(Ok(sent)) => sent,
        _ => return false,
    };
    if sent == token {
        stream.write_all(ACCEPTED).await.is_ok()
    } else {
        let _ = stream.write_all(b"Halfnine refused the connection: the token didn't match.\n").await;
        false
    }
}

async fn line<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        if byte == b'\n' {
            return String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
        }
        if bytes.len() == 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the first line is too long"));
        }
        bytes.push(byte);
    }
}

#[cfg(unix)]
pub use unix::{ClientStream, Listener, connect};
#[cfg(windows)]
pub use windows::{ClientStream, Listener, connect};

#[cfg(unix)]
mod unix {
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tokio::net::{UnixListener, UnixStream};

    pub type ServerStream = UnixStream;
    pub type ClientStream = UnixStream;

    pub struct Listener(UnixListener);

    impl Listener {
        /// Listens on `mcp.sock` in the config folder, readable and writable by the person's own account only.
        pub fn bind(dir: &Path) -> io::Result<(Self, String)> {
            std::fs::create_dir_all(dir)?;
            let path = dir.join("mcp.sock");
            if path.exists() {
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(io::Error::new(io::ErrorKind::AddrInUse, "another Halfnine is listening"));
                }
                // Left behind by a Halfnine that didn't quit cleanly.
                std::fs::remove_file(&path)?;
            }
            let listener = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok((Self(listener), path.to_string_lossy().into_owned()))
        }

        pub async fn accept(&mut self) -> io::Result<ServerStream> {
            self.0.accept().await.map(|(stream, _)| stream)
        }
    }

    pub async fn connect(address: &str) -> io::Result<ClientStream> {
        UnixStream::connect(address).await
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

    use super::IDENTIFIER;

    pub type ServerStream = NamedPipeServer;
    pub type ClientStream = NamedPipeClient;

    /// A named pipe with a random name, which no other account can claim first, and which refuses other machines.
    pub struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    impl Listener {
        pub fn bind(_dir: &Path) -> io::Result<(Self, String)> {
            let name = format!(r"\\.\pipe\{IDENTIFIER}-mcp-{}", uuid::Uuid::new_v4().simple());
            let next = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
            Ok((Self { name: name.clone(), next }, name))
        }

        pub async fn accept(&mut self) -> io::Result<ServerStream> {
            self.next.connect().await?;
            let following = ServerOptions::new().reject_remote_clients(true).create(&self.name)?;
            Ok(std::mem::replace(&mut self.next, following))
        }
    }

    /// Waits briefly while every instance of the pipe is busy, as Windows asks clients to.
    pub async fn connect(address: &str) -> io::Result<ClientStream> {
        const PIPE_BUSY: i32 = 231;
        for _ in 0..20 {
            match ClientOptions::new().open(address) {
                Err(e) if e.raw_os_error() == Some(PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
                other => return other,
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "Halfnine's pipe stayed busy"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_folder_tauri_uses() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], IDENTIFIER);
    }

    #[tokio::test]
    async fn admits_only_the_right_token_and_reads_nothing_past_it() {
        let (mut bridge, mut app) = tokio::io::duplex(1024);
        let (introduced, admitted) = tokio::join!(
            async {
                let result = introduce(&mut bridge, "right").await;
                bridge.write_all(b"{\"jsonrpc\"").await.unwrap();
                result
            },
            admit(&mut app, "right")
        );
        assert!(introduced.is_ok() && admitted);
        let mut rest = [0u8; 10];
        app.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"{\"jsonrpc\"");

        let (mut bridge, mut app) = tokio::io::duplex(1024);
        let (introduced, admitted) = tokio::join!(introduce(&mut bridge, "wrong"), admit(&mut app, "right"));
        assert!(!admitted);
        assert!(introduced.unwrap_err().contains("didn't match"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn listens_on_a_socket_only_its_owner_can_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hn-endpoint-{}", uuid::Uuid::new_v4().simple()));
        let (mut listener, address) = Listener::bind(&dir).unwrap();
        assert_eq!(std::fs::metadata(&address).unwrap().permissions().mode() & 0o777, 0o600);
        let (client, server) = tokio::join!(connect(&address), listener.accept());
        assert!(client.is_ok() && server.is_ok());
        assert!(Listener::bind(&dir).is_err(), "a second listener is refused while the first listens");
        drop(listener);
        let _ = std::fs::remove_dir_all(dir);
    }
}
