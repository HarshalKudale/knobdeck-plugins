//! Close Game: find the running processes started from a known game's install folder (or its one
//! exe), ask them to close, and terminate the ones still running after a grace period.
//!
//! - Windows: `WM_CLOSE` posted to the process's top-level windows (what the window's close
//!   button sends), then `TerminateProcess`.
//! - macOS / Linux: `SIGTERM`, then `SIGKILL`. On Linux, Wine / Proton games are matched by the
//!   Windows path in their command line (`Z:\…` or a `C:\…` path inside their `WINEPREFIX`).

use crate::library::Game;
use std::path::{Path, PathBuf};

/// What a game's processes are matched against.
#[derive(Clone, Debug)]
pub enum Owner {
    /// Every process whose exe is inside this folder.
    Dir(PathBuf),
    /// Only this exe.
    Exe(PathBuf),
}

impl Owner {
    pub fn of(g: &Game) -> Option<Owner> {
        if let Some(e) = &g.exe {
            return Some(Owner::Exe(e.clone()));
        }
        g.dir.clone().filter(|d| safe_dir(d)).map(Owner::Dir)
    }
    pub fn owns(&self, exe: &Path) -> bool {
        // Windows paths compare without case (the registry and the process list may differ).
        let norm = |p: &Path| if cfg!(windows) { PathBuf::from(p.to_string_lossy().to_lowercase()) } else { p.to_path_buf() };
        let exe = norm(exe);
        match self {
            Owner::Dir(d) => exe.starts_with(norm(d)),
            Owner::Exe(e) => exe == norm(e),
        }
    }
}

/// A folder narrow enough to own processes: not a filesystem root, not the home folder or one
/// above it, at least two levels deep (`C:\Games\X`, `/home/u/Games/X`).
pub fn safe_dir(d: &Path) -> bool {
    if !d.is_absolute() || d.components().count() < 3 {
        return false;
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    !home.is_some_and(|h| h.starts_with(d))
}

/// Running processes `(pid, exe)` owned by any of `owners`, never this plugin itself.
pub fn find(owners: &[Owner]) -> Vec<(u32, PathBuf)> {
    let me = std::process::id();
    os::processes().into_iter().filter(|(pid, exe)| *pid != me && owners.iter().any(|o| o.owns(exe))).collect()
}

/// Ask the processes to close.
pub fn close(pids: &[u32]) {
    os::close(pids)
}

/// Terminate the processes.
pub fn kill(pids: &[u32]) {
    os::kill(pids)
}

/// A Wine command-line path as a host path: `Z:\home\u\x.exe` → `/home/u/x.exe`;
/// `C:\Games\x.exe` → `<prefix>/drive_c/Games/x.exe`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn wine_path(arg: &str, prefix: &Path) -> Option<PathBuf> {
    let b = arg.as_bytes();
    if b.len() < 4 || b[1] != b':' || (b[2] != b'\\' && b[2] != b'/') {
        return None;
    }
    let rest = arg[3..].replace('\\', "/");
    match b[0].to_ascii_uppercase() {
        b'Z' => Some(PathBuf::from("/").join(rest)),
        b'C' => Some(prefix.join("drive_c").join(rest)),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::path::PathBuf;

    fn wine_exe(pid: u32) -> Option<PathBuf> {
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let argv0 = cmd.split(|&b| b == 0).next()?;
        let argv0 = String::from_utf8_lossy(argv0);
        let prefix = std::fs::read(format!("/proc/{pid}/environ"))
            .ok()
            .and_then(|env| env.split(|&b| b == 0).find_map(|kv| kv.strip_prefix(b"WINEPREFIX=").map(|v| PathBuf::from(String::from_utf8_lossy(v).to_string()))))
            .unwrap_or_else(|| crate::library::home().join(".wine"));
        super::wine_path(&argv0, &prefix)
    }

    pub fn processes() -> Vec<(u32, PathBuf)> {
        let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
        let mut v = vec![];
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            // Other users' processes can't be read (and couldn't be closed).
            let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else { continue };
            let wine = exe.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("wine") || n.ends_with("-preloader"));
            v.push((pid, if wine { wine_exe(pid).unwrap_or(exe) } else { exe }));
        }
        v
    }

    pub fn close(pids: &[u32]) {
        for &p in pids {
            unsafe { libc::kill(p as i32, libc::SIGTERM) };
        }
    }

    pub fn kill(pids: &[u32]) {
        for &p in pids {
            unsafe { libc::kill(p as i32, libc::SIGKILL) };
        }
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::ffi::c_void;
    use std::path::PathBuf;

    pub fn processes() -> Vec<(u32, PathBuf)> {
        let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if n <= 0 {
            return vec![];
        }
        let mut pids = vec![0i32; n as usize + 64];
        let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr() as *mut c_void, (pids.len() * std::mem::size_of::<i32>()) as i32) };
        pids.truncate(n.max(0) as usize);
        pids.into_iter()
            .filter(|&p| p > 0)
            .filter_map(|p| {
                let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
                let len = unsafe { libc::proc_pidpath(p, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
                (len > 0).then(|| (p as u32, PathBuf::from(String::from_utf8_lossy(&buf[..len as usize]).to_string())))
            })
            .collect()
    }

    pub fn close(pids: &[u32]) {
        for &p in pids {
            unsafe { libc::kill(p as i32, libc::SIGTERM) };
        }
    }

    pub fn kill(pids: &[u32]) {
        for &p in pids {
            unsafe { libc::kill(p as i32, libc::SIGKILL) };
        }
    }
}

#[cfg(windows)]
mod os {
    use std::path::PathBuf;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, TRUE, WPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE};

    fn exe_path(pid: u32) -> Option<PathBuf> {
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_FORMAT(0), PWSTR(buf.as_mut_ptr()), &mut size) }.is_ok();
        let _ = unsafe { CloseHandle(h) };
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
    }

    pub fn processes() -> Vec<(u32, PathBuf)> {
        let mut pids = vec![];
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return vec![] };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if unsafe { Process32FirstW(snap, &mut e) }.is_ok() {
            loop {
                pids.push(e.th32ProcessID);
                if unsafe { Process32NextW(snap, &mut e) }.is_err() {
                    break;
                }
            }
        }
        let _ = unsafe { CloseHandle(snap) };
        pids.into_iter().filter(|&p| p > 4).filter_map(|p| exe_path(p).map(|e| (p, e))).collect()
    }

    unsafe extern "system" fn post_close(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pids = &*(lparam.0 as *const Vec<u32>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() {
            // Posted, not sent: a hung game can't block the plugin.
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        TRUE
    }

    pub fn close(pids: &[u32]) {
        let v: Vec<u32> = pids.to_vec();
        let _ = unsafe { EnumWindows(Some(post_close), LPARAM(&v as *const Vec<u32> as isize)) };
    }

    pub fn kill(pids: &[u32]) {
        for &p in pids {
            if let Ok(h) = unsafe { OpenProcess(PROCESS_TERMINATE, false, p) } {
                let _ = unsafe { TerminateProcess(h, 1) };
                let _ = unsafe { CloseHandle(h) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners() {
        assert!(!safe_dir(Path::new("/")));
        assert!(!safe_dir(Path::new("/opt")));
        assert!(!safe_dir(Path::new("relative/dir/x")));
        assert!(safe_dir(Path::new("/opt/games/celeste")));
        if let Some(h) = std::env::var_os("HOME").filter(|_| !cfg!(windows)) {
            let h = PathBuf::from(h);
            assert!(!safe_dir(&h), "the home folder never owns processes");
            assert!(safe_dir(&h.join("Games/x")));
        }
        let d = Owner::Dir("/opt/games/celeste".into());
        assert!(d.owns(Path::new("/opt/games/celeste/Celeste")) && !d.owns(Path::new("/opt/games/celeste2/x")));
        let e = Owner::Exe("/usr/bin/foo".into());
        assert!(e.owns(Path::new("/usr/bin/foo")) && !e.owns(Path::new("/usr/bin/bar")));
    }

    #[test]
    fn wine_paths() {
        let pfx = Path::new("/home/u/.wine");
        assert_eq!(wine_path("Z:\\home\\u\\Games\\x\\x.exe", pfx), Some(PathBuf::from("/home/u/Games/x/x.exe")));
        assert_eq!(wine_path("C:\\Program Files\\G\\g.exe", pfx), Some(PathBuf::from("/home/u/.wine/drive_c/Program Files/G/g.exe")));
        assert_eq!(wine_path("/usr/bin/wine", pfx), None);
        assert_eq!(wine_path("D:\\x.exe", pfx), None);
    }

    /// Starts a real process from a temp "install folder" and closes it with SIGTERM.
    #[cfg(target_os = "linux")]
    #[test]
    fn closes_a_process_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("knobdeck-close-{}", std::process::id())).join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("sleeper");
        std::fs::copy("/usr/bin/sleep", &exe).unwrap();
        let mut child = std::process::Command::new(&exe).arg("60").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let found = find(&[Owner::Dir(dir.clone())]);
        assert_eq!(found.iter().map(|p| p.0).collect::<Vec<_>>(), [child.id()]);
        close(&[child.id()]);
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(find(&[Owner::Dir(dir.clone())]).is_empty());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
