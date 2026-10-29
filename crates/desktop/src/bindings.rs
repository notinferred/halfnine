//! The typed bridge to the UI: which commands exist, and the TypeScript that calls them.

use tauri_specta::{Builder, ErrorHandlingMode, collect_commands};

use crate::commands;

#[cfg(any(debug_assertions, test))]
const OUTPUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/desktop/src/ipc/bindings.ts");

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .error_handling(ErrorHandlingMode::Throw)
        .commands(collect_commands![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::parse_connection_url,
            commands::open_session,
            commands::switch_database,
            commands::close_session,
            commands::schema,
            commands::structure,
            commands::page,
            commands::count_rows,
            commands::run_statements,
            commands::cancel_run,
            commands::explain,
            commands::export,
            commands::activity,
            commands::stop_session,
            commands::review_changes,
            commands::commit_changes,
            commands::choices,
            commands::choose_import,
            commands::import_csv,
            commands::list_history,
            commands::list_saved_queries,
            commands::save_query,
            commands::delete_saved_query,
        ])
}

/// Writes `apps/desktop/src/ipc/bindings.ts`. A debug build does it on start and the tests do it in CI, where a
/// changed file fails the check. A release build never writes it, so it doesn't have this at all.
#[cfg(any(debug_assertions, test))]
pub fn export(builder: &Builder<tauri::Wry>) {
    builder
        .export(
            specta_typescript::Typescript::default()
                .header("// Generated from crates/desktop/src/commands.rs by crates/desktop/src/bindings.rs. Don't edit.\n"),
            OUTPUT,
        )
        .expect("the TypeScript bindings could not be written");
}

#[cfg(test)]
mod tests {
    #[test]
    fn exports_the_bindings() {
        super::export(&super::builder());
    }
}
