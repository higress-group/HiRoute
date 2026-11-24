//! The installed program may run in place or from the protected per-user
//! directory. The selected path and native directory identity are kept private.
use super::{INVENTORY, UNINSTALLER, data, hash_file, invalid, present, read, write};
use hiroute_platform::windows::{security, shell_location};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const RECORD: &str = "desktop-location.json";
const SCHEMA: &str = "hiroute.windows-desktop-location/v1";
type Identity = (u32, u128);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Location {
    schema: String,
    selection: PathBuf,
    destination: PathBuf,
    destination_id: Identity,
    runtime_id: Identity,
    created_runtime: bool,
    package: Option<PackageCopy>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageCopy {
    nonce: String,
    sha256: String,
    file_id: Option<Identity>,
}

pub(super) struct Choice {
    pub(super) selection: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) runtime: PathBuf,
    pub(super) local_copy: bool,
}

pub(super) fn choice(
    input: &Path,
    active: Option<&Path>,
    old: Option<&Location>,
) -> io::Result<Choice> {
    let selection = shell_location::normalize_destination(input)?;
    let destination = shell_location::resolve_destination(&selection).map_err(unavailable)?;
    if let Some(active) = active {
        let expected = old.map_or(active, |value| &value.destination);
        if !shell_location::same_name(&destination, expected)
            && (old.is_some()
                || !shell_location::same_existing_path(&destination, expected, true)
                    .unwrap_or(false))
        {
            return Err(io::Error::other("INSTALL_DESTINATION_ALREADY_REGISTERED"));
        }
        if let Some(value) = old {
            value.verify_runtime()?;
            if value.package.is_some() {
                value.open_destination()?;
            }
        }
        let local_copy = old.is_some_and(|value| value.package.is_some());
        let runtime = if local_copy {
            active.to_owned()
        } else {
            destination.clone()
        };
        if foreign_program_inputs(&runtime)? {
            return Err(io::Error::other("INSTALL_DESTINATION_PROGRAM_FILES"));
        }
        return Ok(Choice {
            selection: old.map_or(selection, |value| value.selection.clone()),
            destination,
            runtime,
            local_copy,
        });
    }
    let default = shell_location::default_root()?;
    let local_copy = shell_location::validate_custom(&destination).is_err()
        || !shell_location::stable_program_volume(&destination)
        || foreign_program_inputs(&destination)?;
    if local_copy && shell_location::same_name(&destination, &default) {
        return Err(io::Error::other("INSTALL_DESTINATION_PROGRAM_FILES"));
    }
    let runtime = if local_copy {
        default
    } else {
        destination.clone()
    };
    if local_copy
        && (destination
            .ancestors()
            .any(|part| shell_location::same_name(part, &runtime))
            || runtime
                .ancestors()
                .any(|part| shell_location::same_name(part, &destination)))
    {
        return Err(io::Error::other("INSTALL_DESTINATION_OVERLAPS_RUNTIME"));
    }
    if local_copy && foreign_program_inputs(&runtime)? {
        return Err(io::Error::other("INSTALL_DESTINATION_PROGRAM_FILES"));
    }
    for name in [
        "hiroute-shell.exe",
        UNINSTALLER,
        INVENTORY,
        super::program_writes::PREFERENCES,
        super::program_writes::STAGING,
        "standalone",
    ] {
        if present(&runtime.join(name))? {
            return Err(io::Error::other(format!(
                "INSTALL_DESTINATION_OCCUPIED: {name}"
            )));
        }
    }
    Ok(Choice {
        selection,
        destination,
        runtime,
        local_copy,
    })
}

// Windows can resolve libraries, redirection manifests and private assemblies
// from the executable directory before the shell runs its first instruction.
fn foreign_program_inputs(root: &Path) -> io::Result<bool> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "standalone" | "hiroute-shell.exe" | "uninstall.exe"
        ) || name == super::program_writes::STAGING
        {
            continue;
        }
        if !entry.file_type()?.is_file()
            || name.rsplit_once('.').is_some_and(|(_, extension)| {
                matches!(
                    extension,
                    "dll" | "exe" | "manifest" | "local" | "ocx" | "drv" | "cpl" | "ax"
                )
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Choice {
    pub(super) fn prepare(&self, source: Option<&mut File>) -> io::Result<Location> {
        let mut previous = committed()?;
        let (runtime, created) = match security::open_identity(&self.runtime, true) {
            Ok(directory) => (directory, false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Some(old) = &previous
                    && !shell_location::same_name(&old.verify_runtime()?, &self.runtime)
                {
                    return Err(changed());
                }
                let parent = self.runtime.parent().ok_or_else(invalid)?;
                let leaf = self.runtime.file_name().ok_or_else(invalid)?;
                let parent = security::ensure_private_directory(parent).map_err(directory_error)?;
                let directory = security::create_owner_only_directory_at(&parent, leaf)
                    .map_err(directory_error)?;
                // Only the exclusive creator may bind the replacement. Keep the
                // original selection and package ownership through a failed repair.
                if let Some(old) = &mut previous
                    && let Err(error) = old.bind_recreated_runtime(&directory)
                {
                    security::delete_by_handle(&directory)?;
                    return Err(error);
                }
                (directory, true)
            }
            Err(error) => return Err(directory_error(error)),
        };
        shell_location::validate_custom(&self.runtime).map_err(directory_error)?;
        let runtime_id = identity(&runtime)?;
        let (destination_id, package) = if self.local_copy {
            let directory = shell_location::ensure_package_directory(&self.destination)
                .map_err(directory_error)?;
            let source = source.ok_or_else(invalid)?;
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).map_err(|err| io::Error::other(err.to_string()))?;
            let sha256 = hash_file(source)?;
            source.seek(SeekFrom::Start(0))?;
            (
                identity(&directory)?,
                Some(PackageCopy {
                    nonce: format!("{:032x}", u128::from_be_bytes(nonce)),
                    sha256,
                    file_id: None,
                }),
            )
        } else {
            (runtime_id, None)
        };
        drop(runtime);
        let value = Location {
            schema: SCHEMA.into(),
            selection: self.selection.clone(),
            destination: self.destination.clone(),
            destination_id,
            runtime_id,
            created_runtime: created
                || previous.is_some_and(|old| old.created_runtime && old.runtime_id == runtime_id),
            package,
        };
        value.verify_runtime()?;
        if value.package.is_some() {
            value.open_destination()?;
        }
        Ok(value)
    }
}

impl PackageCopy {
    fn name(&self) -> String {
        format!("HiRoute-{}.hiroute-package", self.nonce)
    }

    fn validate(&self) -> io::Result<()> {
        if !hex(&self.nonce, 32)
            || !hex(&self.sha256, 64)
            || self.file_id.is_some_and(|id| id.1 == 0)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Location {
    fn bind_recreated_runtime(&mut self, directory: &File) -> io::Result<()> {
        self.runtime_id = identity(directory)?;
        if self.package.is_none() {
            self.destination_id = self.runtime_id;
        }
        self.created_runtime = true;
        write(&record_path()?, self)
    }

    pub(super) fn selection(&self) -> &Path {
        &self.selection
    }

    pub(super) fn runtime(&self) -> io::Result<PathBuf> {
        if self.package.is_some() {
            shell_location::default_root()
        } else {
            Ok(self.destination.clone())
        }
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        if self.schema != SCHEMA
            || shell_location::normalize_destination(&self.selection)? != self.selection
            || shell_location::normalize_destination(&self.destination)? != self.destination
            || self.destination_id.1 == 0
            || self.runtime_id.1 == 0
            || self.package.is_none() && self.destination_id != self.runtime_id
        {
            return Err(invalid());
        }
        if let Some(package) = &self.package {
            package.validate()?;
        }
        Ok(())
    }

    pub(super) fn verify_runtime(&self) -> io::Result<PathBuf> {
        let runtime = self.runtime()?;
        if self.package.is_none() {
            self.verify_selection()?;
        }
        shell_location::validate_custom(&runtime).map_err(unavailable)?;
        match security::open_identity(&runtime, true) {
            Ok(directory) => require_identity(&directory, self.runtime_id)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Repair can recreate an absent program directory on the same
                // volume. A different directory at that path is never adopted.
                let anchor = runtime
                    .ancestors()
                    .skip(1)
                    .find(|parent| parent.is_dir())
                    .ok_or_else(|| unavailable(error))?;
                if identity(&security::open_identity(anchor, true)?)?.0 != self.runtime_id.0 {
                    return Err(changed());
                }
            }
            Err(error) => return Err(unavailable(error)),
        }
        Ok(runtime)
    }

    fn verify_selection(&self) -> io::Result<()> {
        let current = shell_location::resolve_destination(&self.selection).map_err(unavailable)?;
        if !shell_location::same_name(&current, &self.destination) {
            return Err(changed());
        }
        Ok(())
    }

    fn open_destination(&self) -> io::Result<File> {
        self.verify_selection()?;
        let directory = security::open_identity(&self.destination, true).map_err(unavailable)?;
        require_identity(&directory, self.destination_id)?;
        Ok(directory)
    }

    fn package_for_removal(&self) -> io::Result<Option<File>> {
        let Some(package) = &self.package else {
            return Ok(None);
        };
        let directory = self.open_destination()?;
        let Some(expected) = package.file_id else {
            return Ok(None);
        };
        let name = package.name();
        let file = match shell_location::open_package_for_removal(&directory, OsStr::new(&name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(unavailable(error)),
        };
        // A shared copy is data. A partial or modified copy is preserved.
        if identity(&file)? == expected
            && security::file_link_count(&file)? == 1
            && hash_file(&file).is_ok_and(|actual| actual == package.sha256)
        {
            return Ok(Some(file));
        }
        Ok(None)
    }

    fn remove_package(&self) -> io::Result<()> {
        if let Some(file) = self.package_for_removal()? {
            security::delete_by_handle(&file)?;
        }
        Ok(())
    }
}

pub(super) fn committed() -> io::Result<Option<Location>> {
    match read::<Location>(&record_path()?, true) {
        Ok(value) => {
            value.validate()?;
            Ok(Some(value))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn record_path() -> io::Result<PathBuf> {
    Ok(data()?.join(RECORD))
}

pub(super) fn recovery_record() -> io::Result<Option<(PathBuf, String)>> {
    if committed()?.is_none() {
        return Ok(None);
    }
    let path = record_path()?;
    let digest = hash_file(&security::open_owner_only(&path, false)?)?;
    Ok(Some((path, digest)))
}

pub(super) fn publish_package(pending: &mut super::Pending, source: &mut File) -> io::Result<()> {
    let value = pending.location.as_ref().ok_or_else(invalid)?;
    let directory = value.open_destination()?;
    let package = value.package.as_ref().ok_or_else(invalid)?;
    let expected = package.sha256.clone();
    let mut file = shell_location::create_package_at(&directory, OsStr::new(&package.name()))
        .map_err(directory_error)?;
    let publish = (|| {
        let package = pending
            .location
            .as_mut()
            .and_then(|v| v.package.as_mut())
            .ok_or_else(invalid)?;
        package.file_id = Some(identity(&file)?);
        write(&data()?.join(super::PENDING), pending)?;
        io::copy(source, &mut file)?;
        file.sync_all()?;
        file.seek(SeekFrom::Start(0))?;
        if hash_file(&file)? != expected {
            return Err(io::Error::other("INSTALL_DESTINATION_COPY_MISMATCH"));
        }
        Ok(())
    })();
    if publish.is_err() {
        // This newly created file is still exclusively held by the writer.
        let _ = security::delete_by_handle(&file);
    }
    publish.map_err(directory_error)
}

pub(super) fn finish_commit(location: Option<&Location>) -> io::Result<()> {
    let Some(location) = location else {
        return Ok(());
    };
    // publish_package verified and flushed the copy on its exclusive handle.
    // Its recipient may move or edit it afterwards, including during recovery.
    let new_nonce = location.package.as_ref().map(|v| &v.nonce);
    if let Some(old) = committed()?
        && old
            .package
            .as_ref()
            .is_some_and(|v| Some(&v.nonce) != new_nonce)
    {
        old.remove_package()?;
    }
    write(&record_path()?, location)
}

pub(super) fn rollback(pending: &super::Pending, runtime: &Path) -> io::Result<()> {
    if let Some(location) = &pending.location {
        location.remove_package()?;
    }
    if pending.old.is_empty() && committed()?.is_none() {
        shell_location::remove_unpublished_registration(runtime)?;
    }
    Ok(())
}

pub(super) fn uninstall_package() -> io::Result<Option<File>> {
    committed()?.map_or(Ok(None), |location| location.package_for_removal())
}

pub(super) fn finish_uninstall(runtime: &Path) -> io::Result<()> {
    let Some(location) = committed()? else {
        // Older installations did not record who created the directory.
        return Ok(());
    };
    if !shell_location::same_name(runtime, &location.verify_runtime()?) {
        return Err(changed());
    }
    if location.created_runtime {
        match security::open_delete(runtime, true) {
            Ok(directory) => {
                require_identity(&directory, location.runtime_id)?;
                match security::delete_by_handle(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    fs::remove_file(record_path()?)
}

fn identity(file: &File) -> io::Result<Identity> {
    shell_location::entry_identity(file)
}

fn require_identity(file: &File, expected: Identity) -> io::Result<()> {
    if identity(file)? != expected {
        return Err(changed());
    }
    Ok(())
}

fn changed() -> io::Error {
    io::Error::other("INSTALL_DESTINATION_CHANGED")
}

fn unavailable(error: io::Error) -> io::Error {
    if error.to_string().starts_with("INSTALL_") {
        return error;
    }
    io::Error::new(
        error.kind(),
        format!("INSTALL_DESTINATION_UNAVAILABLE: {error}"),
    )
}

fn directory_error(error: io::Error) -> io::Error {
    if error.to_string().starts_with("INSTALL_") {
        return error;
    }
    let code = match error.kind() {
        io::ErrorKind::StorageFull => "INSTALL_DESTINATION_SPACE",
        io::ErrorKind::PermissionDenied => "INSTALL_DESTINATION_NOT_WRITABLE",
        _ => "INSTALL_DESTINATION_WRITE_FAILED",
    };
    io::Error::new(error.kind(), format!("{code}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn shared_package_cleanup_preserves_changed_bytes_and_replacement_directories() {
        let parent = tempfile::tempdir().unwrap();
        let selection = parent.path().join("selection");
        let destination = shell_location::resolve_destination(&selection).unwrap();
        let directory = shell_location::ensure_package_directory(&destination).unwrap();
        fs::write(destination.join("notes.txt"), b"keep").unwrap();
        assert!(!foreign_program_inputs(&destination).unwrap());
        fs::write(destination.join("another-program.manifest"), b"keep").unwrap();
        assert!(foreign_program_inputs(&destination).unwrap());
        let mut copy = PackageCopy {
            nonce: "a".repeat(32),
            sha256: String::new(),
            file_id: None,
        };
        let name = copy.name();
        let path = destination.join(&name);
        let mut created = shell_location::create_package_at(&directory, OsStr::new(&name)).unwrap();
        created.write_all(b"installer package").unwrap();
        created.sync_all().unwrap();
        drop(created);
        assert_eq!(
            shell_location::create_package_at(&directory, OsStr::new(&name))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        let file = security::open_identity(&path, false).unwrap();
        copy.sha256 = hash_file(&file).unwrap();
        copy.file_id = Some(identity(&file).unwrap());
        let record = Location {
            schema: SCHEMA.into(),
            selection,
            destination,
            destination_id: identity(&directory).unwrap(),
            runtime_id: (1, 1),
            created_runtime: false,
            package: Some(copy),
        };
        let mut invalid = record.clone();
        invalid.destination_id = (123, 0);
        assert!(invalid.validate().is_err());
        let wide: Identity = (123, (1u128 << 100) | 17);
        assert_eq!(
            serde_json::from_slice::<Identity>(&serde_json::to_vec(&wide).unwrap()).unwrap(),
            wide
        );
        drop(file);
        drop(directory);
        fs::write(&path, b"changed content").unwrap();
        record.remove_package().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"changed content");
        fs::write(&path, b"installer package").unwrap();
        let original = parent.path().join("original");
        fs::rename(&record.destination, &original).unwrap();
        fs::create_dir(&record.destination).unwrap();
        fs::write(&path, b"other directory").unwrap();
        assert!(record.remove_package().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"other directory");
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&record.destination).unwrap();
        fs::rename(original, &record.destination).unwrap();
        record.remove_package().unwrap();
        assert!(!path.exists());
    }
}
