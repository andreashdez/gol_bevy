//! Helpers shared by tests in several modules.
use std::{fs, path::PathBuf};

/// A fresh directory under the system temp folder, removed when dropped.
pub(crate) struct TestDirectory(pub PathBuf);
impl TestDirectory {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("gol-bevy-test-{:016x}", fastrand::u64(..)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
