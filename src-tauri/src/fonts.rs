/// Names starting with "@" are vertical-writing variants.
#[cfg(windows)]
pub fn installed_families(_app: &tauri::AppHandle) -> Vec<String> {
    use std::collections::BTreeSet;
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::Graphics::Gdi::{EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW};

    unsafe extern "system" fn collect(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _kind: u32, out: LPARAM) -> i32 {
        // SAFETY: GDI passes a valid LOGFONTW; `out` is the BTreeSet below, alive for the call.
        let (lf, set) = unsafe { (&*lf, &mut *(out as *mut BTreeSet<String>)) };
        let len = lf.lfFaceName.iter().position(|&c| c == 0).unwrap_or(lf.lfFaceName.len());
        let name = String::from_utf16_lossy(&lf.lfFaceName[..len]);
        if !name.is_empty() && !name.starts_with('@') {
            set.insert(name);
        }
        1
    }

    let mut set = BTreeSet::<String>::new();
    // SAFETY: plain GDI calls on the screen DC; the callback only touches `set`.
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return Vec::new();
        }
        let mut lf: LOGFONTW = std::mem::zeroed();
        lf.lfCharSet = DEFAULT_CHARSET;
        EnumFontFamiliesExW(dc, &lf, Some(collect), &mut set as *mut _ as LPARAM, 0);
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// Names starting with "." are system-only faces that cannot be picked by name.
#[cfg(target_os = "macos")]
pub fn installed_families(app: &tauri::AppHandle) -> Vec<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    // NSFontManager may only be used on the main thread
    let sent = app.run_on_main_thread(move || {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let names = objc2_app_kit::NSFontManager::sharedFontManager(mtm).availableFontFamilies();
        let _ = tx.send(names.iter().map(|n| n.to_string()).filter(|n| !n.is_empty() && !n.starts_with('.')).collect::<Vec<_>>());
    });
    let mut v = if sent.is_ok() { rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or_default() } else { Vec::new() };
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

#[cfg(target_os = "linux")]
pub fn installed_families(_app: &tauri::AppHandle) -> Vec<String> {
    let Ok(out) = std::process::Command::new("fc-list").args([":", "family"]).output() else { return Vec::new() };
    let mut v = families_from_fc_list(&String::from_utf8_lossy(&out.stdout));
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

/// One line per face, its names comma-separated (localized ones after the first); `\` escapes `-`, `,` and `:`.
#[cfg(any(target_os = "linux", test))]
fn families_from_fc_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split(',').next())
        .map(|f| f.replace('\\', "").trim().to_owned())
        .filter(|f| !f.is_empty() && !f.starts_with('.'))
        .collect()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn installed_families(_app: &tauri::AppHandle) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn fc_list_lines_give_their_first_family_name() {
        let text = "DejaVu Sans\nNoto Sans CJK JP,Noto Sans CJK JP Regular\nSource Code Pro\\-Light\n\n.LastResort\n";
        assert_eq!(super::families_from_fc_list(text), ["DejaVu Sans", "Noto Sans CJK JP", "Source Code Pro-Light"]);
    }
}
