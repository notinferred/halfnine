//! "Connect an agent": adds Halfnine to an AI client in one step. Each plan is shown to the person before anything
//! runs. Halfnine changes only its own entry in another app's settings, and keeps a copy of the file first; where the
//! client has a command or an install screen of its own, that is used instead of editing its files.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use hn_core::{Error, ErrorKind};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;

const NAME: &str = "halfnine";
/// The Claude Code permission rule that allows every Halfnine tool, so Claude Code doesn't ask as well as Halfnine.
const ALLOW_RULE: &str = "mcp__halfnine";
const SHELL_WAIT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Client {
    ClaudeCode,
    Codex,
    Cursor,
    VsCode,
    ClaudeDesktop,
}

const CLIENTS: [Client; 5] = [Client::ClaudeCode, Client::Codex, Client::Cursor, Client::VsCode, Client::ClaudeDesktop];

impl Client {
    fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::Codex => "Codex",
            Client::Cursor => "Cursor",
            Client::VsCode => "VS Code",
            Client::ClaudeDesktop => "Claude Desktop",
        }
    }
}

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientSetup {
    pub client: Client,
    pub name: String,
    /// Already set up with this copy of Halfnine, as far as its settings show. `None` when they can't tell.
    pub connected: Option<bool>,
}

/// What connecting a client will do, shown before it does it.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetupPlan {
    pub client: Client,
    pub steps: Vec<String>,
    /// The same setup as a command or settings to paste, for doing it by hand.
    pub manual: String,
    /// Why Halfnine can't do it itself, when it can't, in which case `manual` is the way.
    pub blocked: Option<String>,
}

/// The server command a client starts: this copy of Halfnine, with `mcp`.
fn server() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| Error::new(ErrorKind::Storage, format!("Halfnine couldn't find itself: {e}.")))?;
    check_location(&exe)?;
    Ok(exe)
}

/// A copy run from the disk image, or one macOS moved aside because it was opened in place, disappears later.
fn check_location(exe: &Path) -> Result<(), Error> {
    let text = exe.to_string_lossy();
    if text.starts_with("/Volumes/") || text.contains("/AppTranslocation/") {
        return Err(Error::new(ErrorKind::Invalid, "Move Halfnine to Applications, then connect."));
    }
    Ok(())
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        format!("\"{text}\"")
    } else if text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

fn claude_add(exe: &Path) -> String {
    format!("claude mcp add --scope user --transport stdio {NAME} -- {} mcp", quoted(exe))
}

fn codex_add(exe: &Path) -> String {
    format!("codex mcp add {NAME} -- {} mcp", quoted(exe))
}

fn server_entry(exe: &Path) -> Value {
    json!({ "command": exe.to_string_lossy(), "args": ["mcp"] })
}

pub fn cursor_link(exe: &Path) -> String {
    let config = base64::engine::general_purpose::STANDARD.encode(server_entry(exe).to_string());
    format!(
        "cursor://anysphere.cursor-deeplink/mcp/install?name={NAME}&config={}",
        utf8_percent_encode(&config, NON_ALPHANUMERIC)
    )
}

pub fn vscode_link(exe: &Path) -> String {
    let mut entry = server_entry(exe);
    entry["name"] = json!(NAME);
    format!("vscode:mcp/install?{}", utf8_percent_encode(&entry.to_string(), NON_ALPHANUMERIC))
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

fn claude_settings() -> Option<PathBuf> {
    home().map(|h| h.join(".claude").join("settings.json"))
}

fn claude_desktop_config() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Claude").join("claude_desktop_config.json"))
}

/// Adds the rule that allows Halfnine's tools to Claude Code's settings, once. Everything else is kept as it was.
pub fn allow_halfnine(settings: Option<&str>) -> Result<String, String> {
    let mut root = parse_object(settings)?;
    let permissions = root.entry("permissions").or_insert_with(|| json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Err("its \"permissions\" isn't an object".to_string());
    };
    let allow = permissions.entry("allow").or_insert_with(|| json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return Err("its \"permissions.allow\" isn't a list".to_string());
    };
    if !allow.iter().any(|rule| rule == ALLOW_RULE) {
        allow.push(json!(ALLOW_RULE));
    }
    pretty(&root)
}

/// Sets Halfnine's own entry under `mcpServers`, as Claude Desktop reads it. Every other server is kept as it was.
pub fn add_server(config: Option<&str>, exe: &Path) -> Result<String, String> {
    let mut root = parse_object(config)?;
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err("its \"mcpServers\" isn't an object".to_string());
    };
    servers.insert(NAME.to_string(), server_entry(exe));
    pretty(&root)
}

fn parse_object(text: Option<&str>) -> Result<Map<String, Value>, String> {
    match text.map(str::trim) {
        None | Some("") => Ok(Map::new()),
        Some(text) => match serde_json::from_str(text) {
            Ok(Value::Object(root)) => Ok(root),
            Ok(_) => Err("it isn't a JSON object".to_string()),
            Err(_) => Err("it isn't plain JSON, and Halfnine won't rewrite a file it can't read whole".to_string()),
        },
    }
}

fn pretty(root: &Map<String, Value>) -> Result<String, String> {
    serde_json::to_string_pretty(root).map(|t| t + "\n").map_err(|e| e.to_string())
}

/// Rewrites another app's settings file through `edit`, keeping the original beside it the first time.
fn edit_file(path: &Path, edit: impl FnOnce(Option<&str>) -> Result<String, String>) -> Result<(), Error> {
    let shown = path.display();
    let before = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::new(ErrorKind::Storage, format!("Couldn't read {shown}: {e}."))),
    };
    let after = edit(before.as_deref()).map_err(|why| Error::new(ErrorKind::Invalid, format!("Halfnine left {shown} alone: {why}.")))?;
    if before.as_deref() == Some(after.as_str()) {
        return Ok(());
    }
    let failed = |e: std::io::Error| Error::new(ErrorKind::Storage, format!("Couldn't write {shown}: {e}."));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(failed)?;
    }
    let backup = path.with_extension("json.halfnine-backup");
    if before.is_some() && !backup.exists() {
        std::fs::copy(path, &backup).map_err(failed)?;
    }
    let temporary = path.with_extension("json.halfnine-tmp");
    std::fs::write(&temporary, after).map_err(failed)?;
    std::fs::rename(&temporary, path).map_err(failed)
}

/// Runs a client's own command the way the person's terminal would: through their login shell, which knows where
/// they installed it. Gives up after a while rather than hang on a shell that waits for input.
fn in_shell(command: &str) -> Result<String, String> {
    let mut child = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut cmd = Command::new(shell);
        cmd.arg("-l").arg("-i").arg("-c").arg(command);
        cmd
    }
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > SHELL_WAIT => {
                let _ = child.kill();
                return Err("it took too long".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        let error = text(&output.stderr);
        Err(error.lines().last().unwrap_or("it failed").to_string())
    }
}

fn found_in_shell(program: &str) -> bool {
    let probe = if cfg!(windows) {
        format!("where {program}")
    } else {
        format!("command -v {program}")
    };
    in_shell(&probe).is_ok_and(|out| !out.is_empty())
}

fn read_json(path: Option<PathBuf>) -> Option<Value> {
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// Whether a settings file's `mcpServers.halfnine` starts this copy of Halfnine.
fn points_here(config: Option<Value>, exe: &Path) -> bool {
    config
        .and_then(|c| c.get("mcpServers")?.get(NAME)?.get("command")?.as_str().map(str::to_string))
        .is_some_and(|command| Path::new(&command) == exe)
}

fn connected(client: Client, exe: &Path) -> Option<bool> {
    match client {
        Client::ClaudeCode => Some(points_here(read_json(home().map(|h| h.join(".claude.json"))), exe)),
        Client::Cursor => Some(points_here(read_json(home().map(|h| h.join(".cursor").join("mcp.json"))), exe)),
        Client::ClaudeDesktop => Some(points_here(read_json(claude_desktop_config()), exe)),
        Client::Codex => {
            let config = home().and_then(|h| std::fs::read_to_string(h.join(".codex").join("config.toml")).ok())?;
            Some(config.contains("[mcp_servers.halfnine]") && config.contains(exe.to_string_lossy().as_ref()))
        }
        Client::VsCode => None,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn client_setups() -> Result<Vec<ClientSetup>, Error> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).unwrap_or_default();
    Ok(CLIENTS
        .iter()
        .map(|&client| ClientSetup {
            client,
            name: client.name().to_string(),
            connected: connected(client, &exe),
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn plan_client(client: Client) -> Result<SetupPlan, Error> {
    tauri::async_runtime::spawn_blocking(move || plan(client))
        .await
        .map_err(|e| Error::new(ErrorKind::Storage, e.to_string()))?
}

fn plan(client: Client) -> Result<SetupPlan, Error> {
    let exe = server()?;
    let shown = |path: Option<PathBuf>| path.map_or_else(|| "its settings".to_string(), |p| p.display().to_string());
    let (steps, manual, needs) = match client {
        Client::ClaudeCode => (
            vec![
                format!("Run {} through your login shell, as your terminal would.", claude_add(&exe)),
                format!(
                    "Allow Halfnine's tools in {}, so Claude Code leaves the asking to Halfnine. The file is copied first.",
                    shown(claude_settings())
                ),
            ],
            claude_add(&exe),
            Some("claude"),
        ),
        Client::Codex => (
            vec![format!("Run {} through your login shell, as your terminal would.", codex_add(&exe))],
            codex_add(&exe),
            Some("codex"),
        ),
        Client::Cursor => (
            vec!["Open Cursor's own screen for adding Halfnine. You confirm it there.".to_string()],
            server_entry(&exe).to_string(),
            None,
        ),
        Client::VsCode => (
            vec!["Open VS Code's own screen for adding Halfnine. You confirm it there.".to_string()],
            server_entry(&exe).to_string(),
            None,
        ),
        Client::ClaudeDesktop => (
            vec![
                format!("Add Halfnine to {}. The file is copied first.", shown(claude_desktop_config())),
                "Then quit Claude Desktop and open it again.".to_string(),
            ],
            serde_json::to_string_pretty(&json!({ "mcpServers": { NAME: server_entry(&exe) } })).unwrap_or_default(),
            None,
        ),
    };
    let blocked = needs
        .filter(|program| !found_in_shell(program))
        .map(|program| format!("Halfnine couldn't find the {program} command. Run this in your terminal instead:"));
    Ok(SetupPlan {
        client,
        steps,
        manual,
        blocked,
    })
}

/// Does what `plan_client` showed. Returns what to do next, if anything.
#[tauri::command]
#[specta::specta]
pub async fn connect_client(client: Client) -> Result<String, Error> {
    tauri::async_runtime::spawn_blocking(move || connect(client))
        .await
        .map_err(|e| Error::new(ErrorKind::Storage, e.to_string()))?
}

fn connect(client: Client) -> Result<String, Error> {
    let exe = server()?;
    let failed = |what: &str, why: String| Error::new(ErrorKind::Invalid, format!("{what} failed: {why}"));
    match client {
        Client::ClaudeCode => {
            // Replaces an earlier entry, such as one for another copy of Halfnine.
            let _ = in_shell(&format!("claude mcp remove --scope user {NAME}"));
            in_shell(&claude_add(&exe)).map_err(|why| failed("claude mcp add", why))?;
            let settings = claude_settings().ok_or_else(|| Error::new(ErrorKind::Storage, "There is no home folder."))?;
            edit_file(&settings, allow_halfnine)?;
            Ok("Connected. Start a new Claude Code session, or run /mcp in one that is open.".to_string())
        }
        Client::Codex => {
            let _ = in_shell(&format!("codex mcp remove {NAME}"));
            in_shell(&codex_add(&exe)).map_err(|why| failed("codex mcp add", why))?;
            Ok("Connected. Start a new Codex session to use it.".to_string())
        }
        Client::Cursor => open::that(cursor_link(&exe))
            .map(|()| "Cursor opened its install screen. Confirm Halfnine there.".to_string())
            .map_err(|e| failed("Opening Cursor", e.to_string())),
        Client::VsCode => open::that(vscode_link(&exe))
            .map(|()| "VS Code opened its install screen. Confirm Halfnine there.".to_string())
            .map_err(|e| failed("Opening VS Code", e.to_string())),
        Client::ClaudeDesktop => {
            let config = claude_desktop_config().ok_or_else(|| Error::new(ErrorKind::Storage, "There is no settings folder."))?;
            edit_file(&config, |text| add_server(text, &exe))?;
            Ok("Added. Quit Claude Desktop and open it again to use it.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_halfnine_once_and_keeps_everything_else_in_order() {
        let before = r#"{"model": "opus", "permissions": {"deny": ["Bash(rm:*)"], "allow": ["Read"]}, "z": 1}"#;
        let after = allow_halfnine(Some(before)).unwrap();
        let value: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(value["permissions"]["allow"], json!(["Read", "mcp__halfnine"]));
        assert_eq!(value["permissions"]["deny"], json!(["Bash(rm:*)"]));
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "permissions", "z"]);
        assert_eq!(allow_halfnine(Some(&after)).unwrap(), after);
        assert!(allow_halfnine(None).unwrap().contains("mcp__halfnine"));
        assert!(allow_halfnine(Some("{ // a comment\n }")).is_err());
        assert!(allow_halfnine(Some(r#"{"permissions": {"allow": "all"}}"#)).is_err());
    }

    #[test]
    fn adds_only_its_own_server() {
        let before = r#"{"mcpServers": {"other": {"command": "x"}}, "theme": "dark"}"#;
        let after = add_server(Some(before), Path::new("/Applications/Halfnine.app/Contents/MacOS/halfnine")).unwrap();
        let value: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(value["mcpServers"]["other"], json!({"command": "x"}));
        assert_eq!(value["mcpServers"]["halfnine"]["args"], json!(["mcp"]));
        assert_eq!(value["theme"], "dark");
        assert!(points_here(Some(value), Path::new("/Applications/Halfnine.app/Contents/MacOS/halfnine")));
    }

    #[test]
    fn edits_a_file_keeping_the_original_beside_it() {
        let dir = std::env::temp_dir().join(format!("hn-setup-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"a": 1}"#).unwrap();
        edit_file(&path, allow_halfnine).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.halfnine-backup")).unwrap(), r#"{"a": 1}"#);
        assert!(std::fs::read_to_string(&path).unwrap().contains("mcp__halfnine"));
        std::fs::write(&path, "not json").unwrap();
        assert!(edit_file(&path, allow_halfnine).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn links_to_each_editors_own_install_screen() {
        let exe = Path::new("/Applications/Halfnine.app/Contents/MacOS/halfnine");
        let cursor = cursor_link(exe);
        let encoded = cursor.split("config=").nth(1).unwrap();
        let decoded = percent_encoding::percent_decode_str(encoded).decode_utf8().unwrap();
        let config = base64::engine::general_purpose::STANDARD.decode(decoded.as_bytes()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&config).unwrap(),
            json!({"command": "/Applications/Halfnine.app/Contents/MacOS/halfnine", "args": ["mcp"]})
        );
        assert!(cursor.starts_with("cursor://anysphere.cursor-deeplink/mcp/install?name=halfnine&config="));
        let vscode = vscode_link(exe);
        let query = percent_encoding::percent_decode_str(vscode.trim_start_matches("vscode:mcp/install?"))
            .decode_utf8()
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&query).unwrap()["name"], "halfnine");
    }

    #[test]
    fn refuses_a_copy_that_will_disappear() {
        assert!(check_location(Path::new("/Volumes/Halfnine/Halfnine.app/Contents/MacOS/halfnine")).is_err());
        assert!(
            check_location(Path::new(
                "/private/var/folders/x/AppTranslocation/y/d/Halfnine.app/Contents/MacOS/halfnine"
            ))
            .is_err()
        );
        assert!(check_location(Path::new("/Applications/Halfnine.app/Contents/MacOS/halfnine")).is_ok());
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(quoted(Path::new("/a/b")), if cfg!(windows) { "\"/a/b\"" } else { "/a/b" });
        if !cfg!(windows) {
            assert_eq!(
                quoted(Path::new("/Users/o'neil/My Apps/halfnine")),
                r"'/Users/o'\''neil/My Apps/halfnine'"
            );
        }
    }
}
