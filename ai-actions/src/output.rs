//! OS seam: put the answer on the clipboard, or enter it in place of the selection.
//!
//! Entering text is the one place this plugin produces input events, on purpose (the action's
//! job *is* entering text, like Text Snippet): Windows and Linux paste it (Ctrl+V after putting it
//! on the clipboard: long answers arrive at once and line breaks don't press Enter in chat apps);
//! macOS types it as Unicode key events, line breaks as ⇧Return.

#[cfg_attr(target_os = "linux", allow(unused_imports))]
use knobdeck_sdk::{Error, Result};

// ── Windows ───────────────────────────────────────────────────────────────────

#[cfg(windows)]
pub fn set_clipboard(text: &str) -> Result {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    let werr = |what: &str, e: windows::core::Error| Error::msg(format!("{what}: {}", e.message()));
    // CRLF line breaks: Windows apps paste a bare LF as nothing or as one line.
    let wide: Vec<u16> = text.replace("\r\n", "\n").replace('\n', "\r\n").encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Another app can hold the clipboard for a moment; try a few times.
        let mut opened = OpenClipboard(HWND::default());
        for _ in 0..5 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            opened = OpenClipboard(HWND::default());
        }
        opened.map_err(|e| werr("Couldn't open the clipboard", e))?;
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

/// Put `text` on the clipboard and press Ctrl+V in the window that has focus. Windows has no
/// app-independent paste call (`WM_PASTE` reaches only classic edit controls).
#[cfg(windows)]
pub fn enter_text(text: &str) -> Result {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_V};
    /// KnobDeck's `INJECTED_INPUT_TAG`: the dial's keyboard hook lets keys carrying it through.
    const INJECTED_INPUT_TAG: usize = 0x4B4E_4F42;
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        let flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
        INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: INJECTED_INPUT_TAG } } }
    }
    set_clipboard(text)?;
    std::thread::sleep(std::time::Duration::from_millis(40));
    let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(Error::msg("Windows blocked the paste (an app running as administrator is in front). The answer is on the clipboard"));
    }
    Ok(())
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
        Err(Error::msg("Couldn't put the answer on the clipboard"))
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXIsProcessTrusted() -> bool;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventCreateKeyboardEvent(src: *const c_void, key: u16, down: bool) -> *mut c_void;
        pub fn CGEventSetFlags(ev: *mut c_void, flags: u64);
        pub fn CGEventSetIntegerValueField(ev: *mut c_void, field: u32, value: i64);
        pub fn CGEventPost(tap: u32, ev: *mut c_void);
        pub fn CGEventKeyboardSetUnicodeString(ev: *mut c_void, len: usize, chars: *const u16);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(cf: *const c_void);
    }
    /// KnobDeck's tag for its own injected input, so its key listener ignores these events.
    pub const INJECTED_INPUT_TAG: i64 = 0x4B4E_4F42;
    pub const FIELD_SOURCE_USER_DATA: u32 = 42;
    pub const RETURN: u16 = 36;
    pub const SHIFT_MASK: u64 = 0x2_0000;
}

/// Type `text` into the focused app as Unicode key events (layout-independent).
#[cfg(target_os = "macos")]
pub fn enter_text(text: &str) -> Result {
    use mac::*;
    if !unsafe { AXIsProcessTrusted() } {
        return Err(Error::msg("Allow KnobDeck in System Settings ▸ Privacy & Security ▸ Accessibility to type the answer. It is on the clipboard"));
    }
    for c in text.chars().filter(|&c| c != '\r') {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        for down in [true, false] {
            unsafe {
                let ev = CGEventCreateKeyboardEvent(std::ptr::null(), if c == '\n' { RETURN } else { 0 }, down);
                if ev.is_null() {
                    return Err(Error::msg("Couldn't create the key event"));
                }
                // Line breaks as ⇧Return (a new line in chat apps, not "send"); no other
                // modifiers, so a held ⌘/⌥ can't turn letters into shortcuts.
                CGEventSetFlags(ev, if c == '\n' { SHIFT_MASK } else { 0 });
                if c != '\n' {
                    CGEventKeyboardSetUnicodeString(ev, units.len(), units.as_ptr());
                }
                CGEventSetIntegerValueField(ev, FIELD_SOURCE_USER_DATA, INJECTED_INPUT_TAG);
                CGEventPost(0, ev); // kCGHIDEventTap
                CFRelease(ev);
            }
        }
        // Apps drop characters when events arrive faster than their run loop drains them.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
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

/// Put `text` on the clipboard and paste it with Ctrl+V. Linux has no app-independent paste
/// request (Wayland lets no client act on another's text field), and the virtual keyboard can't
/// type most non-ASCII text, so this is what linux-tools' Text Snippet does too.
#[cfg(target_os = "linux")]
pub fn enter_text(text: &str) -> Result {
    set_clipboard(text)?;
    // Give the clipboard owner a moment to announce the new selection before the app reads it.
    std::thread::sleep(std::time::Duration::from_millis(60));
    knobdeck_sdk::send_keys("ctrl v")
}
