//! Temporary fixture trees for scanner tests (rule 9: %TEMP% only, never real user folders).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Unique suffix so parallel test binaries never collide.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A self-cleaning fixture directory under %TEMP%.
pub struct Fixture {
    path: PathBuf,
}

impl Fixture {
    /// Empty fixture directory.
    pub fn empty(name: &str) -> Fixture {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("bytemap-fixture-{}-{}", name, n));
        let _ = std::fs::remove_dir_all(&path); // stale leftovers from crashed runs
        std::fs::create_dir_all(&path).expect("create fixture dir");
        Fixture { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a file with `size` bytes of content.
    pub fn file(&self, rel: &str, size: usize) {
        let full = self.path.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).expect("create parent");
        std::fs::write(&full, vec![b'x'; size]).expect("write fixture file");
    }

    /// Junction `rel_link` -> `target_rel` via `cmd /c mklink /J` (no admin needed).
    pub fn junction(&self, rel_link: &str, target_rel: &str) -> bool {
        let link = self.path.join(rel_link);
        let target = self.path.join(target_rel);
        let out = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.as_os_str())
            .arg(target.as_os_str())
            .output()
            .expect("run mklink");
        out.status.success()
    }

    /// Hardlink `rel_link` -> `target_rel` via CreateHardLinkW (no admin needed).
    pub fn hardlink(&self, target_rel: &str, rel_link: &str) -> bool {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::CreateHardLinkW;
        fn wide(p: &Path) -> Vec<u16> {
            p.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
        let target = self.path.join(target_rel);
        let link = self.path.join(rel_link);
        let ok = unsafe {
            CreateHardLinkW(
                wide(&link).as_ptr(),
                wide(&target).as_ptr(),
                std::ptr::null(),
            )
        };
        ok != 0
    }

    /// Denies read access for the current user via icacls. Returns false when
    /// the setup itself fails (caller must skip loudly, never silently).
    pub fn deny_read(&self, rel: &str) -> bool {
        let dir = self.path.join(rel);
        std::fs::create_dir_all(&dir).expect("create denied dir");
        let user = std::env::var("USERNAME").unwrap_or_default();
        let out = Command::new("icacls")
            .arg(dir.as_os_str())
            .args(["/deny", &format!("{user}:(OI)(CI)R")])
            .output()
            .expect("run icacls");
        if !out.status.success() {
            return false;
        }
        // Self-verify: enumeration must now fail (skip loudly if it does not).
        std::fs::read_dir(&dir).is_err()
    }

    /// Deep chain for long-path coverage (each component 24 chars, 5 levels).
    pub fn deep(&self) -> PathBuf {
        let mut dir = self.path.clone();
        for i in 1..=5 {
            dir.push(format!("dddddddddddddddddddddddd{i}"));
        }
        std::fs::create_dir_all(&dir).expect("create deep dirs");
        dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Owner can always reset the DACL, lifting a deny ACE before deletion.
        let _ = Command::new("icacls")
            .arg(self.path.as_os_str())
            .args(["/reset", "/T"])
            .output();
        if std::fs::remove_dir_all(&self.path).is_err() {
            eprintln!(
                "WARNING: fixture cleanup failed, leftover at {:?}",
                self.path
            );
        }
    }
}
