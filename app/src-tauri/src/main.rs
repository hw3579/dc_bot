// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let headless = std::env::args().skip(1).any(|argument| argument == "--headless");

    if headless {
        ib_options_relay_lib::run_headless();
    } else {
        ib_options_relay_lib::run();
    }
}
