//! `halfnine mcp`: what an AI agent starts to reach Halfnine. It holds no logic of its own. It finds the running app,
//! opening it if it is closed, proves it is this person's own process, then copies bytes between the agent's stdio and
//! the app, where the MCP server, the access levels, the approvals and the activity log all live. Standard output
//! carries only the protocol, so everything else goes to standard error.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;

use super::endpoint;

/// How long the bridge waits for an app it opened to start listening.
const LAUNCH_WAIT: Duration = Duration::from_secs(20);

pub fn run() -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => return fail(&e.to_string()),
    };
    match runtime.block_on(relay()) {
        Ok(()) => 0,
        Err(message) => fail(&message),
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("halfnine mcp: {message}");
    1
}

async fn relay() -> Result<(), String> {
    let dir = endpoint::config_dir().ok_or("there is no config folder for this account")?;
    let stream = match attach(&dir).await {
        Ok(stream) => stream,
        Err(_) => {
            launch()?;
            wait_for_app(&dir).await?
        }
    };
    let (mut from_app, mut to_app) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        // The agent closed its end: the app hears it as the end of the stream.
        _ = tokio::io::copy(&mut stdin, &mut to_app) => {
            let _ = to_app.shutdown().await;
        }
        // The app closed this session, or quit.
        _ = tokio::io::copy(&mut from_app, &mut stdout) => {
            let _ = stdout.flush().await;
        }
    }
    // Leaving at once rather than waiting on the thread that reads standard input, which may never return.
    std::process::exit(0)
}

async fn attach(dir: &Path) -> Result<endpoint::ClientStream, String> {
    let endpoint = endpoint::read(dir).map_err(|e| e.message)?;
    if endpoint.address.is_empty() {
        return Err("Halfnine isn't running".to_string());
    }
    let mut stream = endpoint::connect(&endpoint.address).await.map_err(|e| e.to_string())?;
    endpoint::introduce(&mut stream, &endpoint.token).await?;
    Ok(stream)
}

async fn wait_for_app(dir: &Path) -> Result<endpoint::ClientStream, String> {
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        match attach(dir).await {
            Ok(stream) => return Ok(stream),
            Err(e) if started.elapsed() > LAUNCH_WAIT => return Err(format!("Halfnine was opened but didn't start listening: {e}")),
            Err(_) => {}
        }
    }
}

/// Opens the app in the background, apart from the agent, so it outlives the agent and doesn't hold its pipes.
fn launch() -> Result<(), String> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("couldn't find Halfnine itself: {e}"))?;
    let mut command = match app_bundle(&exe) {
        Some(bundle) => {
            let mut open = Command::new("/usr/bin/open");
            open.arg("-g").arg("-a").arg(bundle);
            open
        }
        None => detached(&exe),
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Halfnine isn't running and couldn't be opened: {e}"))
}

/// The `.app` a macOS binary sits in, three levels up: `Halfnine.app/Contents/MacOS/halfnine`.
fn app_bundle(exe: &Path) -> Option<&Path> {
    let bundle = exe.parent()?.parent()?.parent()?;
    (cfg!(target_os = "macos") && bundle.extension().is_some_and(|e| e == "app")).then_some(bundle)
}

#[cfg(unix)]
fn detached(exe: &Path) -> Command {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(exe);
    command.process_group(0);
    command
}

#[cfg(windows)]
fn detached(exe: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut command = Command::new(exe);
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_a_mac_binary_sits_in() {
        let exe = Path::new("/Applications/Halfnine.app/Contents/MacOS/halfnine");
        let expected = cfg!(target_os = "macos").then_some(Path::new("/Applications/Halfnine.app"));
        assert_eq!(app_bundle(exe), expected);
        assert_eq!(app_bundle(Path::new("/repo/target/debug/halfnine")), None);
    }
}
