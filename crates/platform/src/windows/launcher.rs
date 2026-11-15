//! Token-free descriptor for the Codex adapter's early app-server startup.
//! The selected native object is verified before the exact managed argv is applied.
use super::{
    job::{Child, Launch, Stream},
    security,
};
use std::{
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const DESCRIPTOR_ENV: &str = "HIROUTE_NATIVE_LAUNCH_DESCRIPTOR";
const LIMIT: u64 = 64 * 1024;

/// Checks the image format without launching it. Windows still decides whether
/// the selected program can start when the caller attempts a real launch.
pub fn is_executable_image(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetBinaryTypeW;

    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut kind = 0;
    // SAFETY: path is a NUL-terminated wide string and kind is a live out parameter.
    unsafe { GetBinaryTypeW(path.as_ptr(), &mut kind) != 0 }
}

pub fn descriptor(binary: &Path, prefix: &[PathBuf], overrides: &[String]) -> io::Result<Vec<u8>> {
    let file = security::open_identity(binary, false)?;
    let (volume, index) = security::file_identity(&file)?;
    let mut entries = Vec::new();
    for path in prefix {
        if !path.is_absolute() {
            return Err(io::Error::other("launcher entry"));
        }
        let file = security::open_identity(path, false)?;
        let (volume, index) = security::file_identity(&file)?;
        entries.push(serde_json::json!({"path":path,"volume":volume,"index":index}));
    }
    let value = serde_json::json!({
        "schema": "hiroute.native-launch/v1", "binary": binary,
        "volume": volume, "index": index, "prefix": entries, "overrides": overrides,
    });
    let bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("descriptor bound"));
    }
    Ok(bytes)
}

pub fn run_from_environment() -> io::Result<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args != [OsString::from("app-server")] {
        return Err(io::Error::other("launcher arguments"));
    }
    let path = PathBuf::from(
        std::env::var_os(DESCRIPTOR_ENV).ok_or_else(|| io::Error::other("descriptor absent"))?,
    );
    let mut descriptor = security::open_owner_only(&path, false)?;
    if descriptor.metadata()?.len() > LIMIT {
        return Err(io::Error::other("descriptor bound"));
    }
    let mut bytes = Vec::new();
    (&mut descriptor).take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("descriptor bound"));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let invalid = || io::Error::other("invalid launch descriptor");
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 6 || value["schema"] != "hiroute.native-launch/v1" {
        return Err(invalid());
    }
    let binary = PathBuf::from(value["binary"].as_str().ok_or_else(invalid)?);
    let identity = (
        value["volume"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(invalid)?,
        value["index"].as_u64().ok_or_else(invalid)?,
    );
    // Keep the native file object open through CreateProcess to block replacement.
    let native = security::open_identity(&binary, false)?;
    if security::file_identity(&native)? != identity {
        return Err(invalid());
    }
    let mut argv = Vec::new();
    let mut entries = Vec::new();
    for entry in value["prefix"].as_array().ok_or_else(invalid)? {
        if entry.as_object().is_none_or(|entry| entry.len() != 3) {
            return Err(invalid());
        }
        let path = PathBuf::from(entry["path"].as_str().ok_or_else(invalid)?);
        let identity = (
            entry["volume"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(invalid)?,
            entry["index"].as_u64().ok_or_else(invalid)?,
        );
        let file = security::open_identity(&path, false)?;
        if security::file_identity(&file)? != identity {
            return Err(invalid());
        }
        entries.push(file);
        argv.push(path.into_os_string());
    }
    for override_value in value["overrides"].as_array().ok_or_else(invalid)? {
        let argument = override_value
            .as_str()
            .filter(|value| !value.contains('\0'))
            .ok_or_else(invalid)?;
        argv.extend([OsString::from("-c"), OsString::from(argument)]);
    }
    argv.push(OsString::from("app-server"));
    // Only the controlled parent supplied this environment. Do not re-read secrets from files.
    let environment: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            !key.as_encoded_bytes()
                .eq_ignore_ascii_case(DESCRIPTOR_ENV.as_bytes())
        })
        .collect();
    let cwd = std::env::current_dir()?;
    let child = Child::spawn(Launch {
        program: &binary,
        args: &argv,
        env: &environment,
        cwd: Some(&cwd),
        stdin: Stream::Inherit,
        stdout: Stream::Inherit,
        stderr: Stream::Inherit,
    })?;
    let code = loop {
        if let Some(code) = child.exit_code()? {
            break code;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    child.terminate()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !child.empty()? {
        if Instant::now() >= deadline {
            return Err(io::Error::other("launcher scope remains"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(code)
}

/// Produce Claude's shell-mediated apiKeyHelper command without evaluating user paths
/// in cmd.exe. PowerShell receives one UTF-16LE program containing literal native argv.
/// Only a system-directory executable appears in the outer command language.
pub fn managed_helper_command(executable: &Path, args: &[String]) -> io::Result<String> {
    use base64::Engine;
    if !executable.is_absolute() || args.len() > 32 {
        return Err(io::Error::other("helper arguments"));
    }
    let path = executable
        .to_str()
        .ok_or_else(|| io::Error::other("helper path encoding"))?;
    let quote = |value: &str| -> io::Result<String> {
        if value.contains(['\0', '\r', '\n']) || value.len() > 32768 {
            return Err(io::Error::other("helper argument"));
        }
        Ok(format!("'{}'", value.replace('\'', "''")))
    };
    let mut script = format!("$ErrorActionPreference='Stop'; try {{ & {}", quote(path)?);
    for arg in args {
        script.push(' ');
        script.push_str(&quote(arg)?);
    }
    script.push_str("; exit $LASTEXITCODE } catch { exit 1 }");
    let payload: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
    let powershell = system_executable(Path::new("WindowsPowerShell/v1.0/powershell.exe"))?;
    let path = powershell
        .to_str()
        .ok_or_else(|| io::Error::other("system executable encoding"))?;
    // cmd expansion can precede quotation; reject those characters in its one path.
    if path.contains(['%', '!', '"', '&', '|', '<', '>', '^', '\r', '\n', '\0']) {
        return Err(io::Error::other("system executable command path"));
    }
    let command =
        format!("\"{path}\" -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}");
    if command.encode_utf16().count() >= 8191 {
        return Err(io::Error::other("helper command bound"));
    }
    Ok(command)
}

/// Use the kernel's system directory, never an ambient SystemRoot/COMSPEC/PATH selector.
pub fn system_executable(relative: &Path) -> io::Result<PathBuf> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(io::Error::other("system executable leaf"));
    }
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    if length >= buffer.len() {
        return Err(io::Error::other("system directory bound"));
    }
    let root = PathBuf::from(
        String::from_utf16(&buffer[..length])
            .map_err(|_| io::Error::other("system directory encoding"))?,
    );
    let path = root.join(relative);
    let file = security::open_identity(&path, false)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("system executable kind"));
    }
    Ok(path)
}
