// A release build on Windows opens no console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bindings;
mod commands;
mod secrets;
mod sessions;
mod store;

use tauri::Manager;

pub struct AppState {
    pub store: store::Store,
    pub sessions: sessions::Sessions,
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
        .invoke_handler(commands.invoke_handler())
        .setup(|app| {
            secrets::init()?;
            let store = store::Store::open(app.path().app_config_dir()?)?;
            app.manage(AppState {
                store,
                sessions: sessions::Sessions::default(),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TableUltra could not start");
}
