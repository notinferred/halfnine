//! Finding the Docker engine on this computer. A macOS app opened from the Dock doesn't see the shell's environment,
//! so it follows the Docker CLI's own order: `DOCKER_HOST`, then the current context, then the places Docker Desktop,
//! OrbStack, Colima and Rancher Desktop put their socket. Only engines on this computer are used.

use std::path::{Path, PathBuf};

/// Where an engine listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    Pipe(String),
}

impl Endpoint {
    /// The address as bollard takes it.
    pub fn address(&self) -> String {
        match self {
            Endpoint::Unix(path) => path.to_string_lossy().into_owned(),
            Endpoint::Pipe(pipe) => pipe.clone(),
        }
    }

    /// Reads a Docker host address. A TCP or SSH engine is elsewhere, and isn't used.
    pub fn parse(host: &str) -> Option<Self> {
        if let Some(path) = host.strip_prefix("unix://") {
            Some(Endpoint::Unix(PathBuf::from(path)))
        } else {
            host.strip_prefix("npipe://").map(|pipe| Endpoint::Pipe(pipe.replace('/', "\\")))
        }
    }
}

/// An endpoint, and what pointed at it, to show people which engine Halfnine uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub endpoint: Endpoint,
    pub by: String,
}

/// What discovery may look at: the environment and the files around the home folder. Passed in so tests can fake them.
pub struct Surroundings<'a> {
    pub var: &'a dyn Fn(&str) -> Option<String>,
    pub home: &'a Path,
    pub read: &'a dyn Fn(&Path) -> Option<String>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    pub list: &'a dyn Fn(&Path) -> Vec<PathBuf>,
}

/// The endpoints worth trying on this computer, from its real environment and files.
pub fn here() -> Vec<Found> {
    let home = dirs::home_dir().unwrap_or_default();
    let list = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).map_or_else(|_| Vec::new(), |entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
    };
    candidates(&Surroundings {
        var: &|name| std::env::var(name).ok().filter(|v| !v.is_empty()),
        home: &home,
        read: &|path| std::fs::read_to_string(path).ok(),
        exists: &|path| path.exists(),
        list: &list,
    })
}

/// Every engine endpoint worth trying, most likely first, without repeats.
pub fn candidates(around: &Surroundings) -> Vec<Found> {
    let mut found = Vec::new();
    let mut add = |endpoint: Option<Endpoint>, by: &str| {
        if let Some(endpoint) = endpoint
            && !found.iter().any(|f: &Found| f.endpoint == endpoint)
        {
            found.push(Found {
                endpoint,
                by: by.to_string(),
            });
        }
    };
    if let Some(host) = (around.var)("DOCKER_HOST") {
        add(Endpoint::parse(&host), "DOCKER_HOST");
    }
    let config = (around.var)("DOCKER_CONFIG").map_or_else(|| around.home.join(".docker"), PathBuf::from);
    let context = (around.var)("DOCKER_CONTEXT").or_else(|| current_context(&config, around));
    if let Some(name) = context.filter(|name| name != "default") {
        add(context_host(&config, &name, around), &format!("the {name} context"));
    }
    if cfg!(windows) {
        add(Some(Endpoint::Pipe(r"\\.\pipe\docker_engine".into())), "Docker Desktop");
        add(Some(Endpoint::Pipe(r"\\.\pipe\dockerDesktopLinuxEngine".into())), "Docker Desktop");
        return found;
    }
    let colima = (around.var)("COLIMA_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let config = (around.var)("XDG_CONFIG_HOME").map_or_else(|| around.home.join(".config"), PathBuf::from);
        let xdg = config.join("colima");
        if (around.exists)(&xdg) { xdg } else { around.home.join(".colima") }
    });
    let probes = [
        (around.home.join(".docker/run/docker.sock"), "Docker Desktop"),
        (PathBuf::from("/var/run/docker.sock"), "Docker"),
        (around.home.join(".orbstack/run/docker.sock"), "OrbStack"),
        (colima.join("default/docker.sock"), "Colima"),
        (around.home.join(".colima/docker.sock"), "Colima"),
        (around.home.join(".rd/docker.sock"), "Rancher Desktop"),
    ];
    for (path, by) in probes {
        if (around.exists)(&path) {
            add(Some(Endpoint::Unix(path)), by);
        }
    }
    found
}

fn current_context(config: &Path, around: &Surroundings) -> Option<String> {
    let text = (around.read)(&config.join("config.json"))?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("currentContext")?.as_str().map(str::to_string)
}

/// A context's engine, from the `meta.json` whose `Name` matches. Go reads these keys in any case, so this does too.
fn context_host(config: &Path, name: &str, around: &Surroundings) -> Option<Endpoint> {
    let get = |value: &serde_json::Value, key: &str| -> Option<serde_json::Value> {
        value
            .as_object()?
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    (around.list)(&config.join("contexts/meta")).into_iter().find_map(|dir| {
        let meta: serde_json::Value = serde_json::from_str(&(around.read)(&dir.join("meta.json"))?).ok()?;
        if get(&meta, "Name")?.as_str()? != name {
            return None;
        }
        let host = get(&get(&get(&meta, "Endpoints")?, "docker")?, "Host")?;
        Endpoint::parse(host.as_str()?)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn found(vars: &[(&str, &str)], files: &[(&str, &str)], sockets: &[&str]) -> Vec<Found> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let files: HashMap<PathBuf, String> = files.iter().map(|(k, v)| (PathBuf::from(k), v.to_string())).collect();
        let sockets: Vec<PathBuf> = sockets.iter().map(PathBuf::from).collect();
        let around = Surroundings {
            var: &|name| vars.get(name).cloned(),
            home: Path::new("/Users/dan"),
            read: &|path| files.get(path).cloned(),
            exists: &|path| sockets.iter().any(|s| s == path),
            list: &|dir| {
                files
                    .keys()
                    .filter(|f| f.starts_with(dir))
                    .filter_map(|f| f.parent().map(Path::to_path_buf))
                    .collect()
            },
        };
        candidates(&around)
    }

    #[test]
    fn follows_the_current_context_before_probing() {
        if cfg!(windows) {
            return;
        }
        let meta = r#"{"Name":"desktop-linux","Endpoints":{"docker":{"Host":"unix:///Users/dan/.docker/run/docker.sock"}}}"#;
        let other = r#"{"Name":"colima","Endpoints":{"docker":{"Host":"unix:///Users/dan/.colima/default/docker.sock"}}}"#;
        let all = found(
            &[],
            &[
                ("/Users/dan/.docker/config.json", r#"{"currentContext":"desktop-linux"}"#),
                ("/Users/dan/.docker/contexts/meta/fe9c/meta.json", meta),
                ("/Users/dan/.docker/contexts/meta/ab12/meta.json", other),
            ],
            &["/Users/dan/.docker/run/docker.sock", "/Users/dan/.orbstack/run/docker.sock"],
        );
        assert_eq!(
            all.iter().map(|f| (f.endpoint.address(), f.by.as_str())).collect::<Vec<_>>(),
            [
                ("/Users/dan/.docker/run/docker.sock".to_string(), "the desktop-linux context"),
                ("/Users/dan/.orbstack/run/docker.sock".to_string(), "OrbStack"),
            ]
        );
    }

    #[test]
    fn takes_docker_host_first_and_skips_a_remote_engine() {
        if cfg!(windows) {
            return;
        }
        let local = found(&[("DOCKER_HOST", "unix:///tmp/d.sock")], &[], &[]);
        assert_eq!(local[0].endpoint, Endpoint::Unix(PathBuf::from("/tmp/d.sock")));
        assert!(found(&[("DOCKER_HOST", "tcp://10.0.0.5:2375")], &[], &[]).is_empty());
        assert!(found(&[("DOCKER_HOST", "ssh://dan@box")], &[], &[]).is_empty());
    }

    #[test]
    fn finds_colima_where_its_home_says() {
        if cfg!(windows) {
            return;
        }
        let at_home = found(&[("COLIMA_HOME", "/opt/colima")], &[], &["/opt/colima/default/docker.sock"]);
        assert_eq!(at_home[0].by, "Colima");
        let xdg = found(&[], &[], &["/Users/dan/.config/colima", "/Users/dan/.config/colima/default/docker.sock"]);
        assert_eq!(xdg[0].endpoint.address(), "/Users/dan/.config/colima/default/docker.sock");
    }

    #[test]
    fn reads_named_pipes() {
        assert_eq!(
            Endpoint::parse("npipe:////./pipe/docker_engine"),
            Some(Endpoint::Pipe(r"\\.\pipe\docker_engine".into()))
        );
    }
}
