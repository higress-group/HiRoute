use super::*;
use crate::{StandaloneInstallRecordV1, StandaloneLayout};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Install,
    Uninstall,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    Committed,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Hardlink,
    Copy,
    Remove,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub destination: PathBuf,
    pub source: PathBuf,
    pub backup: PathBuf,
    pub old_sha256: Option<String>,
    pub new_sha256: Option<String>,
    pub kind: EntryKind,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub schema: String,
    pub operation: Operation,
    pub phase: Phase,
    pub old_marker: Option<StandaloneInstallRecordV1>,
    pub new_marker: StandaloneInstallRecordV1,
    pub old_definition: Option<Value>,
    pub new_definition: Value,
    pub files: Vec<Entry>,
}

/// Native manager effects stay at the Windows leaf; this core owns byte/recovery ordering.
pub trait ManagerHost {
    fn quiesce(&mut self, definitions: &[&Value]) -> io::Result<()>;
    fn register(&mut self, definition: &Value, previous: Option<&Value>) -> io::Result<()>;
    fn unregister(&mut self, definition: &Value) -> io::Result<()>;
}
impl Journal {
    pub fn path(layout: &StandaloneLayout) -> PathBuf {
        layout.data_root.join("service/install-journal.json")
    }
    pub fn read(layout: &StandaloneLayout) -> io::Result<Option<Self>> {
        #[cfg(windows)]
        if Self::path(layout).try_exists()? {
            hiroute_platform::windows::security::verify_owner_only(&Self::path(layout), false)?;
        }
        let journal: Option<Self> = read_json(&Self::path(layout), MAX_RECORD)?;
        if let Some(value) = &journal {
            value.validate(layout)?;
        }
        Ok(journal)
    }
    pub fn validate(&self, layout: &StandaloneLayout) -> io::Result<()> {
        let root = layout.home.join(".local/lib/hiroute");
        let definition = layout.data_root.join("service/windows-background.json");
        let skills: Vec<_> = [".agents", ".claude"]
            .iter()
            .map(|p| layout.home.join(p).join("skills/hiroute-management"))
            .collect();
        if self.schema != JOURNAL_SCHEMA
            || self.files.len() > 64
            || self.old_marker.is_some() != self.old_definition.is_some()
        {
            return Err(invalid());
        }
        for marker in self
            .old_marker
            .iter()
            .chain(std::iter::once(&self.new_marker))
        {
            marker.validate().map_err(|_| invalid())?;
            windows_machine(&marker.target)?;
            if marker.target != self.new_marker.target
                || marker.service_definition != definition
                || !within(&marker.install_root, &root)
                || marker.install_root == root
                || marker.install_root.parent() != Some(root.as_path())
                || marker.installed_skills != skills
                || marker.cpa_binary.as_ref()
                    != Some(&marker.install_root.join("libexec/cliproxyapi.exe"))
            {
                return Err(invalid());
            }
        }
        let mut destinations = BTreeSet::new();
        for item in &self.files {
            let allowed = item.destination == layout.home.join(".local/bin/hiroute.exe")
                || item.destination == layout.home.join(".local/bin/hirouted.exe")
                || item.destination == layout.data_root.join("package-manifest.json")
                || within(&item.destination, &layout.data_root.join("licenses"))
                || within(&item.destination, &layout.data_root.join("docs"))
                || skills
                    .iter()
                    .any(|p| item.destination == p.join("SKILL.md"));
            if !allowed
                || !destinations.insert(&item.destination)
                || !within(
                    &item.backup,
                    &layout.data_root.join("service/install-backups"),
                )
                || item.backup == layout.data_root.join("service/install-backups")
                || !std::iter::once(&self.new_marker)
                    .chain(self.old_marker.iter())
                    .any(|m| within(&item.source, &m.install_root))
                || item
                    .old_sha256
                    .iter()
                    .chain(item.new_sha256.iter())
                    .any(|s| !hex(s))
            {
                return Err(invalid());
            }
            regular(&item.destination)?;
            regular(&item.backup)?;
            regular(&item.source)?;
        }
        Ok(())
    }
    pub fn persist(&self, layout: &StandaloneLayout) -> io::Result<()> {
        self.validate(layout)?;
        write_json(&Self::path(layout), self)
    }
    pub fn apply_files(&self) -> io::Result<()> {
        // Admit every current digest before replacing the first entry.
        for entry in &self.files {
            entry.check_current()?;
        }
        for entry in &self.files {
            entry.replace(false)?;
        }
        Ok(())
    }
    pub fn commit(&mut self, layout: &StandaloneLayout) -> io::Result<()> {
        self.phase = Phase::Committed;
        self.persist(layout)
    }
    pub fn finish(&self, layout: &StandaloneLayout) -> io::Result<()> {
        if self.phase != Phase::Committed {
            return Err(invalid());
        }
        if self.operation == Operation::Uninstall {
            self.remove_metadata(layout)?;
        }
        self.finish_backups(layout)
    }
    fn finish_backups(&self, layout: &StandaloneLayout) -> io::Result<()> {
        for item in &self.files {
            if let Some(actual) = digest(&item.backup)? {
                if Some(&actual) != item.old_sha256.as_ref() {
                    return Err(invalid());
                }
                remove_owned(&item.backup)?;
            }
        }
        if Self::path(layout).try_exists()? {
            remove_owned(&Self::path(layout))?;
        }
        Ok(())
    }
    fn remove_metadata(&self, layout: &StandaloneLayout) -> io::Result<()> {
        let old = self.old_marker.as_ref().ok_or_else(invalid)?;
        let pairs = [
            (
                layout.marker_path.clone(),
                serde_json::to_value(old).map_err(|_| invalid())?,
            ),
            (
                old.service_definition.clone(),
                self.old_definition.clone().ok_or_else(invalid)?,
            ),
        ];
        for (path, expected) in pairs {
            if let Some(actual) = read_json::<Value>(&path, MAX_RECORD)? {
                if actual != expected {
                    return Err(invalid());
                }
                remove_owned(&path)?;
            }
        }
        Ok(())
    }
    /// Prepared install rolls back; prepared uninstall completes.
    pub fn recover(
        mut self,
        layout: &StandaloneLayout,
        host: &mut impl ManagerHost,
    ) -> io::Result<()> {
        self.validate(layout)?;
        if self.phase == Phase::Committed {
            return self.finish(layout);
        }
        for entry in &self.files {
            entry.check_current()?;
        }
        let mut definitions = vec![&self.new_definition];
        if let Some(old) = &self.old_definition {
            definitions.push(old);
        }
        host.quiesce(&definitions)?;
        if self.operation == Operation::Uninstall {
            host.unregister(self.old_definition.as_ref().ok_or_else(invalid)?)?;
            self.apply_files()?;
            self.commit(layout)?;
            return self.finish(layout);
        }
        if let Some(old) = &self.old_marker {
            let old_definition = self.old_definition.as_ref().ok_or_else(invalid)?;
            host.register(old_definition, Some(&self.new_definition))?;
            write_json(&old.service_definition, old_definition)?;
            write_json(&layout.marker_path, old)?;
        } else {
            host.unregister(&self.new_definition)?;
        }
        for entry in self.files.iter().rev() {
            entry.replace(true)?;
        }
        if self.old_marker.is_none() {
            for path in [&layout.marker_path, &self.new_marker.service_definition] {
                if path.try_exists()? {
                    let actual: Value = read_json(path, MAX_RECORD)?.ok_or_else(invalid)?;
                    let expected = if path == &layout.marker_path {
                        serde_json::to_value(&self.new_marker).map_err(|_| invalid())?
                    } else {
                        self.new_definition.clone()
                    };
                    if actual != expected {
                        return Err(invalid());
                    }
                    remove_owned(path)?;
                }
            }
        }
        self.finish_backups(layout)
    }
}
impl Entry {
    fn check_current(&self) -> io::Result<()> {
        if let Some(actual) = digest(&self.destination)?
            && Some(&actual) != self.old_sha256.as_ref()
            && Some(&actual) != self.new_sha256.as_ref()
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn replace(&self, rollback: bool) -> io::Result<()> {
        self.check_current()?;
        let expected = if rollback {
            &self.old_sha256
        } else {
            &self.new_sha256
        };
        if expected.is_none() {
            if self.destination.try_exists()? {
                remove_owned(&self.destination)?;
            }
            return Ok(());
        }
        let source = if rollback { &self.backup } else { &self.source };
        if digest(source)?.as_ref() != expected.as_ref() {
            return Err(invalid());
        }
        let stage = sibling(&self.destination)?;
        if self.kind == EntryKind::Hardlink {
            fs::hard_link(source, &stage)?;
        } else {
            copy_new(source, &stage)?;
        }
        let result = hiroute_platform::durable::rename(&stage, &self.destination);
        if stage.try_exists()? {
            remove_owned(&stage)?;
        }
        result
    }
}
/// Plan ownership first, then snapshot admitted entries. `source` already belongs to a verified package.
pub fn plan(
    entries: Vec<(PathBuf, PathBuf, EntryKind, String)>,
    layout: &StandaloneLayout,
    old: Option<&Package>,
    uninstall: bool,
) -> io::Result<Vec<Entry>> {
    let backup = layout
        .data_root
        .join("service/install-backups")
        .join(nonce()?);
    let mut result = Vec::new();
    for (index, (destination, source, kind, name)) in entries.into_iter().enumerate() {
        let actual = digest(&destination)?;
        if let Some(actual) = &actual {
            let owned = if name == "__package_manifest__" {
                read_json::<Value>(&destination, 1024 * 1024)?
                    == old
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|_| invalid())?
            } else {
                old.and_then(|p| p.files.get(&name))
                    .is_some_and(|facts| &facts.sha256 == actual)
            };
            if !owned {
                return Err(invalid());
            }
        }
        let new = if uninstall {
            None
        } else {
            Some(digest(&source)?.ok_or_else(invalid)?)
        };
        result.push(Entry {
            destination,
            source,
            backup: backup.join(index.to_string()),
            old_sha256: actual,
            new_sha256: new,
            kind,
        });
    }
    directory(&backup)?;
    for item in &result {
        directory(item.destination.parent().ok_or_else(invalid)?)?;
        if item.old_sha256.is_some() {
            if item.kind == EntryKind::Hardlink {
                fs::hard_link(&item.destination, &item.backup)?;
            } else {
                copy_new(&item.destination, &item.backup)?;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
