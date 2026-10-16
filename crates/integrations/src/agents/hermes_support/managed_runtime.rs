//! Read the dependency generation selected by native Hermes PM; never enumerate old venvs.
use super::{bounded_text, python_package_paths};
use hiroute_domain::CanonicalDigest;
use std::path::{Path, PathBuf};

/// `None` retains direct-interpreter inspection. An invalid PM selection has no SDKs.
pub(super) fn sdk_paths(project: &Path, interpreters: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let source = bounded_text(&project.join("pm/environments.py"), 2 * 1024 * 1024)?;
    if !source.contains("def install_key(") || !source.contains("def _recorded_venv(") {
        return None;
    }
    Some(selected_paths(project, interpreters).unwrap_or_default())
}

fn selected_paths(project: &Path, interpreters: &[PathBuf]) -> Option<Vec<PathBuf>> {
    // The native launcher binds an owning tool store. A daemon's HOME or a different
    // profile must not supply dependency facts for this installation.
    let python = interpreters.iter().take(8).find(|python| {
        python
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .is_some_and(|store| store.file_name().is_some_and(|name| name == "tools"))
    })?;
    let bin = python.parent()?;
    let owner = bin.parent()?.parent()?.parent()?;
    let canonical = project.canonicalize().ok()?;
    let key = CanonicalDigest::of_bytes(canonical.to_str()?.as_bytes());
    let state = owner.join("installs").join(&key.as_str()[7..23]);
    let text = bounded_text(&state.join("facts.json"), 128 * 1024)?;
    let facts: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    if facts["schema"].as_u64() != Some(1) {
        return None;
    }
    let selected = Path::new(facts["packages"]["venv"]["environment"].as_str()?);
    if !selected.is_absolute() {
        return None;
    }
    let selected = selected.canonicalize().ok()?;
    if !selected.starts_with(state.join("environments").canonicalize().ok()?) {
        return None;
    }
    let config = bounded_text(&selected.join("pyvenv.cfg"), 64 * 1024)?;
    let home = config.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == "home").then(|| PathBuf::from(value.trim()))
    })?;
    if home.canonicalize().ok()? != bin.canonicalize().ok()? {
        return None;
    }
    Some(python_package_paths(&[selected.join("bin/python")]))
}
