//! Read-only native delivery adapter. It never decrypts or grants authority.
use std::{fs, io::Read, path::Path};
use zeroize::Zeroizing;

pub fn read_sealed_collaboration(
    home: &Path,
    agent: &str,
) -> Result<Zeroizing<String>, &'static str> {
    if !home.is_absolute() || !matches!(agent, "codex" | "claude-code") {
        return Err("invalid collaboration installation");
    }
    let root = home.join(".hiroute");
    let directory = root.join("credential-artifacts");
    let path = directory.join(format!("{agent}.sealed"));
    for (path, is_dir) in [(&root, true), (&directory, true), (&path, false)] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "enable collaboration in Agent settings first")?;
        if metadata.file_type().is_symlink()
            || metadata.is_dir() != is_dir
            || (!is_dir && !metadata.is_file())
        {
            return Err("invalid collaboration installation");
        }
        #[cfg(not(unix))]
        return Err("owner-validated collaboration installation unavailable");
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let mut bytes = Zeroizing::new(String::new());
    options
        .open(path)
        .and_then(|file| file.take(4097).read_to_string(&mut bytes))
        .map_err(|_| "invalid collaboration installation")?;
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("invalid collaboration installation");
    }
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn sealed_delivery_accepts_readable_modes_but_requires_regular_file_and_exact_agent() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".hiroute");
        let directory = root.join("credential-artifacts");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let file = directory.join("codex.sealed");
        fs::write(&file, "opaque-sealed-delivery").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_sealed_collaboration(home.path(), "codex")
                .unwrap()
                .as_str(),
            "opaque-sealed-delivery"
        );
        assert!(read_sealed_collaboration(home.path(), "../codex").is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            read_sealed_collaboration(home.path(), "codex")
                .unwrap()
                .as_str(),
            "opaque-sealed-delivery"
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::remove_file(&file).unwrap();
        let other = directory.join("other");
        fs::write(&other, "do-not-follow").unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&other, &file).unwrap();
        assert!(read_sealed_collaboration(home.path(), "codex").is_err());
    }
}
