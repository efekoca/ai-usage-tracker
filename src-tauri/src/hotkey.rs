use crate::state::AppState;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn parse(hotkey: &str) -> Result<Shortcut, String> {
    let s: Shortcut = hotkey.trim().parse().map_err(|e| format!("{e}"))?;
    if s.mods.is_empty() {
        // a bare key would be taken away from every other program
        return Err("hotkey_needs_modifier".into());
    }
    Ok(s)
}

/// The error is kept for the settings screen: another program may already own the combination.
/// `chosen` is true when the user picked this shortcut just now rather than at startup.
pub fn apply(app: &AppHandle, hotkey: &str, chosen: bool) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    #[cfg(not(target_os = "linux"))]
    let _ = chosen;
    #[cfg(target_os = "linux")]
    {
        crate::portal_shortcut::clear();
        crate::kde_shortcut::clear();
        let wayland = crate::portal_shortcut::wayland_session();
        if !hotkey.trim().is_empty() && wayland && crate::portal_shortcut::available() {
            parse(hotkey)?;
            // a GNOME custom shortcut left from before the portal would toggle the widget a second time
            if crate::gnome_shortcut::is_gnome() {
                let _ = crate::gnome_shortcut::remove();
            }
            *app.state::<AppState>().hotkey_error.lock().unwrap() = None;
            let (app, hotkey) = (app.clone(), hotkey.to_owned());
            // the desktop may ask the user first, so the answer is awaited off the caller's thread
            std::thread::spawn(move || {
                let press = app.clone();
                let r = crate::portal_shortcut::bind(&hotkey, "Show or hide the AI Usage Tracker widget", move || {
                    crate::windows::handle_menu(&press, "tray:widget");
                });
                if let Err(e) = &r {
                    log::warn!("widget shortcut not registered through the portal: {e}");
                }
                *app.state::<AppState>().hotkey_error.lock().unwrap() = r.err();
            });
            return Ok(());
        }
        if wayland && !hotkey.trim().is_empty() && crate::kde_shortcut::is_kde() && crate::kde_shortcut::available() {
            parse(hotkey)?;
            let press = app.clone();
            match crate::kde_shortcut::bind(hotkey, chosen, move || crate::windows::handle_menu(&press, "tray:widget")) {
                Ok(()) => {
                    *app.state::<AppState>().hotkey_error.lock().unwrap() = None;
                    return Ok(());
                }
                Err(e) => log::warn!("KDE shortcut not registered, using an X11 one: {e}"),
            }
        }
        if wayland && crate::gnome_shortcut::is_gnome() {
            if !hotkey.trim().is_empty() {
                parse(hotkey)?;
            }
            match crate::gnome_shortcut::apply(hotkey, chosen) {
                Ok(()) => {
                    *app.state::<AppState>().hotkey_error.lock().unwrap() = None;
                    return Ok(());
                }
                Err(e) => log::warn!("GNOME custom shortcut not set, using an X11 one: {e}"),
            }
        }
    }
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
