//! OS seam: put text on the clipboard, open a terminal in a folder.

use knobdeck_sdk::{Error, Result};
use std::path::Path;
use std::process::Command;

fn spawn(c: &mut Command) -> std::io::Result<()> {
    c.spawn().map(|_| ())
}

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

/// Windows Terminal when installed, else Command Prompt.
#[cfg(windows)]
pub fn terminal(dir: &Path) -> Result {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    if spawn(Command::new("wt.exe").arg("-d").arg(dir)).is_ok() {
        return Ok(());
    }
    spawn(Command::new("cmd.exe").arg("/K").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE)).map_err(|e| Error::msg(format!("Couldn't open a terminal: {e}")))
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

/// The Terminal app, opened on the folder.
#[cfg(target_os = "macos")]
pub fn terminal(dir: &Path) -> Result {
    spawn(Command::new("/usr/bin/open").args(["-a", "Terminal"]).arg(dir)).map_err(|e| Error::msg(format!("Couldn't open Terminal: {e}")))
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

/// The desktop's own terminal (Konsole, GNOME Console / Terminal), else whatever
/// `xdg-terminal-exec` picks.
#[cfg(target_os = "linux")]
pub fn terminal(dir: &Path) -> Result {
    use linux_desktop::{desktop, Desktop};
    let d = dir.as_os_str();
    let mut tries: Vec<Command> = vec![];
    let mut c = |prog: &str, args: &[&std::ffi::OsStr]| {
        let mut c = Command::new(prog);
        c.args(args).current_dir(dir);
        tries.push(c);
    };
    let wd = format!("--working-directory={}", dir.display());
    match desktop() {
        Desktop::Kde => c("konsole", &["--workdir".as_ref(), d]),
        _ => {
            c("kgx", &[wd.as_ref()]);
            c("gnome-terminal", &[wd.as_ref()]);
        }
    }
    c("xdg-terminal-exec", &[]);
    c("konsole", &["--workdir".as_ref(), d]);
    for mut cmd in tries {
        if spawn(&mut cmd).is_ok() {
            return Ok(());
        }
    }
    Err(Error::msg("No terminal app found (Konsole, Console or GNOME Terminal)"))
}
