//! KDE Plasma 5 has no usable shortcut portal for unsandboxed apps, but its own shortcut service
//! (kglobalaccel) takes registrations over D-Bus the way KDE apps make them, and then the key works
//! from anywhere and shows up in System Settings.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const DEST: &str = "org.kde.kglobalaccel";
const COMPONENT: &str = "ai-usage-tracker";
const ACTION: &str = "toggle-widget";

/// kglobalaccel's setShortcut flags: overwrite the saved keys, and mark the action present (its app runs).
const SET_PRESENT: u32 = 2;
const NO_AUTOLOADING: u32 = 4;

static CONN: Mutex<Option<Connection>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("kde")))
}

pub fn available() -> bool {
    Connection::session().ok().and_then(|c| zbus::blocking::fdo::DBusProxy::new(&c).ok()?.name_has_owner(DEST.try_into().ok()?).ok()).unwrap_or(false)
}

pub fn clear() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Some(conn) = CONN.lock().unwrap().take()
        && let Ok(p) = accel(&conn)
    {
        // keeps the entry in System Settings but frees the keys
        let _ = p.call_method("setShortcut", &(action_id(), Vec::<i32>::new(), SET_PRESENT | NO_AUTOLOADING));
    }
}

/// `chosen` writes the keys; at startup the keys saved in KDE win, so a change made in System Settings is kept.
pub fn bind(hotkey: &str, chosen: bool, on_press: impl Fn() + Send + 'static) -> Result<(), String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let key = qt_key(hotkey).ok_or("hotkey_unknown_key")?;
    let conn = Connection::session().map_err(err)?;
    let p = accel(&conn)?;
    p.call_method("doRegister", &(action_id(),)).map_err(err)?;
    let flags = if chosen { SET_PRESENT | NO_AUTOLOADING } else { SET_PRESENT };
    p.call_method("setShortcut", &(action_id(), vec![key], flags)).map_err(err)?;

    let path: OwnedObjectPath = p.call("getComponent", &(COMPONENT,)).map_err(err)?;
    let component = Proxy::new(&conn, DEST, path, "org.kde.kglobalaccel.Component").map_err(err)?;
    let pressed = component.receive_signal("globalShortcutPressed").map_err(err)?;
    *CONN.lock().unwrap() = Some(conn);
    std::thread::Builder::new()
        .name("kde-shortcut".into())
        .spawn(move || {
            for msg in pressed {
                if GENERATION.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Ok((component, action, _ts)) = msg.body().deserialize::<(String, String, i64)>()
                    && component == COMPONENT
                    && action == ACTION
                {
                    on_press();
                }
            }
        })
        .map_err(err)?;
    Ok(())
}

fn accel(conn: &Connection) -> Result<Proxy<'static>, String> {
    Proxy::new(conn, DEST, "/kglobalaccel", "org.kde.KGlobalAccel").map_err(err)
}

/// Component, action, and their names as System Settings shows them.
fn action_id() -> Vec<&'static str> {
    vec![COMPONENT, ACTION, "AI Usage Tracker", "Show or hide the widget"]
}

/// Qt's key code: modifier bits plus the key, e.g. Ctrl+Alt+Shift+W = 0x0E000057.
fn qt_key(hotkey: &str) -> Option<i32> {
    let mut code = 0i32;
    let mut key = None;
    for part in hotkey.split('+') {
        match part {
            "Ctrl" => code |= 0x0400_0000,
            "Alt" => code |= 0x0800_0000,
            "Shift" => code |= 0x0200_0000,
            "Super" => code |= 0x1000_0000,
            k => key = Some(k),
        }
    }
    let k = key?;
    let base = match k {
        _ if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => k.to_ascii_uppercase().chars().next()? as i32,
        "Space" => 0x20,
        "Left" => 0x0100_0012,
        "Up" => 0x0100_0013,
        "Right" => 0x0100_0014,
        "Down" => 0x0100_0015,
        "Home" => 0x0100_0010,
        "End" => 0x0100_0011,
        "PageUp" => 0x0100_0016,
        "PageDown" => 0x0100_0017,
        "Insert" => 0x0100_0006,
        "Delete" => 0x0100_0007,
        _ => match k.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
            Some(n @ 1..=24) => 0x0100_0030 + n - 1,
            _ => return None,
        },
    };
    Some(code | base)
}

fn err(e: impl std::fmt::Display) -> String {
    format!("kglobalaccel: {e}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn hotkeys_become_qt_key_codes() {
        assert_eq!(super::qt_key("Ctrl+Alt+Shift+W"), Some(0x0E00_0057));
        assert_eq!(super::qt_key("Super+F9"), Some(0x1000_0000 | 0x0100_0038));
        assert_eq!(super::qt_key("Ctrl+Digit1"), None);
        assert_eq!(super::qt_key("Ctrl+1"), Some(0x0400_0031));
    }
}
