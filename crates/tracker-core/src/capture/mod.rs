//! Opt-in live capture. Everything here is off by default, changes outside this app's own
//! folder are recorded in [`claude_settings::CaptureState`] and can be reverted in one step.

pub mod claude_settings;
pub mod claude_usage;
pub mod codex_limits;
pub mod otlp;
pub mod statusline;
