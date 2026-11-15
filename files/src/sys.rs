//! OS seam: put text on the clipboard.

#[cfg_attr(target_os = "linux", allow(unused_imports))] // Error: Windows and macOS only
use knobdeck_sdk::{Error, Result};

// ── Windows ───────────────────────────────────────────────────────────────────

#[cfg(windows)]
pub fn set_clipboard(text: &str) -> Result {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    let werr = |what: &str, e: windows::core::Error| Error::msg(format!("{what}: {}", e.message()));
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| werr("OpenClipboard", e))?;
        let r = (|| {
            EmptyClipboard().map_err(|e| werr("EmptyClipboard", e))?;
            let g = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|e| werr("GlobalAlloc", e))?;
            let p = GlobalLock(g) as *mut u16;
            if p.is_null() {
                let _ = GlobalFree(g);
                return Err(Error::msg("GlobalLock failed"));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
            let _ = GlobalUnlock(g);
            // On success the clipboard owns the memory.
            SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(g.0)).map(|_| ()).map_err(|e| {
                let _ = GlobalFree(g);
                werr("SetClipboardData", e)
            })
        })();
        let _ = CloseClipboard();
        r
    }
}

// ── macOS ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
pub fn set_clipboard(text: &str) -> Result {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    if pb.setString_forType(&NSString::from_str(text), &NSString::from_str("public.utf8-plain-text")) {
        Ok(())
    } else {
        Err(Error::msg("Couldn't put the text on the clipboard"))
    }
}

// ── Linux ─────────────────────────────────────────────────────────────────────

/// Through the desktop's clipboard owner (Klipper on Plasma, the KnobDeck extension on GNOME),
/// as linux-tools does.
#[cfg(target_os = "linux")]
pub fn set_clipboard(text: &str) -> Result {
    use linux_desktop::{bus, desktop, Desktop};
    match desktop() {
        Desktop::Kde => Ok(bus::call0(&bus::session()?, "org.kde.klipper", "/klipper", "org.kde.klipper.klipper", "setClipboardContents", &(text,))?),
        Desktop::Gnome => Ok(linux_desktop::gnome::call("clipboard-set", &knobdeck_sdk::json!({ "text": text })).map(|_| ())?),
        Desktop::Other => Err(linux_desktop::unsupported()),
    }
}
