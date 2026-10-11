//! Bind synthetic Agent inputs to the child's HOME before adding fixture-specific overrides.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

pub fn configure(command: &mut Command, home: &Path) {
    // The validation caller deliberately isolates these roots too. Inheriting its empty roots
    // would hide this child's synthetic credentials, while inheriting a real caller's roots
    // would let discovery leave the fixture. Keep the parent process environment unchanged.
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if [
            "CODEX_",
            "CLAUDE_",
            "ANTHROPIC_",
            "OPENAI_",
            "HIROUTE_",
            "QODER_",
            "PI_",
            "DSH_",
            "XDG_",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
        {
            command.env_remove(name);
        }
    }
    // Create each intermediate directory explicitly so a permissive caller umask cannot
    // make a native source or startup cache fail the production private-directory contract.
    for suffix in [
        "",
        ".codex",
        ".claude",
        ".qoder",
        ".pi",
        ".pi/agent",
        ".dsh",
        ".config",
        ".cache",
        ".local",
        ".local/share",
        ".local/state",
        ".run",
        ".tmp",
    ] {
        let directory = home.join(suffix);
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    command
        .env("HOME", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", home.join(".claude"))
        .env("QODER_CONFIG_DIR", home.join(".qoder"))
        .env("PI_CODING_AGENT_DIR", home.join(".pi/agent"))
        .env("DSH_HOME", home.join(".dsh"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_RUNTIME_DIR", home.join(".run"))
        .env("TMPDIR", home.join(".tmp"));
}
