//! Resolve helper-tool paths relative to the running executable.
//!
//! `bundle.sh` drops `qemu-img`, `socket_vmnet`, and friends into
//! `<app>.app/Contents/MacOS/` alongside the main binary.  At runtime we
//! prefer those bundled copies so end users don't need anything on `$PATH`.

use std::path::PathBuf;

/// Path to a helper tool bundled next to the current executable, falling
/// back to the bare tool name (so `Command::new` consults `$PATH`) when no
/// bundled copy is found.  This keeps dev runs (`cargo run`) working — the
/// fallback hits a Homebrew install — while shipped `.app` bundles always
/// use the embedded binary.
pub fn tool(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let mut candidates = vec![dir.join(name)];
            #[cfg(windows)]
            {
                let exe_name = if name.to_ascii_lowercase().ends_with(".exe") {
                    name.to_string()
                } else {
                    format!("{name}.exe")
                };
                candidates.push(dir.join(&exe_name));
                candidates.push(dir.join("qemu").join(&exe_name));
            }
            #[cfg(not(windows))]
            candidates.push(dir.join("qemu").join(name));

            for candidate in candidates {
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    #[cfg(windows)]
    {
        if !name.to_ascii_lowercase().ends_with(".exe") {
            return PathBuf::from(format!("{name}.exe"));
        }
    }
    PathBuf::from(name)
}
