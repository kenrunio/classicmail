// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // started elevated to set the computer's mail client (src/mapi.rs): do that and exit
    if let Some(code) = classicmail_lib::mapi::elevated_command() {
        std::process::exit(code);
    }
    classicmail_lib::run()
}
