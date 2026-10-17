use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub sha256: String,
    pub size: u64,
    pub mode: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    filename: String,
    sha256: String,
    size: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cpa {
    version: String,
    sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema: String,
    pub version: String,
    pub revision: String,
    pub target: String,
    archive: Archive,
    pub files: BTreeMap<String, PackageFile>,
    cpa: Cpa,
    distribution: String,
}
impl Package {
    pub fn directory_name(&self) -> String {
        format!(
            "{}-{}",
            self.version,
            &self.revision[..self.revision.len().min(12)]
        )
    }
    pub fn read(directory: &Path) -> io::Result<Self> {
        let value: Self = read_json(&directory.join("package-manifest.json"), 1024 * 1024)?
            .ok_or_else(invalid)?;
        value.validate()?;
        value.verify_directory(directory)?;
        Ok(value)
    }
    pub fn validate(&self) -> io::Result<()> {
        windows_machine(&self.target)?;
        let base: BTreeSet<&str> = [
            "bin/hiroute.exe",
            "bin/hirouted.exe",
            "libexec/cliproxyapi.exe",
            "licenses/CLIProxyAPI-LICENSE",
            "licenses/HiRoute-LICENSE",
            "licenses/THIRD-PARTY-LICENSES.txt",
            "licenses/third-party-licenses.json",
            "skills/hiroute-management/SKILL.md",
            "docs/standalone-cli.md",
        ]
        .into_iter()
        .collect();
        if self.schema != "hiroute.standalone-package/v1"
            || !portable(&self.version, 64)
            || !portable(&self.revision, 128)
            || self.distribution != "integration-candidate"
            || self.archive.filename.len() > 240
            || !self.archive.filename.ends_with(".tar.gz")
            || self.archive.filename.contains(['/', '\\'])
            || !hex(&self.archive.sha256)
            || self.archive.size == 0
            || self.archive.size > 1024 * 1024 * 1024
            || self.cpa.version.is_empty()
            || self.cpa.version.len() > 128
            || self.cpa.version.chars().any(char::is_control)
            || !hex(&self.cpa.sha256)
            || self.files.len() > base.len() + 2048
            || base.iter().any(|name| !self.files.contains_key(*name))
        {
            return Err(invalid());
        }
        let mut total = 0_u64;
        let mut console = 0;
        for (name, file) in &self.files {
            if !base.contains(name.as_str()) {
                if !name.starts_with("console/")
                    || name.len() > 240
                    || name.split('/').any(|part| {
                        !portable(part, 240)
                            && !(part.contains('@')
                                && part.bytes().all(|b| {
                                    b.is_ascii_alphanumeric()
                                        || matches!(b, b'.' | b'_' | b'-' | b'@')
                                }))
                    })
                    || file.size > 16 * 1024 * 1024
                {
                    return Err(invalid());
                }
                console += file.size;
            }
            if !hex(&file.sha256)
                || file.mode
                    != if name.starts_with("bin/") || name.starts_with("libexec/") {
                        "executable"
                    } else {
                        "read_only"
                    }
            {
                return Err(invalid());
            }
            total = total.checked_add(file.size).ok_or_else(invalid)?;
        }
        if total > 1024 * 1024 * 1024
            || console > 64 * 1024 * 1024
            || (console > 0 && !self.files.contains_key("console/index.html"))
            || self.files["libexec/cliproxyapi.exe"].sha256 != self.cpa.sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn relative(name: &str) -> &str {
        name.strip_prefix("bin/").unwrap_or(name)
    }
    pub fn verify_directory(&self, directory: &Path) -> io::Result<()> {
        let mut expected: BTreeSet<PathBuf> = self
            .files
            .keys()
            .map(|name| PathBuf::from(Self::relative(name)))
            .collect();
        expected.insert("package-manifest.json".into());
        if walk(directory, directory)? != expected {
            return Err(invalid());
        }
        for (name, facts) in &self.files {
            let path = directory.join(Self::relative(name));
            let file = regular(&path)?.ok_or_else(invalid)?;
            if file.metadata()?.len() != facts.size
                || digest(&path)?.as_deref() != Some(&facts.sha256)
            {
                return Err(invalid());
            }
            if facts.mode == "executable" {
                pe_machine(file, windows_machine(&self.target)?)?;
            }
        }
        Ok(())
    }
    pub fn cpa_digest(&self) -> &str {
        &self.cpa.sha256
    }
    /// The current producer serializes one closed manifest shape.
    pub fn canonical(&self) -> io::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| invalid())
    }
}
fn walk(root: &Path, current: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err(invalid());
        }
        if metadata.is_dir() {
            files.extend(walk(root, &path)?);
        } else {
            let _ = regular(&path)?.ok_or_else(invalid)?;
            files.insert(path.strip_prefix(root).map_err(|_| invalid())?.to_owned());
        }
        if files.len() > 2060 {
            return Err(invalid());
        }
    }
    Ok(files)
}
fn pe_machine(mut file: fs::File, machine: u16) -> io::Result<()> {
    let mut dos = [0; 64];
    file.read_exact(&mut dos)?;
    let offset = u32::from_le_bytes(dos[60..64].try_into().map_err(|_| invalid())?);
    if &dos[..2] != b"MZ" || !(64..=16 * 1024 * 1024).contains(&offset) {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut pe = [0; 6];
    file.read_exact(&mut pe)?;
    if &pe[..4] != b"PE\0\0" || u16::from_le_bytes([pe[4], pe[5]]) != machine {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
