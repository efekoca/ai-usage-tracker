//! The widget's system-wide shortcut (shows or hides it from any program).

use crate::state::AppState;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Checks a shortcut's spelling (`Ctrl+Alt+Shift+W`, `Super+F9`, …) without registering it.
pub fn parse(hotkey: &str) -> Result<Shortcut, String> {
    let s: Shortcut = hotkey.trim().parse().map_err(|e| format!("{e}"))?;
    if s.mods.is_empty() {
        // a bare key would be taken away from every other program
        return Err("hotkey_needs_modifier".into());
    }
    Ok(s)
}

/// Replaces the registered shortcut with `hotkey` (empty = none). The outcome is kept for the
/// settings screen: another program may already own the combination.
pub fn apply(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let result = if hotkey.trim().is_empty() {
        Ok(())
    } else {
        parse(hotkey).and_then(|s| {
            gs.on_shortcut(s, |app, _, e| {
                if e.state == ShortcutState::Pressed {
                    crate::windows::handle_menu(app, "tray:widget");
                }
            })
            .map_err(|e| e.to_string())
        })
    };
    if let Err(e) = &result {
        log::warn!("widget shortcut not registered: {e}");
    }
    *app.state::<AppState>().hotkey_error.lock().unwrap() = result.as_ref().err().cloned();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_need_a_modifier_and_a_known_key() {
        assert!(parse("Ctrl+Alt+Shift+W").is_ok());
        assert!(parse("Super+Shift+F9").is_ok());
        assert!(parse("Ctrl+Alt+Digit1").is_ok());
        assert!(parse("W").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+Alt+NoSuchKey").is_err());
    }
}
