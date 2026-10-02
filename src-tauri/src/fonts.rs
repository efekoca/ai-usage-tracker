//! Installed font families, for the widget's font picker.

/// Sorted, de-duplicated family names of the fonts installed for this user and system-wide.
/// Vertical variants ("@…") are skipped.
#[cfg(windows)]
pub fn installed_families() -> Vec<String> {
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

#[cfg(not(windows))]
pub fn installed_families() -> Vec<String> {
    Vec::new()
}
