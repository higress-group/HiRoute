//! The existing full-App inventory is shared by update installation and manual recovery.
use crate::DesktopRelease;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const COMPONENTS: [&str; 4] = ["hiroute-desktop", "hirouted", "hiroute", "cliproxyapi"];
const INVENTORY: &str = "Contents/Resources/installation.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopPackageIdentity {
    pub app_path: PathBuf,
    pub version: String,
    pub revision: String,
    pub architecture: String,
    pub distribution: String,
    pub inventory_sha256: String,
    pub binaries: BTreeMap<String, String>,
}

pub fn file_sha256(path: &Path) -> Result<String, &'static str> {
    let mut file = regular_file(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "PACKAGE_READ_FAILED")?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn regular_file(path: &Path) -> Result<fs::File, &'static str> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| "PACKAGE_READ_FAILED")?;
    if !file
        .metadata()
        .map_err(|_| "PACKAGE_READ_FAILED")?
        .is_file()
    {
        return Err("PACKAGE_FILE_INVALID");
    }
    Ok(file)
}
fn bounded_json(path: &Path) -> Result<serde_json::Value, &'static str> {
    let mut bytes = Vec::new();
    regular_file(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "PACKAGE_READ_FAILED")?;
    if bytes.len() > 1024 * 1024 {
        return Err("PACKAGE_INVENTORY_INVALID");
    }
    serde_json::from_slice(&bytes).map_err(|_| "PACKAGE_INVENTORY_INVALID")
}
fn text<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str, &'static str> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("PACKAGE_INVENTORY_INVALID")
}
fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn inventory(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "PACKAGE_READ_FAILED")?;
    if metadata.file_type().is_symlink() {
        return Err("PACKAGE_SYMLINK_DENIED");
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| "PACKAGE_READ_FAILED")? {
            let entry = entry.map_err(|_| "PACKAGE_READ_FAILED")?;
            inventory(root, &entry.path(), files)?;
        }
    } else if metadata.is_file() {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "PACKAGE_INVENTORY_INVALID")?;
        if relative
            .components()
            .any(|c| c == Component::Normal("_CodeSignature".as_ref()))
            || relative == Path::new("Contents/MacOS/hiroute-desktop")
            || relative == Path::new(INVENTORY)
        {
            return Ok(());
        }
        let name = relative
            .to_str()
            .ok_or("PACKAGE_INVENTORY_INVALID")?
            .to_owned();
        files.insert(name, file_sha256(path)?);
    } else {
        return Err("PACKAGE_FILE_INVALID");
    }
    Ok(())
}

/// Validates the producer's closed inventory and every component byte. No App executable is run.
/// macOS hosts additionally validate the code signature below before installing a new App.
pub fn read_desktop_package_identity(app: &Path) -> Result<DesktopPackageIdentity, &'static str> {
    if !app.is_absolute()
        || fs::symlink_metadata(app)
            .map_err(|_| "PACKAGE_READ_FAILED")?
            .file_type()
            .is_symlink()
    {
        return Err("PACKAGE_PATH_INVALID");
    }
    let app = fs::canonicalize(app).map_err(|_| "PACKAGE_PATH_INVALID")?;
    let value = bounded_json(&app.join(INVENTORY))?;
    let version = text(&value, "version")?;
    semver::Version::parse(version).map_err(|_| "PACKAGE_VERSION_INVALID")?;
    let revision = text(&value, "revision")?;
    if revision.len() != 40
        || !revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("PACKAGE_REVISION_INVALID");
    }
    let architecture = text(&value, "architecture")?;
    let target = match architecture {
        "arm64" => "aarch64-apple-darwin",
        "x86_64" => "x86_64-apple-darwin",
        _ => return Err("PACKAGE_PLATFORM_INVALID"),
    };
    let distribution = text(&value, "distribution")?;
    if text(&value, "target")? != target
        || !matches!(distribution, "controlled-trial" | "developer-id")
    {
        return Err("PACKAGE_PLATFORM_INVALID");
    }
    let expected: BTreeMap<String, String> =
        serde_json::from_value(value["files"].clone()).map_err(|_| "PACKAGE_INVENTORY_INVALID")?;
    if expected.is_empty()
        || expected.len() > 10000
        || expected.iter().any(|(p, d)| {
            let path = Path::new(p);
            !sha(d)
                || path.is_absolute()
                || !p.starts_with("Contents/")
                || path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
        })
    {
        return Err("PACKAGE_INVENTORY_INVALID");
    }
    let mut actual = BTreeMap::new();
    inventory(&app, &app, &mut actual)?;
    if actual != expected {
        return Err("PACKAGE_INVENTORY_MISMATCH");
    }
    let binary_manifest = value["binaries"]
        .as_object()
        .ok_or("PACKAGE_COMPONENT_INVALID")?;
    if binary_manifest.len() != COMPONENTS.len() {
        return Err("PACKAGE_COMPONENT_INVALID");
    }
    let mut binaries = BTreeMap::new();
    for name in COMPONENTS {
        let path = app.join("Contents/MacOS").join(name);
        let record = binary_manifest
            .get(name)
            .ok_or("PACKAGE_COMPONENT_INVALID")?;
        let hash = file_sha256(&path)?;
        if record["architecture"] != architecture
            || (name != "hiroute-desktop"
                && (record["sha256"] != hash
                    || record["size"].as_u64()
                        != Some(
                            regular_file(&path)?
                                .metadata()
                                .map_err(|_| "PACKAGE_READ_FAILED")?
                                .len(),
                        )))
        {
            return Err("PACKAGE_COMPONENT_MISMATCH");
        }
        binaries.insert(name.into(), hash);
    }
    let cpa = &value["cpa"]["artifacts"];
    if cpa.as_array().is_none_or(|a| a.len() != 1) || cpa[0]["sha256"] != binaries["cliproxyapi"] {
        return Err("PACKAGE_COMPONENT_MISMATCH");
    }
    Ok(DesktopPackageIdentity {
        app_path: app.clone(),
        version: version.into(),
        revision: revision.into(),
        architecture: architecture.into(),
        distribution: distribution.into(),
        inventory_sha256: file_sha256(&app.join(INVENTORY))?,
        binaries,
    })
}

#[cfg(target_os = "macos")]
fn tool(program: &str, args: &[&std::ffi::OsStr]) -> Result<String, &'static str> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|_| "PACKAGE_VERIFY_FAILED")?;
    if !output.status.success() {
        return Err("PACKAGE_VERIFY_FAILED");
    }
    let mut bytes = output.stdout;
    bytes.extend(output.stderr);
    String::from_utf8(bytes).map_err(|_| "PACKAGE_VERIFY_FAILED")
}

#[cfg(target_os = "macos")]
pub fn verify_desktop_app(
    app: &Path,
    bundle_id: &str,
    release: Option<&DesktopRelease>,
    expected_team: Option<&str>,
) -> Result<DesktopPackageIdentity, &'static str> {
    let identity = read_desktop_package_identity(app)?;
    if let Some(release) = release {
        let distribution = if release.distribution == "self-signed" {
            "controlled-trial"
        } else {
            "developer-id"
        };
        if identity.version != release.version
            || !identity.revision.starts_with(release.revision_prefix()?)
            || identity.architecture != release.architecture
            || identity.distribution != distribution
        {
            return Err("UPGRADE_PACKAGE_IDENTITY_MISMATCH");
        }
    }
    use std::ffi::OsStr as O;
    let info = identity.app_path.join("Contents/Info.plist");
    let read_info = |key: &str| {
        tool(
            "/usr/bin/plutil",
            &[
                O::new("-extract"),
                O::new(key),
                O::new("raw"),
                O::new("-o"),
                O::new("-"),
                info.as_os_str(),
            ],
        )
    };
    if read_info("CFBundleIdentifier")?.trim() != bundle_id
        || read_info("CFBundleShortVersionString")?.trim() != identity.version
        || read_info("LSMinimumSystemVersion")?.trim() != "15.0"
    {
        return Err("PACKAGE_BUNDLE_IDENTITY_MISMATCH");
    }
    for name in COMPONENTS {
        let binary = identity.app_path.join("Contents/MacOS").join(name);
        tool(
            "/usr/bin/codesign",
            &[O::new("--verify"), O::new("--strict"), binary.as_os_str()],
        )?;
        if tool("/usr/bin/lipo", &[O::new("-archs"), binary.as_os_str()])?.trim()
            != identity.architecture
        {
            return Err("PACKAGE_PLATFORM_INVALID");
        }
    }
    tool(
        "/usr/bin/codesign",
        &[
            O::new("--verify"),
            O::new("--strict"),
            identity.app_path.as_os_str(),
        ],
    )?;
    let signature = tool(
        "/usr/bin/codesign",
        &[
            O::new("--display"),
            O::new("--verbose=4"),
            identity.app_path.as_os_str(),
        ],
    )?;
    if identity.distribution == "controlled-trial" {
        if !signature.lines().any(|l| l == "Signature=adhoc") {
            return Err("PACKAGE_SIGNATURE_IDENTITY_MISMATCH");
        }
    } else {
        let team = expected_team.ok_or("UPGRADE_SIGNATURE_IDENTITY_UNCONFIGURED")?;
        if !signature
            .lines()
            .any(|l| l == format!("TeamIdentifier={team}"))
            || !signature.contains("Authority=Developer ID Application:")
        {
            return Err("PACKAGE_SIGNATURE_IDENTITY_MISMATCH");
        }
        tool(
            "/usr/sbin/spctl",
            &[
                O::new("--assess"),
                O::new("--type"),
                O::new("execute"),
                identity.app_path.as_os_str(),
            ],
        )?;
    }
    Ok(identity)
}

#[cfg(not(target_os = "macos"))]
pub fn verify_desktop_app(
    _: &Path,
    _: &str,
    _: Option<&DesktopRelease>,
    _: Option<&str>,
) -> Result<DesktopPackageIdentity, &'static str> {
    Err("UPGRADE_PLATFORM_UNSUPPORTED")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path();
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        let mut binaries = serde_json::Map::new();
        for name in COMPONENTS {
            let path = app.join("Contents/MacOS").join(name);
            fs::write(&path, name.as_bytes()).unwrap();
            let mut value = serde_json::json!({"architecture":"arm64"});
            if name != "hiroute-desktop" {
                value["sha256"] = file_sha256(&path).unwrap().into();
                value["size"] = (name.len() as u64).into();
            }
            binaries.insert(name.into(), value);
        }
        fs::write(app.join("Contents/Info.plist"), b"fixture bundle identity").unwrap();
        let mut files = BTreeMap::new();
        inventory(app, app, &mut files).unwrap();
        let cpa_sha = binaries["cliproxyapi"]["sha256"].clone();
        let value = serde_json::json!({"version":"0.1.0","revision":"a".repeat(40),
            "architecture":"arm64","target":"aarch64-apple-darwin","distribution":"controlled-trial",
            "binaries":binaries,"files":files,"cpa":{"artifacts":[{"sha256":cpa_sha}]}});
        fs::write(app.join(INVENTORY), serde_json::to_vec(&value).unwrap()).unwrap();
        directory
    }
    #[test]
    fn upgrade_package_inventory_rejects_changed_and_extra_components() {
        let good = package();
        let identity = read_desktop_package_identity(good.path()).unwrap();
        assert_eq!(identity.binaries.len(), 4);
        fs::write(
            good.path().join("Contents/MacOS/hirouted"),
            b"modified daemon",
        )
        .unwrap();
        assert_eq!(
            read_desktop_package_identity(good.path()).unwrap_err(),
            "PACKAGE_INVENTORY_MISMATCH"
        );
        let extra = package();
        fs::write(
            extra.path().join("Contents/MacOS/other-service"),
            b"extra executable",
        )
        .unwrap();
        assert_eq!(
            read_desktop_package_identity(extra.path()).unwrap_err(),
            "PACKAGE_INVENTORY_MISMATCH"
        );
    }
    #[test]
    #[cfg(unix)]
    fn upgrade_package_inventory_rejects_links_and_escaping_manifest_paths() {
        let linked = package();
        let daemon = linked.path().join("Contents/MacOS/hirouted");
        fs::remove_file(&daemon).unwrap();
        std::os::unix::fs::symlink("hiroute", &daemon).unwrap();
        assert_eq!(
            read_desktop_package_identity(linked.path()).unwrap_err(),
            "PACKAGE_SYMLINK_DENIED"
        );
        let invalid = package();
        let path = invalid.path().join(INVENTORY);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["files"]["Contents/../../outside"] = "a".repeat(64).into();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            read_desktop_package_identity(invalid.path()).unwrap_err(),
            "PACKAGE_INVENTORY_INVALID"
        );
    }
}
