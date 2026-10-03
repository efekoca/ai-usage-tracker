#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  if let Some(code) = ai_usage_tracker_lib::cli_mode() {
    std::process::exit(code);
  }
  ai_usage_tracker_lib::run();
}
