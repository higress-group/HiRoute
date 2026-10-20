//! Read-only export of this product's fixed current-user NSIS registration.
use super::job::{Child, Launch, Stream};
use std::{
    ffi::OsString,
    io::{self, Read},
    path::Path,
    time::{Duration, Instant},
};

pub fn export(destination: &Path) -> io::Result<()> {
    export_key(
        destination,
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\HiRoute",
    )
}

fn nonce() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn system_reg() -> io::Result<std::path::PathBuf> {
    let mut directory = [0_u16; 32768];
    // SAFETY: native system-directory API and an actual writable UTF-16 buffer.
    let count = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            directory.as_mut_ptr(),
            directory.len() as u32,
        )
    } as usize;
    if count == 0 || count >= directory.len() {
        return Err(io::Error::last_os_error());
    }
    let directory = String::from_utf16(&directory[..count]).map_err(io::Error::other)?;
    Ok(Path::new(&directory).join("reg.exe"))
}

fn export_to(key: &str, destination: &Path) -> io::Result<()> {
    let reg = system_reg()?;
    let windows = reg
        .parent()
        .and_then(Path::parent)
        .ok_or(io::ErrorKind::InvalidInput)?;
    let args = [
        "export".into(),
        key.into(),
        destination.as_os_str().to_owned(),
        "/y".into(),
    ];
    let temporary = destination.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let env = [
        (OsString::from("SystemRoot"), windows.as_os_str().to_owned()),
        (OsString::from("TEMP"), temporary.as_os_str().to_owned()),
        (OsString::from("TMP"), temporary.as_os_str().to_owned()),
    ];
    // reg.exe also needs a writable scratch directory while exporting. With a
    // cleared environment its Windows-directory fallback fails for an ordinary
    // account, even when the final destination itself is writable. Keep every
    // scratch file under this already private, held staging directory.
    // The existing launcher gives only this suspended child private user-SID
    // defaults, including for an elevated caller, without changing the parent.
    let child = Child::spawn(Launch {
        program: &reg,
        args: &args,
        env: &env,
        cwd: destination.parent(),
        stdin: Stream::Null,
        stdout: Stream::Null,
        stderr: if cfg!(test) {
            Stream::Piped
        } else {
            Stream::Null
        },
    })?;
    #[cfg(test)]
    let mut child = child;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(code) = child.exit_code()? {
            if code != 0 {
                #[cfg(test)]
                if let Some(stderr) = child.stderr.as_mut() {
                    let mut bytes = Vec::new();
                    let _ = stderr.take(4096).read_to_end(&mut bytes);
                    eprintln!(
                        "registry system child exit={code}, bounded stderr={:?}, bytes={bytes:02x?}",
                        String::from_utf8_lossy(&bytes),
                    );
                }
                return Err(io::Error::other("SHELL_REGISTRATION_UNAVAILABLE"));
            }
            if child.empty()? {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            let _ = child.terminate();
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_child(parent: &std::fs::File, name: &std::ffi::OsStr) -> io::Result<Vec<u8>> {
    // reg.exe creates an inherited DACL. Admit only the controlled child's
    // single-owner regular file under this held, strictly private parent.
    let file = super::security::open_inherited_private_file_at(parent, name)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

fn export_key(destination: &Path, key: &str) -> io::Result<()> {
    let parent = super::security::open_owner_only(
        destination.parent().ok_or(io::ErrorKind::InvalidInput)?,
        true,
    )?;
    let name = destination.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    // Keep unpublished external output outside the immutable retained package.
    // A crash cannot leave a partial export at its final recovery name.
    let stage_path = std::env::temp_dir().join(format!("hiroute-registry-{}", nonce()?));
    let mut stage = crate::private_fs::OwnedDirectory::create(&stage_path)?;
    let result = (|| {
        let staged = stage.path().join("uninstall.reg");
        export_to(key, &staged)?;
        let held = super::security::open_owner_only(stage.path(), true)?;
        let bytes = read_child(&held, std::ffi::OsStr::new("uninstall.reg"))?;
        match read_child(&parent, name) {
            Ok(previous) => {
                if previous != bytes {
                    return Err(io::Error::other("SHELL_REGISTRATION_CHANGED"));
                }
                // An interrupted earlier preparation may have left the same
                // complete bytes with inherited protection. Publish only the
                // independently re-exported, identical value with private ACLs.
                crate::durable::write_private_atomic(destination, &bytes)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                crate::durable::write_private_new(destination, &bytes)?;
            }
            Err(error) => return Err(error),
        }
        super::security::open_owner_only(destination, false)?;
        Ok(())
    })();
    let _ = stage.remove();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    struct Registry(String);
    impl Drop for Registry {
        fn drop(&mut self) {
            let _ = Command::new(system_reg().unwrap())
                .args(["delete", &self.0, "/f"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    #[test]
    fn inherited_export_is_published_privately_and_changed_material_is_not_overwritten() {
        let registry = Registry(format!(
            r"HKCU\Software\HiRouteRecoveryTest-{}",
            nonce().unwrap()
        ));
        assert!(
            Command::new(system_reg().unwrap())
                .args([
                    "add",
                    &registry.0,
                    "/v",
                    "Marker",
                    "/t",
                    "REG_SZ",
                    "/d",
                    "original",
                    "/f",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        super::super::security::ensure_private_directory(&private).unwrap();
        let inherited = private.join("inherited.reg");
        export_to(&registry.0, &inherited).unwrap();
        assert!(
            super::super::security::open_owner_only(&inherited, false).is_err(),
            "the previous strict-reader path rejects native reg.exe inheritance"
        );
        let original = std::fs::read(&inherited).unwrap();
        export_key(&inherited, &registry.0).unwrap();
        super::super::security::open_owner_only(&inherited, false).unwrap();
        assert_eq!(std::fs::read(&inherited).unwrap(), original);
        let destination = private.join("retained.reg");
        export_key(&destination, &registry.0).unwrap();
        super::super::security::open_owner_only(&destination, false).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        crate::durable::write_private_atomic(&destination, b"changed retained material").unwrap();
        assert!(export_key(&destination, &registry.0).is_err());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"changed retained material"
        );
        let missing = private.join("missing.reg");
        assert!(export_key(&missing, &(registry.0.clone() + "-missing")).is_err());
        assert!(
            !missing.exists(),
            "an unsuccessful export must not publish a recovery artifact"
        );
    }
}
