//! Actual native controls. These mutate only fresh test-owned ACLs, never a
//! profile, token, shell policy or machine configuration. Output stays in RAM.
use super::*;
use hiroute_platform::{file_security, windows::security};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::Command,
    time::{Duration, Instant},
};

const BOUND: u64 = 16 * 1024;

#[derive(Clone, Copy)]
enum Shell {
    WindowsPowerShell,
    Pwsh,
}

impl Shell {
    fn program(self) -> PathBuf {
        match self {
            Self::WindowsPowerShell => hiroute_platform::windows::launcher::system_executable(
                Path::new("WindowsPowerShell/v1.0/powershell.exe"),
            )
            .unwrap(),
            Self::Pwsh => std::env::split_paths(&std::env::var_os("PATH").expect("native PATH"))
                .filter(|root| root.is_absolute())
                .map(|root| root.join("pwsh.exe"))
                .find(|path| security::open_identity(path, false).is_ok())
                .expect("the actual native pwsh executable is required; no skip or PS5 fallback"),
        }
    }
}

struct Output {
    code: i32,
    stdout: String,
    stderr_empty: bool,
}

fn run(mut command: Command) -> Output {
    use hiroute_platform::windows::job::Stream;
    let root = super::super::super::probe_files::temporary("hiroute-readonly-capture-").unwrap();
    let stdout = super::super::super::probe_files::file(&root.path().join("stdout")).unwrap();
    let stderr = super::super::super::probe_files::file(&root.path().join("stderr")).unwrap();
    command
        .env_clear()
        .env("HOME", root.path())
        .current_dir(root.path());
    let child = super::super::super::windows_runtime::spawn(
        &command,
        false,
        Stream::Null,
        Stream::File(&stdout),
        Stream::File(&stderr),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let code = loop {
        assert!(
            stdout.metadata().unwrap().len() <= BOUND && stderr.metadata().unwrap().len() <= BOUND,
            "native readonly capture byte bound"
        );
        if let Some(code) = child.exit_code().unwrap()
            && child.empty().unwrap()
        {
            break code;
        }
        assert!(
            Instant::now() < deadline,
            "native readonly control must exit and empty its actual Job"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    // No forced stop is accepted. Only after exact root exit and natural Job
    // emptiness can the shared capture file offsets be moved or output inspected.
    assert!(child.empty().unwrap());
    let read = |file: &File| {
        let mut file = file.try_clone().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.take(BOUND + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() as u64 <= BOUND);
        bytes
    };
    let output = String::from_utf8(read(&stdout)).expect("fixed native readonly output is UTF-8");
    let error = read(&stderr);
    eprintln!(
        "native readonly filesystem control: root_exit={code} natural_job_empty=true stderr_empty={}",
        error.is_empty()
    );
    Output {
        code,
        stdout: output,
        stderr_empty: error.is_empty(),
    }
}

fn execute(shell: Shell, script: &str) -> Output {
    let program = shell.program();
    let held = security::open_identity(&program, false).unwrap();
    let identity = security::file_identity(&held).unwrap();
    let mut command = Command::new(&program);
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let result = run(command);
    assert_eq!(
        security::file_identity(&security::open_identity(&program, false).unwrap()).unwrap(),
        identity
    );
    result
}

fn script(mode: &str, control: &ReadOnlyControl) -> String {
    match mode {
        "FullLanguage" => control.command().to_owned(),
        "ConstrainedLanguage" => format!(
            "$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'; {}",
            control.command()
        ),
        _ => unreachable!("only fixed native test modes"),
    }
}

fn writable(shell: Shell) {
    let root = super::super::super::probe_files::temporary("hiroute-readonly-writable-").unwrap();
    let held = security::open_owner_only(root.path(), true).unwrap();
    let before = file_security::capture(&held).unwrap();
    for mode in ["FullLanguage", "ConstrainedLanguage"] {
        let mut control = ReadOnlyControl::new(root.path(), mode).unwrap();
        let output = execute(shell, &script(mode, &control));
        assert_eq!(output.code, 1);
        assert!(output.stderr_empty);
        let facts = control.receipt_facts(&output.stdout);
        assert_eq!(facts.attempt_matches, 1);
        assert_eq!(facts.denial_matches, 0);
        assert!(
            matching_lines(&output.stdout, "hiroute-readonly-write-succeeded")
                .next()
                .is_some()
        );
        let mut file = security::open_identity(&control.path, false).unwrap();
        assert_eq!(security::file_link_count(&file).unwrap(), 1);
        let mut contents = Vec::new();
        (&mut file).take(128).read_to_end(&mut contents).unwrap();
        assert!(
            contents == b"hiroute-readonly-unexpected-write",
            "the actual filesystem write must occur"
        );
        assert!(control.confirm(&output.stdout).is_err());
        assert!(!control.verified() && control.verify_after_stop().is_err());
        drop(file);
        std::fs::remove_file(&control.path).unwrap();
    }
    assert!(file_security::capture(&held).unwrap() == before);
}

fn denied(shell: Shell) {
    let root = super::super::super::probe_files::temporary("hiroute-readonly-denied-").unwrap();
    let held = security::open_owner_only(root.path(), true).unwrap();
    let identity = security::file_identity(&held).unwrap();
    // Atomic native creation and SetSecurityInfo do not promise the same DACL
    // inheritance control bits. Establish the fixture's restoration baseline
    // using its actual restore operation before adding the deny ACE. This is a
    // fresh explicitly current-user-owned object, never an adopted directory.
    security::set_owner_only(root.path(), true).unwrap();
    assert_eq!(
        security::file_identity(&security::open_owner_only(root.path(), true).unwrap()).unwrap(),
        identity
    );
    let before = file_security::capture(&held).unwrap();
    security::set_owner_only(root.path(), true).unwrap();
    assert!(
        file_security::capture(&held).unwrap() == before,
        "the actual fixture restoration operation must be idempotent"
    );
    let owner = hiroute_platform::windows::background::current_sid_string().unwrap();
    let mut command = Command::new(
        hiroute_platform::windows::launcher::system_executable(Path::new("icacls.exe")).unwrap(),
    );
    command
        .arg(root.path())
        .arg("/deny")
        .arg(format!("*{owner}:(WD,AD)"));
    assert_eq!(
        run(command).code,
        0,
        "only this fresh owned directory gets a write-deny ACE"
    );
    assert_eq!(security::file_identity(&held).unwrap(), identity);
    assert!(file_security::capture(&held).unwrap() != before);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for mode in ["FullLanguage", "ConstrainedLanguage"] {
            let mut control = ReadOnlyControl::new(root.path(), mode).unwrap();
            let output = execute(shell, &script(mode, &control));
            let facts = control.receipt_facts(&output.stdout);
            // Numeric/class facts are closed; never print native error text.
            eprintln!(
                "native readonly filesystem receipt: mode={mode} root_exit={} facts={facts:?}",
                output.code
            );
            assert_eq!(output.code, 0);
            assert!(output.stderr_empty);
            assert_eq!(facts.attempt_matches, 1);
            assert_eq!(facts.denial_matches, 1);
            assert_eq!(facts.script.failure_matches, 0);
            assert_eq!(facts.pipeline.failure_matches, 0);
            assert_eq!(facts.pipeline.error_matches, 0);
            if mode == "ConstrainedLanguage" {
                assert_eq!(facts.pipeline.mode, LanguageMode::ConstrainedLanguage);
                assert!(unique_line(&output.stdout, &control.filesystem_denial).is_some());
            }
            control.confirm(&output.stdout).unwrap();
            control.verify_after_stop().unwrap();
        }
    }));
    // This is a new explicitly current-user-owned fixture. The existing setter
    // refuses another owner; no borrowed object or token default is changed.
    assert_eq!(
        security::file_identity(&security::open_identity(root.path(), true).unwrap()).unwrap(),
        identity
    );
    security::set_owner_only(root.path(), true).unwrap();
    assert!(
        file_security::capture(&held).unwrap() == before,
        "restore the exact owned native descriptor"
    );
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn language_error(shell: Shell) {
    let root = super::super::super::probe_files::temporary("hiroute-readonly-language-").unwrap();
    let mut control = ReadOnlyControl::new(root.path(), "language-negative").unwrap();
    // Execute the originally failing first Console invocation under real CLM.
    // It must not be translated into a filesystem UnauthorizedAccess receipt.
    let script = format!(
        "$ErrorActionPreference='Stop'; $ExecutionContext.SessionState.LanguageMode='ConstrainedLanguage'; '{}'; try {{ [Console]::WriteLine('not-a-file-write') }} catch {{ if($_.FullyQualifiedErrorId -ceq 'MethodInvocationNotSupportedInConstrainedLanguage') {{ '{}ConstrainedMethodInvocation' }}; exit 1 }}",
        control.pipeline_entered, control.pipeline_error_prefix
    );
    let output = execute(shell, &script);
    assert_eq!(output.code, 1);
    assert!(output.stderr_empty);
    let facts = control.receipt_facts(&output.stdout);
    assert_eq!(facts.pipeline.entered_matches, 1);
    assert_eq!(
        facts.pipeline.error_class,
        ScriptErrorClass::ConstrainedMethodInvocation
    );
    assert_eq!(facts.attempt_matches, 0);
    assert_eq!(facts.denial_matches, 0);
    require_absent(&control.path).unwrap();
    assert!(control.confirm(&output.stdout).is_err());
    assert!(!control.verified());
}

#[test]
fn readonly_windows_powershell_clm_writable_actual_write_is_rejected() {
    writable(Shell::WindowsPowerShell);
}
#[test]
fn readonly_windows_powershell_clm_acl_denial_is_accepted() {
    denied(Shell::WindowsPowerShell);
}
#[test]
fn readonly_windows_powershell_clm_method_error_cannot_authorize_denial() {
    language_error(Shell::WindowsPowerShell);
}
#[test]
fn readonly_windows_pwsh_clm_writable_actual_write_is_rejected() {
    writable(Shell::Pwsh);
}
#[test]
fn readonly_windows_pwsh_clm_acl_denial_is_accepted() {
    denied(Shell::Pwsh);
}
#[test]
fn readonly_windows_pwsh_clm_method_error_cannot_authorize_denial() {
    language_error(Shell::Pwsh);
}
