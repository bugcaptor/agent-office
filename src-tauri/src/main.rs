// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = agent_office_lib::maybe_run_observer_forwarder(std::env::args_os()) {
        std::process::exit(code);
    }
    if let Some(code) = agent_office_lib::maybe_run_sessiond(std::env::args_os()) {
        std::process::exit(code);
    }
    if let Some(code) = agent_office_lib::maybe_run_cli(std::env::args_os()) {
        std::process::exit(code);
    }
    agent_office_lib::run();
}
