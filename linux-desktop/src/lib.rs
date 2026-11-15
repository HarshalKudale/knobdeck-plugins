//! Shared by the Linux plugins (`linux-tools`, `spotify`): the KDE Plasma / GNOME window command
//! set ([`wm`]), XDG desktop entry icons ([`apps`]) and small process helpers.
//!
//! A thin layer over `knobdeck_sdk::os::linux` (the `knobdeck-os` crate KnobDeck itself uses),
//! which owns the D-Bus helpers, the KWin script, the KnobDeck GNOME Shell extension and the
//! desktop detection. They are re-exported here.
//!
//! Supported desktops: **KDE Plasma 6** and **GNOME 45+**. Others get a clear error from the
//! window functions; the freedesktop parts (D-Bus, desktop entries) work everywhere.
#![cfg(target_os = "linux")]

pub mod apps;
pub mod wm;

pub use knobdeck_sdk::os::linux::{bus, desktop, gnome, Desktop};

/// Error for features that need GNOME or KDE Plasma.
pub fn unsupported() -> knobdeck_sdk::Error {
    knobdeck_sdk::Error::msg("This action needs GNOME or KDE Plasma")
}

/// Start a program detached from the plugin (own process group, no inherited stdio), reaping it
/// in the background so it never lingers as a zombie.
pub fn spawn_detached(program: &str, args: &[&str]) -> knobdeck_sdk::Result {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| knobdeck_sdk::Error::msg(format!("Could not start {program}: {e}")))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The first of `programs` found on `PATH`.
pub fn which(programs: &[&'static str]) -> Option<&'static str> {
    let path = std::env::var_os("PATH")?;
    programs.iter().copied().find(|p| std::env::split_paths(&path).any(|d| is_executable(&d.join(p))))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_sh() {
        assert_eq!(which(&["no-such-program-kd", "sh"]), Some("sh"));
    }
}
