//! User-owned controlling-terminal input. Secret bytes never enter ordinary CLI output.
use hiroute_application_api::ErrorCode;
#[cfg(unix)]
use zeroize::{Zeroize, Zeroizing};

pub(crate) fn read(format: &str) -> Result<String, ErrorCode> {
    if !matches!(format, "bearer" | "raw") {
        return Err(ErrorCode::InvalidArguments);
    }
    #[cfg(unix)]
    {
        terminal::read(format).map(|mut value| std::mem::take(&mut *value))
    }
    #[cfg(not(unix))]
    Err(ErrorCode::ActionRequired)
}

#[cfg(unix)]
fn authorization(bytes: &[u8], format: &str) -> Result<Zeroizing<String>, ErrorCode> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?
        .trim();
    if value.is_empty()
        || value.contains(char::is_control)
        || format == "bearer"
            && value
                .as_bytes()
                .get(..7)
                .is_some_and(|v| v.eq_ignore_ascii_case(b"bearer "))
    {
        return Err(ErrorCode::InvalidArguments);
    }
    let mut result = Zeroizing::new(String::with_capacity(value.len() + 7));
    if format == "bearer" {
        result.push_str("Bearer ");
    }
    result.push_str(value);
    Ok(result)
}

#[cfg(unix)]
mod terminal {
    use super::*;
    use nix::sys::termios::{self, LocalFlags, SetArg, SpecialCharacterIndices, Termios};
    use std::{
        fs::File,
        io::{Read, Write},
    };

    struct HiddenTerminal {
        file: File,
        original: Termios,
    }
    impl HiddenTerminal {
        fn open() -> Result<Self, ErrorCode> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .map_err(|_| ErrorCode::ActionRequired)?;
            let original = termios::tcgetattr(&file).map_err(|_| ErrorCode::ActionRequired)?;
            let mut hidden = original.clone();
            // Read control characters ourselves so Ctrl-C/Ctrl-D cancel through cleanup.
            hidden.local_flags.remove(
                LocalFlags::ECHO
                    | LocalFlags::ECHONL
                    | LocalFlags::ICANON
                    | LocalFlags::ISIG
                    | LocalFlags::IEXTEN,
            );
            hidden.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
            hidden.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
            termios::tcsetattr(&file, SetArg::TCSAFLUSH, &hidden)
                .map_err(|_| ErrorCode::ActionRequired)?;
            Ok(Self { file, original })
        }
    }
    impl Drop for HiddenTerminal {
        fn drop(&mut self) {
            let _ = termios::tcsetattr(&self.file, SetArg::TCSAFLUSH, &self.original);
        }
    }

    pub(super) fn read(format: &str) -> Result<Zeroizing<String>, ErrorCode> {
        let mut terminal = HiddenTerminal::open()?;
        let prompt = if format == "bearer" {
            "API key (hidden; paste the key only, without Bearer): "
        } else {
            "Authorization value (hidden): "
        };
        let result = (|| {
            terminal
                .file
                .write_all(prompt.as_bytes())
                .map_err(|_| ErrorCode::ActionRequired)?;
            terminal
                .file
                .flush()
                .map_err(|_| ErrorCode::ActionRequired)?;
            let mut bytes = Zeroizing::new(Vec::with_capacity(16 * 1024));
            loop {
                let mut byte = Zeroizing::new([0_u8; 1]);
                match terminal.file.read(&mut *byte) {
                    Ok(0) => return Err(ErrorCode::ActionRequired),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(ErrorCode::ActionRequired),
                }
                match byte[0] {
                    b'\r' | b'\n' => return authorization(&bytes, format),
                    3 | 4 | 26 => return Err(ErrorCode::ActionRequired),
                    8 | 127 => {
                        if let Some(last) = bytes.last_mut() {
                            last.zeroize();
                            bytes.pop();
                        }
                    }
                    0..=31 => return Err(ErrorCode::InvalidArguments),
                    value if bytes.len() < 16 * 1024 => bytes.push(value),
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
        })();
        let _ = terminal.file.write_all(b"\n");
        result
    }
}
