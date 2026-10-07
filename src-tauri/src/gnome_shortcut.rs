//! GNOME before 48 has no shortcut portal, and on Wayland an X11 key grab only fires while the app
//! is focused. GNOME's own custom shortcuts run a command on a key from anywhere, so the widget
//! shortcut becomes one of those, running `ai-usage-tracker --toggle-widget`. Only our own entry
//! is touched; the user's other custom shortcuts stay as they are.

use std::process::Command;

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const LIST: &str = "custom-keybindings";
const PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/ai-usage-tracker/";

pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("gnome")))
}

/// Points our entry at this copy of the app. `keys` also rewrites the binding: done when the user
/// changes the shortcut in the app, not at startup, so a change made in GNOME Settings is kept.
pub fn apply(hotkey: &str, keys: bool) -> Result<(), String> {
    if hotkey.trim().is_empty() {
        return remove();
    }
    let mut list = parse_list(&gsettings(&["get", SCHEMA, LIST])?);
    let ours = format!("{SCHEMA}.custom-keybinding:{PATH}");
    let fresh = !list.iter().any(|p| p == PATH);
    gsettings(&["set", &ours, "name", &quote("AI Usage Tracker widget")])?;
    gsettings(&["set", &ours, "command", &quote(&command(&crate::app_path()?))])?;
    if keys || fresh {
        gsettings(&["set", &ours, "binding", &quote(&binding(hotkey)?)])?;
    }
    if fresh {
        list.push(PATH.to_owned());
        gsettings(&["set", SCHEMA, LIST, &format_list(&list)])?;
    }
    Ok(())
}

pub fn remove() -> Result<(), String> {
    let list = parse_list(&gsettings(&["get", SCHEMA, LIST])?);
    if list.iter().any(|p| p == PATH) {
        let kept: Vec<String> = list.into_iter().filter(|p| p != PATH).collect();
        gsettings(&["set", SCHEMA, LIST, &format_list(&kept)])?;
    }
    gsettings(&["reset-recursively", &format!("{SCHEMA}.custom-keybinding:{PATH}")]).map(|_| ())
}

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = crate::desktop_env(&mut Command::new("gsettings")).args(args).output().map_err(|e| format!("gsettings: {e}"))?;
    if !out.status.success() {
        return Err(format!("gsettings: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// A GVariant string literal.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `['/a/', '/b/']` or `@as []`; the entries are D-Bus-style paths, so they hold no quotes.
fn parse_list(text: &str) -> Vec<String> {
    text.split('\'').skip(1).step_by(2).map(str::to_owned).collect()
}

fn format_list(paths: &[String]) -> String {
    if paths.is_empty() {
        return "@as []".to_owned();
    }
    format!("[{}]", paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "))
}

/// GNOME runs the command through shell-style word splitting, so the path is single-quoted.
fn command(exe: &std::path::Path) -> String {
    format!("'{}' {}", exe.to_string_lossy().replace('\'', r"'\''"), crate::TOGGLE_WIDGET)
}

/// `Ctrl+Alt+Shift+W` becomes `<Control><Alt><Shift>w`, GTK's accelerator format.
fn binding(hotkey: &str) -> Result<String, String> {
    hotkey
        .split('+')
        .map(|k| match k {
            "Ctrl" => Ok("<Control>".to_owned()),
            "Alt" => Ok("<Alt>".to_owned()),
            "Shift" => Ok("<Shift>".to_owned()),
            "Super" => Ok("<Super>".to_owned()),
            k => crate::hotkey::xkb_key(k).ok_or_else(|| "hotkey_unknown_key".to_owned()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_list_round_trips_and_keeps_other_entries() {
        let mine = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
        assert_eq!(parse_list("@as []"), Vec::<String>::new());
        let list = parse_list(&format!("['{mine}', '{PATH}']"));
        assert_eq!(list, [mine, PATH]);
        assert_eq!(format_list(&list), format!("['{mine}', '{PATH}']"));
        assert_eq!(format_list(&[]), "@as []");
    }

    #[test]
    fn keys_and_command_use_gnome_formats() {
        assert_eq!(binding("Ctrl+Alt+Shift+W").unwrap(), "<Control><Alt><Shift>w");
        assert_eq!(binding("Super+F9").unwrap(), "<Super>F9");
        assert_eq!(binding("Ctrl+Alt+Minus").unwrap(), "<Control><Alt>minus");
        assert_eq!(binding("Ctrl+Numpad1").unwrap(), "<Control>KP_1");
        assert!(binding("Ctrl+Banana").is_err());
        assert_eq!(command(std::path::Path::new("/opt/AI Usage/o'neil/ai-usage-tracker")), r"'/opt/AI Usage/o'\''neil/ai-usage-tracker' --toggle-widget");
        assert_eq!(quote(r"a'b\c"), r"'a\'b\\c'");
    }
}
