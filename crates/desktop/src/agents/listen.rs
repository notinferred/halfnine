//! The app's side of `halfnine mcp`: it listens on the endpoint, admits a bridge that knows this launch's token, and
//! serves each one its own MCP session.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{Agents, Server};
use crate::mcp::endpoint::{self, Endpoint, Listener};

/// Listens until the app quits. If it can't, agents can't reach Halfnine, and the log says why.
pub async fn listen(agents: Arc<Agents>, dir: PathBuf) {
    let (mut listener, address) = match Listener::bind(&dir) {
        Ok(bound) => bound,
        Err(e) => {
            log::warn!("Agents can't reach Halfnine: {e}");
            return;
        }
    };
    let token = endpoint::new_token();
    let endpoint = Endpoint {
        address,
        token: token.clone(),
        pid: std::process::id(),
    };
    if let Err(e) = endpoint::write(&dir, &endpoint) {
        log::warn!("Agents can't reach Halfnine: {}", e.message);
        return;
    }
    loop {
        match listener.accept().await {
            Ok(mut stream) => {
                let agents = agents.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    if endpoint::admit(&mut stream, &token).await {
                        serve(agents, stream).await;
                    }
                });
            }
            Err(e) => {
                log::warn!("An agent couldn't connect: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// One agent's MCP session, from its first message until it hangs up.
pub async fn serve<S>(agents: Arc<Agents>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let session = format!("m-{}", uuid::Uuid::new_v4().simple());
    agents.connected(&session);
    if let Ok(running) = Server::new(agents.clone(), session.clone()).serve(stream).await {
        let _ = running.waiting().await;
    }
    agents.disconnected(&session);
}
