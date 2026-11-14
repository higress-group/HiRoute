//! Windows DSH lookup and the standalone npm launcher form.
use super::{
    executable::{self, ProbeFailure},
    filesystem::dsh_native::dsh_observation,
    managed_launch::windows_executable_path,
    windows_runtime::{self, declared_npm_bin},
};
use hiroute_platform::windows::{launcher, security};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) const PACKAGE: &str = "@deepseek-ai/dsh";

pub(super) fn find(
    name: &Path,
    version_diagnostic: bool,
) -> Result<Option<(PathBuf, String)>, ProbeFailure> {
    let search = std::env::var_os("PATH");
    let mut first_failure = None;
    for candidate in executable::candidates(name) {
        let outcome = match windows_executable_path(&candidate) {
            Ok(path) if windows_runtime::is_native(&path) || is_shim(&path) => {
                if version_diagnostic {
                    dsh_observation(path, true)
                } else {
                    let runnable = windows_runtime::command_invocation(&path, search.as_deref())
                        .is_ok_and(|(program, _)| launcher::is_executable_image(&program));
                    if runnable {
                        dsh_observation(path, false)
                    } else {
                        Err(ProbeFailure::NotExecutable)
                    }
                }
            }
            Ok(_) => Err(ProbeFailure::NotExecutable),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => Err(ProbeFailure::Unavailable),
        };
        match outcome {
            Ok(found) => return Ok(Some(found)),
            Err(reason) => {
                first_failure.get_or_insert(reason);
            }
        }
    }
    first_failure.map_or(Ok(None), Err)
}

pub(super) fn is_shim(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().eq_ignore_ascii_case(b"dsh.cmd"))
}

/// Recognize a single quoted Node invocation paired with the actual DSH package.
/// Run its program and entry as argv through the same held Job as a regular npm bin.
pub(super) fn standalone_shim(
    shim: &Path,
    search: Option<&std::ffi::OsStr>,
) -> io::Result<(PathBuf, PathBuf)> {
    const LIMIT: u64 = 8192;
    let file = security::open_identity(shim, false)?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "unsupported DSH npm launcher");
    if bytes.len() as u64 > LIMIT {
        return Err(invalid());
    }
    let script = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let mut lines = script
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let mut command = lines.next().ok_or_else(invalid)?;
    if command.eq_ignore_ascii_case("@echo off") {
        command = lines.next().ok_or_else(invalid)?;
    }
    if lines.next().is_some() {
        return Err(invalid());
    }
    let command = command.strip_prefix('@').unwrap_or(command);
    let (node, tail) = quoted_argument(command)
        .or_else(|| command.find([' ', '\t']).map(|end| command.split_at(end)))
        .ok_or_else(invalid)?;
    let (entry, tail) = quoted_argument(
        tail.strip_prefix([' ', '\t'])
            .ok_or_else(invalid)?
            .trim_start(),
    )
    .ok_or_else(invalid)?;
    if tail.strip_prefix([' ', '\t']).map(str::trim) != Some("%*")
        || node.contains('%')
        || entry.contains('%')
    {
        return Err(invalid());
    }
    let entry = Path::new(entry);
    if !entry.is_absolute()
        || !entry
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    {
        return Err(invalid());
    }
    let entry = windows_executable_path(entry)?;
    let root = entry
        .ancestors()
        .skip(1)
        .find(|directory| {
            directory
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("dsh"))
                && directory
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name.eq_ignore_ascii_case("@deepseek-ai"))
        })
        .ok_or_else(invalid)?;
    let declared = declared_npm_bin(root, PACKAGE, "dsh", |path| {
        security::open_identity(path, false).map(drop)
    })?;
    if declared != entry.canonicalize()? {
        return Err(invalid());
    }
    let selected = Path::new(node);
    let node = if node.eq_ignore_ascii_case("node") || node.eq_ignore_ascii_case("node.exe") {
        windows_runtime::node_path(search, &entry)?
    } else if selected.is_absolute()
        && selected
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("node.exe"))
    {
        windows_executable_path(selected)?
    } else {
        return Err(invalid());
    };
    Ok((node, entry))
}

fn quoted_argument(argument: &str) -> Option<(&str, &str)> {
    argument.strip_prefix('"')?.split_once('"')
}
