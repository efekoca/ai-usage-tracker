//! Wayland gives no app a key of its own, so there the shortcut goes through xdg-desktop-portal's
//! GlobalShortcuts (KDE Plasma 5.27+, GNOME 48+): the desktop owns the key, asks the user once and
//! reports each press. Desktops without it keep the X11 shortcut.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const ID: &str = "toggle-widget";
/// Matches the .desktop file the Linux packages install; the portal cannot tell who an unsandboxed app is otherwise.
const APP_ID: &str = "ai-usage-tracker";
const REGISTRY: &str = "org.freedesktop.host.portal.Registry";
/// Starts the error when the portal found no `ai-usage-tracker.desktop` for this app.
pub const UNREGISTERED: &str = "portal: app not registered";

/// The live session; replacing it closes the old one, which also ends its listener.
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

struct Session {
    conn: Connection,
    handle: OwnedObjectPath,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(p) = Proxy::new(&self.conn, DEST, self.handle.as_ref(), "org.freedesktop.portal.Session") {
            let _ = p.call_method("Close", &());
        }
    }
}

pub fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

/// Whether this desktop's portal offers global shortcuts and lets an unsandboxed app name itself.
/// Older portals (before 1.19) accept the binding but the desktop drops it for want of an app ID.
pub fn available() -> bool {
    let Ok(c) = Connection::session() else { return false };
    let has = |iface: &str| Proxy::new(&c, DEST, PATH, iface).ok().and_then(|p| p.get_property::<u32>("version").ok()).is_some();
    has(IFACE) && has(REGISTRY)
}

/// Whether the desktop holds the shortcut, so it is changed in the desktop's own settings.
pub fn active() -> bool {
    SESSION.lock().unwrap().is_some()
}

pub fn clear() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    SESSION.lock().unwrap().take();
}

/// Binds `hotkey` (`Ctrl+Alt+Shift+W`) in a new session and calls `on_press` for each press.
/// Blocks until the desktop answers, which may wait for the user.
pub fn bind(hotkey: &str, description: &str, on_press: impl Fn() + Send + 'static) -> Result<(), String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    SESSION.lock().unwrap().take();
    let conn = Connection::session().map_err(err)?;
    // once per connection and before any other portal call, so each binding gets a fresh connection
    write_appimage_entry();
    Proxy::new(&conn, DEST, PATH, REGISTRY)
        .map_err(err)?
        .call_method("Register", &(APP_ID, HashMap::<&str, Value>::new()))
        .map_err(|e| format!("{UNREGISTERED}: {e}"))?;
    let portal = Proxy::new(&conn, DEST, PATH, IFACE).map_err(err)?;

    let token = format!("aiut{generation}");
    let opts = HashMap::from([("handle_token", Value::from(token.as_str())), ("session_handle_token", Value::from(token.as_str()))]);
    let created = request(&conn, &token, || portal.call_method("CreateSession", &(opts,)).map(|_| ()))?;
    let handle: String = created.get("session_handle").and_then(|v| String::try_from(v.clone()).ok()).ok_or("portal: no session")?;
    let handle = OwnedObjectPath::try_from(handle).map_err(err)?;

    let bind_token = format!("aiut{generation}b");
    let props = HashMap::from([("description", Value::from(description)), ("preferred_trigger", Value::from(trigger(hotkey)))]);
    let shortcuts = vec![(ID, props)];
    let bind_opts = HashMap::from([("handle_token", Value::from(bind_token.as_str()))]);
    let session_path = ObjectPath::from(&handle);
    request(&conn, &bind_token, || portal.call_method("BindShortcuts", &(&session_path, shortcuts, "", bind_opts)).map(|_| ()))?;

    let activated = portal.receive_signal("Activated").map_err(err)?;
    let mine = handle.clone();
    *SESSION.lock().unwrap() = Some(Session { conn, handle });
    std::thread::Builder::new()
        .name("portal-shortcut".into())
        .spawn(move || {
            for msg in activated {
                if GENERATION.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Ok((session, id, _ts, _opts)) = msg.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
                    && session == mine
                    && id == ID
                {
                    on_press();
                }
            }
        })
        .map_err(err)?;
    Ok(())
}

/// Calls a portal method and waits for its Request's Response, subscribed before the call so it cannot be missed.
fn request(conn: &Connection, token: &str, call: impl FnOnce() -> zbus::Result<()>) -> Result<HashMap<String, OwnedValue>, String> {
    let sender = conn.unique_name().ok_or("portal: no bus name")?.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let req = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request").map_err(err)?;
    let mut responses = req.receive_signal("Response").map_err(err)?;
    call().map_err(err)?;
    let msg = responses.next().ok_or("portal: no answer")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(err)?;
    match code {
        0 => Ok(results),
        1 => Err("hotkey_declined".into()),
        _ => Err("portal: request failed".into()),
    }
}

/// The XDG shortcut format: CTRL, ALT, SHIFT and LOGO, then an XKB key name.
fn trigger(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|k| match k {
            "Ctrl" => "CTRL".to_owned(),
            "Alt" => "ALT".to_owned(),
            "Shift" => "SHIFT".to_owned(),
            "Super" => "LOGO".to_owned(),
            "Space" => "space".to_owned(),
            "PageUp" => "Prior".to_owned(),
            "PageDown" => "Next".to_owned(),
            k if k.len() == 1 => k.to_lowercase(),
            k => k.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The portal names the app by a .desktop file whose program exists. The packages install one; an
/// AppImage gets one in the user's applications folder that points at the AppImage itself.
fn write_appimage_entry() {
    let Some(appimage) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).filter(|p| p.is_file()) else { return };
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")));
    let Some(dir) = data.map(|d| d.join("applications")) else { return };
    let file = dir.join(format!("{APP_ID}.desktop"));
    let entry = appimage_entry(&appimage);
    if std::fs::read_to_string(&file).is_ok_and(|s| s == entry) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, entry)) {
        log::warn!("could not write {}: {e}", file.display());
    }
}

fn appimage_entry(appimage: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=AI Usage Tracker\nComment=Shows the AI Usage Tracker widget\nExec={} --toggle-widget\nTerminal=false\nCategories=Utility;\nNoDisplay=true\n",
        crate::windows::desktop_exec_arg(&appimage.to_string_lossy())
    )
}

fn err(e: impl std::fmt::Display) -> String {
    format!("portal: {e}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_appimage_entry_runs_the_appimage_itself() {
        let e = super::appimage_entry(std::path::Path::new("/home/u/Apps/AI Usage Tracker.AppImage"));
        assert!(e.contains("Exec=\"/home/u/Apps/AI Usage Tracker.AppImage\" --toggle-widget\n"), "{e}");
        assert!(e.contains("NoDisplay=true"));
    }

    #[test]
    fn shortcuts_use_the_xdg_trigger_format() {
        assert_eq!(super::trigger("Ctrl+Alt+Shift+W"), "CTRL+ALT+SHIFT+w");
        assert_eq!(super::trigger("Super+F9"), "LOGO+F9");
        assert_eq!(super::trigger("Ctrl+Space"), "CTRL+space");
    }
}
