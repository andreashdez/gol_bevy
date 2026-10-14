//! Write beside the destination, then replace it only after flushing the complete file.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

pub(crate) fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_with(path, |file| file.write_all(contents))
}

fn write_with(
    path: &Path,
    write_contents: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    // Renaming over a symlink would replace the link itself, so write to its target
    // instead (dotfile managers commonly symlink config files). A path that does not
    // exist yet, or a dangling link, is written as given.
    let resolved = fs::canonicalize(path);
    let path = resolved.as_deref().unwrap_or(path);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = parent.join(format!(".gol-bevy-{:016x}.tmp", fastrand::u64(..)));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        write_contents(&mut file)?;
        // Replacing the file must not loosen permissions the user set on it.
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()
    })();
    // Close before replacing or cleaning up, including on Windows.
    drop(file);
    let result = result.and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return result;
    }
    // Persist the rename itself. The new contents are already in place, so a
    // failure here is not worth reporting as a failed save.
    #[cfg(unix)]
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_write_failure_keeps_the_previous_file_and_removes_the_temporary_file() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("settings.toml");
        fs::write(&path, "original settings").unwrap();
        let result = write_with(&path, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::other("simulated write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original settings");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        write(&path, b"complete replacement").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "complete replacement");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("settings.toml");
        fs::write(&path, "original settings").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write(&path, b"new settings").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "new settings");
    }

    #[cfg(unix)]
    #[test]
    fn writing_through_a_symlink_replaces_the_target_and_keeps_the_link() {
        let directory = crate::test_support::TestDirectory::new();
        let dotfiles = directory.0.join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        let target = dotfiles.join("settings.toml");
        fs::write(&target, "original settings").unwrap();
        let link = directory.0.join("settings.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write(&link, b"new settings").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new settings");
        // No temporary file is left beside either the link or its target.
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
        assert_eq!(fs::read_dir(&dotfiles).unwrap().count(), 1);
    }
}
