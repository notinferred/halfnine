// A release build on Windows opens no console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bindings;
mod commands;
mod export;
mod files;
mod import;
mod queries;
mod secrets;
mod sessions;
mod store;

use tauri::Manager;

pub struct AppState {
    pub store: store::Store,
    pub queries: queries::Queries,
    pub sessions: sessions::Sessions,
    pub imports: import::Imports,
    pub passwords: secrets::Passwords,
}

fn main() {
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
        .setup(|app| {
            secrets::init()?;
            let dir = app.path().app_config_dir()?;
            app.manage(AppState {
                store: store::Store::open(dir.clone())?,
                queries: queries::Queries::open(dir)?,
                sessions: sessions::Sessions::default(),
                imports: import::Imports::default(),
                passwords: secrets::Passwords::default(),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TableUltra could not start");
}
