// A release build on Windows opens no console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod backups;
mod bindings;
mod commands;
mod connections;
mod docker;
mod export;
mod files;
mod import;
mod mcp;
mod queries;
mod secrets;
mod sessions;
mod settings;
mod setup;
mod store;

use std::sync::Arc;

use tauri::Manager;

pub struct AppState {
    pub store: Arc<store::Store>,
    pub queries: queries::Queries,
    pub sessions: Arc<sessions::Sessions>,
    pub imports: import::Imports,
    pub passwords: Arc<secrets::Passwords>,
    pub directory: Arc<connections::Directory>,
    pub settings: Arc<settings::Settings>,
    pub docker: Arc<docker::Docker>,
    pub agents: Arc<agents::Agents>,
    pub backups: Arc<backups::Backups>,
}

fn main() {
    // An agent starts `halfnine mcp`, which only relays to the app, so it never starts Tauri. The single-instance
    // plugin would otherwise take it for a second launch.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "mcp") {
        std::process::exit(mcp::bridge::run());
    }

    let commands = bindings::builder();
    #[cfg(debug_assertions)]
    bindings::export(&commands);

    tauri::Builder::default()
        // A second launch focuses the window that is already open, so two copies never write the same files.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands.invoke_handler())
        .setup(move |app| {
            secrets::init()?;
            commands.mount_events(app);
            let dir = app.path().app_config_dir()?;
            let store = Arc::new(store::Store::open(dir.clone())?);
            let sessions = Arc::new(sessions::Sessions::default());
            let passwords = Arc::new(secrets::Passwords::default());
            let desk = Arc::new(agents::desk::WindowDesk(app.handle().clone()));
            let settings = Arc::new(settings::Settings::open(&dir)?);
            let handle = app.handle().clone();
            let docker = Arc::new(docker::Docker::new(&dir, settings.clone(), move || {
                let _ = tauri_specta::Event::emit(&docker::commands::DockerChanged, &handle);
            }));
            docker.watch();
            let backups = Arc::new(backups::Backups::open(&dir)?);
            let directory = Arc::new(connections::Directory::new(store.clone(), passwords.clone(), docker.clone()));
            let agents = Arc::new(agents::Agents::new(&dir, directory.clone(), settings.clone(), sessions.clone(), desk));
            tauri::async_runtime::spawn(agents::listen::listen(agents.clone(), dir.clone()));
            app.manage(AppState {
                store,
                queries: queries::Queries::open(dir)?,
                sessions,
                imports: import::Imports::default(),
                passwords,
                directory,
                settings,
                docker,
                agents,
                backups,
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Halfnine could not start")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event
                && let Ok(dir) = app.path().app_config_dir()
            {
                mcp::endpoint::remove(&dir);
            }
        });
}
