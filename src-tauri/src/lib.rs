// Lib.rs - Main library entry point for Time Locker
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod error;
pub mod crypto;
pub mod archive;
pub mod keyfile;
pub mod tlock_format;
pub mod commands;
pub mod progress;
pub mod cli;

/// Run the Tauri GUI application
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::OperationState::default())
        .invoke_handler(tauri::generate_handler![
            commands::lock_item,
            commands::lock_item_with_progress,
            commands::unlock_item,
            commands::unlock_item_with_progress,
            commands::cancel_operation,
            commands::get_locked_items,
            commands::scan_for_keys,
            commands::get_settings,
            commands::save_settings,
            commands::get_app_state,
            // Migration commands: .key.md + .7z -> .7z.tlock
            commands::migrate_to_tlock,
            commands::read_tlock_metadata,
            commands::is_tlock_file,
            commands::is_legacy_key_file,
            commands::unlock_tlock_file,
            commands::open_in_explorer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
