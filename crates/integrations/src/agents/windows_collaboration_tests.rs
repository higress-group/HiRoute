//! Native main-client preflight through the actual production CollaborationProbe.
//! This proves native Skill + public CLI execution, not worker delegation or account OAuth.
use super::*;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

struct QoderAccount {
    context: QoderNativeContext,
    directories: Vec<(PathBuf, File)>,
}

impl QoderAccount {
    fn selected() -> Result<Self, &'static str> {
        use hiroute_platform::windows::security;
        let path = |variable| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| {
                    path.is_absolute()
                        && path.to_str().is_some_and(|text| !text.contains('\0'))
                        && !path
                            .components()
                            .any(|part| part == std::path::Component::ParentDir)
                })
                .ok_or("explicit-profile-selector")
        };
        let home = path("HIROUTE_WINDOWS_QODER_ACCOUNT_HOME")?;
        let config = path("HIROUTE_WINDOWS_QODER_ACCOUNT_CONFIG")?;
        let mut directories = Vec::new();
        for directory in [&home, &config] {
            directories.push((
                directory.clone(),
                security::open_owner_only(directory, true)
                    .map_err(|_| "private-profile-directory")?,
            ));
        }
        let auth = config.join(".auth");
        directories.push((
            auth.clone(),
            security::open_identity(&auth, true).map_err(|_| "auth-directory")?,
        ));
        for (name, limit) in [("user", 1024 * 1024), ("machine_id", 4096)] {
            let file = security::open_identity(&auth.join(name), false)
                .map_err(|_| "readable-official-auth-file")?;
            let metadata = file.metadata().map_err(|_| "official-auth-metadata")?;
            if !metadata.is_file()
                || metadata.len() == 0
                || metadata.len() > limit
                || security::file_link_count(&file).map_err(|_| "official-auth-links")? != 1
            {
                return Err("official-auth-file-shape");
            }
            // Opening GENERIC_READ proves access without reading/decrypting credentials.
            // Release this deny-delete file before Qoder's native atomic refresh.
        }
        let context = QoderNativeContext::from_selected(&home, Some(&config))
            .map_err(|_| "borrowed-native-context")?;
        let account = Self {
            context,
            directories,
        };
        account.verify()?;
        Ok(account)
    }

    fn verify(&self) -> Result<(), &'static str> {
        use hiroute_platform::windows::security;
        for (path, held) in &self.directories {
            let current =
                security::open_identity(path, true).map_err(|_| "profile-directory-identity")?;
            if security::file_identity(held).map_err(|_| "profile-directory-identity")?
                != security::file_identity(&current).map_err(|_| "profile-directory-identity")?
            {
                return Err("profile-directory-identity");
            }
        }
        Ok(())
    }
}

type SettingsReceipt = Option<(
    zeroize::Zeroizing<Vec<u8>>,
    hiroute_platform::file_security::WindowsPermissions,
    (u32, u64),
)>;

fn settings_receipt(context: &QoderNativeContext) -> Result<SettingsReceipt, &'static str> {
    use hiroute_platform::{file_security, windows::security};
    let path = context.config_root.join("settings.json");
    let file = match security::open_identity(&path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("native-settings-read"),
    };
    let permissions = file_security::capture(&file).map_err(|_| "native-settings-permissions")?;
    let identity = security::file_identity(&file).map_err(|_| "native-settings-identity")?;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "native-settings-read")?;
    if bytes.len() > 1024 * 1024 {
        return Err("native-settings-bound");
    }
    Ok(Some((bytes, permissions, identity)))
}

fn selected(variable: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .unwrap_or_else(|| panic!("{variable} must select the fixed actual native executable"))
}

#[test]
#[ignore = "fixed actual Codex and this candidate hiroute.exe; required CI selects exactly one case"]
fn actual_windows_codex_native_skill_executes_the_public_cli() {
    CodexNativeIngressProbe::bind()
        .unwrap()
        .run_collaboration(
            &selected("HIROUTE_WORKER_CODEX_BINARY"),
            &selected("HIROUTE_WORKER_CLI_BINARY"),
        )
        .unwrap();
}

#[test]
#[ignore = "explicit Claude authentication diagnostic; excluded from Windows acceptance"]
fn actual_windows_claude_native_authentication_without_tools() {
    ClaudeNativeIngressProbe::run(&selected("HIROUTE_WORKER_CLAUDE_BINARY")).unwrap();
}

#[test]
#[ignore = "fixed actual Claude and this candidate hiroute.exe; required CI selects exactly one case"]
fn actual_windows_claude_native_skill_executes_the_public_cli() {
    ClaudeNativeIngressProbe::run_collaboration(
        &selected("HIROUTE_WORKER_CLAUDE_BINARY"),
        &selected("HIROUTE_WORKER_CLI_BINARY"),
    )
    .unwrap();
}

#[test]
#[ignore = "fixed actual Qoder and this candidate hiroute.exe; absent native login fails rather than synthesizing an account"]
fn actual_windows_qoder_native_skill_executes_the_public_cli_without_synthesized_account() {
    let account =
        QoderAccount::selected().expect("official private Qoder account profile required");
    let context = &account.context;
    let settings_before = settings_receipt(context).unwrap();
    assert!(!context.skill_target().exists());
    QoderCollaborationProbe::run(
        &selected("HIROUTE_WORKER_QODER_BINARY"),
        context,
        &selected("HIROUTE_WORKER_CLI_BINARY"),
        QoderCollaborationProbeTarget::PreinstallCapability,
    )
    .unwrap();
    account.verify().unwrap();
    // The test borrows the dedicated account. It owns neither credentials nor settings.
    assert!(!context.skill_target().exists());
    assert!(
        settings_receipt(context).unwrap() == settings_before,
        "official login settings must retain bytes, native permissions and file identity"
    );
}
