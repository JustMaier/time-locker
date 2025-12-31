// Main.rs - Application entry point
// Supports both CLI and GUI modes based on arguments

// Hide console window in release builds on Windows (only for GUI mode)
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::process::ExitCode;

fn main() -> ExitCode {
    // Check if CLI arguments were provided
    if time_locker_lib::cli::has_cli_args() {
        // Run in CLI mode
        time_locker_lib::cli::run()
    } else {
        // Run in GUI mode
        time_locker_lib::run();
        ExitCode::SUCCESS
    }
}
