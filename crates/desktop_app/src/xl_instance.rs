//! One Termy XL process. A second launch focuses the window that is already open.

use std::fs::{self, File, OpenOptions};
use std::path::Path;

use fs4::fs_std::FileExt;

pub struct InstanceLock {
    _file: File,
}

pub fn claim(root: &Path) -> Option<InstanceLock> {
    fs::create_dir_all(root).ok()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("instance.lock"))
        .ok()?;
    if file.try_lock_exclusive().ok()? {
        Some(InstanceLock { _file: file })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_on_the_same_root_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let primary = claim(dir.path());
        assert!(primary.is_some());
        assert!(claim(dir.path()).is_none());
        drop(primary);
        assert!(claim(dir.path()).is_some());
    }
}
