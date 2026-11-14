//! Windows DSH lookup and the standalone npm launcher form.
use super::{
    executable::{self, ProbeFailure},
    filesystem::dsh_native::dsh_observation,
    managed_launch::windows_executable_path,
    windows_runtime::{self, declared_npm_bin},
};
use hiroute_domain::CanonicalDigest;
use hiroute_platform::windows::{launcher, security};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) const PACKAGE: &str = "@deepseek-ai/dsh";

const NPM_SETUP: &[&str] = &[
    "@echo off",
    "goto start",
    ":find_dp0",
    "set dp0=%~dp0",
    "exit /b",
    ":start",
    "setlocal",
    "call :find_dp0",
    r#"if exist "%dp0%\node.exe" ("#,
    r#"set "_prog=%dp0%\node.exe""#,
    ") else (",
    r#"set "_prog=node""#,
];
const NPM_RUN: &str = r#"endlocal & goto #_undefined_# 2>nul || title %comspec% & "#;
const NPM_PATHEXT: &str = "set pathext=%pathext:;.js;=;%";

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

/// Bind the launcher and the files that supply its actual program and entry.
/// npm and Node updates can replace these files without rewriting the shim.
pub(in crate::agents) struct DshInvocation {
    pub(crate) program: PathBuf,
    pub(crate) entry: Option<PathBuf>,
    pub(crate) dependency: CanonicalDigest,
}

pub(in crate::agents) fn invocation(launcher: &Path) -> io::Result<DshInvocation> {
    use std::os::windows::fs::MetadataExt;

    let search = std::env::var_os("PATH");
    let (program, entry) = windows_runtime::command_invocation(launcher, search.as_deref())?;
    let paths = std::iter::once(launcher)
        .chain((launcher != program).then_some(program.as_path()))
        .chain(entry.as_deref());
    let dependencies = paths
        .map(|path| {
            let file = security::open_identity(path, false)?;
            let meta = file.metadata()?;
            Ok::<_, io::Error>((
                path,
                security::file_identity(&file)?,
                meta.file_size(),
                meta.creation_time(),
                meta.last_write_time(),
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let dependency = CanonicalDigest::of(&dependencies).map_err(io::Error::other)?;
    Ok(DshInvocation {
        program,
        entry,
        dependency,
    })
}

/// Recognize the simple Node launcher and npm's generated Node launcher.
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
    let lines = script
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let (setup, command) = lines.split_at(lines.len().saturating_sub(1));
    let [command] = command else {
        return Err(invalid());
    };
    let npm_tail = npm_node_arguments(setup, command);
    let (node, tail) = if let Some(tail) = npm_tail {
        ("node", tail)
    } else {
        if !setup.is_empty() && !matches!(setup, [echo] if echo.eq_ignore_ascii_case("@echo off")) {
            return Err(invalid());
        }
        let command = command.strip_prefix('@').unwrap_or(command);
        quoted_argument(command)
            .or_else(|| command.find([' ', '\t']).map(|end| command.split_at(end)))
            .ok_or_else(invalid)?
    };
    let (entry, tail) = quoted_argument(
        tail.strip_prefix([' ', '\t'])
            .ok_or_else(invalid)?
            .trim_start(),
    )
    .ok_or_else(invalid)?;
    if tail.strip_prefix([' ', '\t']).map(str::trim) != Some("%*") {
        return Err(invalid());
    }
    let entry = literal_path(shim, entry, npm_tail.is_some()).ok_or_else(invalid)?;
    if !entry
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    {
        return Err(invalid());
    }
    let entry = windows_executable_path(&entry)?;
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
    let node = if node.eq_ignore_ascii_case("node") || node.eq_ignore_ascii_case("node.exe") {
        windows_runtime::node_path(search, shim)?
    } else {
        let selected = literal_path(shim, node, false)
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("node.exe"))
            })
            .ok_or_else(invalid)?;
        windows_executable_path(&selected)?
    };
    Ok((node, entry))
}

fn npm_node_arguments<'a>(setup: &[&str], command: &'a str) -> Option<&'a str> {
    let (header, end) = setup.split_at_checked(NPM_SETUP.len())?;
    if !header
        .iter()
        .zip(NPM_SETUP)
        .all(|(line, expected)| line.eq_ignore_ascii_case(expected))
    {
        return None;
    }
    let command = strip_ascii_prefix(command, NPM_RUN)?;
    let command = match end {
        [")"] => strip_ascii_prefix(command, NPM_PATHEXT)?.strip_prefix(" & ")?,
        [set, ")"] if set.eq_ignore_ascii_case(NPM_PATHEXT) => command,
        _ => return None,
    };
    strip_ascii_prefix(command, r#""%_prog%""#)
}

fn strip_ascii_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|part| part.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn literal_path(shim: &Path, value: &str, npm: bool) -> Option<PathBuf> {
    use std::path::Component;
    let expression = if npm { "%dp0%" } else { "%~dp0" };
    let suffix = strip_ascii_prefix(value, expression);
    let Some(suffix) = suffix else {
        let path = Path::new(value);
        return (!npm && !value.contains('%') && path.is_absolute()).then(|| path.to_owned());
    };
    let suffix = if npm {
        suffix.strip_prefix('\\')?
    } else {
        suffix
    };
    if suffix.is_empty() || suffix.contains('%') {
        return None;
    }
    let mut path = shim.parent()?.to_owned();
    // cmd-shim emits paths relative to the launcher, including parent
    // components for packages in a separate install tree.
    for component in Path::new(suffix).components() {
        match component {
            Component::Normal(name) => path.push(name),
            Component::CurDir => {}
            Component::ParentDir if path.pop() => {}
            _ => return None,
        }
    }
    Some(path)
}

fn quoted_argument(argument: &str) -> Option<(&str, &str)> {
    argument.strip_prefix('"')?.split_once('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_dsh_recognizes_both_npm_node_shim_templates() {
        let mut older = NPM_SETUP.to_vec();
        older.extend([NPM_PATHEXT, ")"]);
        let mut newer = NPM_SETUP.to_vec();
        newer.push(")");
        for (setup, middle) in [(older, ""), (newer, "set pathext=%pathext:;.js;=;% & ")] {
            let arguments = r#"  "%dp0%\..\packages\@deepseek-ai\dsh\entry.js" %*"#;
            let command = format!(r#"{NPM_RUN}{middle}"%_prog%"{arguments}"#);
            assert_eq!(npm_node_arguments(&setup, &command), Some(arguments));
            let mut extra = setup;
            extra.insert(0, "unrecognized line");
            assert!(npm_node_arguments(&extra, &command).is_none());
        }
        let shim = Path::new(r"C:\launcher\dsh.cmd");
        assert_eq!(
            literal_path(shim, r"%~dp0..\packages\entry.js", false),
            Some(PathBuf::from(r"C:\packages\entry.js"))
        );
        assert!(literal_path(shim, r"%OTHER%\entry.js", false).is_none());
    }
}
