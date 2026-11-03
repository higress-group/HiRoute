use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use serde::Serialize;

pub(super) fn write_ready_frame(role: &hiroute_daemon::RoleAllHandle) -> std::io::Result<()> {
    write_ready_to(
        &mut std::io::stdout().lock(),
        role.control_endpoint().path(),
        role.gateway_address(),
    )
}

pub(super) fn write_startup_failure_frame(
    code: hiroute_diagnostics::event::StartupFailureCode,
) -> std::io::Result<()> {
    let mut writer = std::io::stdout().lock();
    StartupFailure {
        schema: "hiroute.daemon-startup-failure/v1",
        code,
    }
    .write_to(&mut writer)
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct StartupFailure {
    schema: &'static str,
    code: hiroute_diagnostics::event::StartupFailureCode,
}

impl StartupFailure {
    fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

pub(super) fn write_ready_to(
    writer: &mut impl Write,
    control_endpoint: &Path,
    gateway_listen: SocketAddr,
) -> std::io::Result<()> {
    Ready {
        schema: "hiroute.daemon-ready/v1",
        role: "all",
        control_endpoint,
        gateway_listen,
        process_id: std::process::id(),
    }
    .write_to(writer)
}

#[derive(Serialize)]
struct Ready<'a> {
    schema: &'static str,
    role: &'static str,
    control_endpoint: &'a Path,
    gateway_listen: SocketAddr,
    process_id: u32,
}

impl Ready<'_> {
    fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind};

    use super::*;

    fn ready() -> Ready<'static> {
        Ready {
            schema: "hiroute.daemon-ready/v1",
            role: "all",
            control_endpoint: Path::new("/runtime/control.sock"),
            gateway_listen: "127.0.0.1:4567".parse().unwrap(),
            process_id: 123,
        }
    }

    struct Writer {
        bytes: Vec<u8>,
        limit: usize,
        max_write: usize,
        interrupted: bool,
        fail_flush: bool,
        flushes: usize,
    }

    impl Default for Writer {
        fn default() -> Self {
            Self {
                bytes: Vec::new(),
                limit: usize::MAX,
                max_write: usize::MAX,
                interrupted: false,
                fail_flush: false,
                flushes: 0,
            }
        }
    }

    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if std::mem::take(&mut self.interrupted) {
                return Err(ErrorKind::Interrupted.into());
            }
            if self.bytes.len() == self.limit {
                return Err(ErrorKind::BrokenPipe.into());
            }
            let count = bytes
                .len()
                .min(self.max_write)
                .min(self.limit - self.bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.fail_flush {
                Err(ErrorKind::BrokenPipe.into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn ready_frame_is_one_flushed_protocol_line() {
        let mut writer = Writer::default();
        ready().write_to(&mut writer).unwrap();
        assert_eq!(writer.bytes.last(), Some(&b'\n'));
        assert_eq!(
            writer.bytes.iter().filter(|&&byte| byte == b'\n').count(),
            1
        );
        assert_eq!(writer.flushes, 1);
        let value: serde_json::Value = serde_json::from_slice(&writer.bytes).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "schema": "hiroute.daemon-ready/v1", "role": "all",
                "control_endpoint": "/runtime/control.sock",
                "gateway_listen": "127.0.0.1:4567", "process_id": 123
            })
        );
    }

    #[test]
    fn codex_home_conflict_frame_exposes_no_paths_or_configuration() {
        let mut writer = Writer::default();
        StartupFailure {
            schema: "hiroute.daemon-startup-failure/v1",
            code: hiroute_diagnostics::event::StartupFailureCode::CodexHomeConflict,
        }
        .write_to(&mut writer)
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&writer.bytes).unwrap(),
            serde_json::json!({"schema":"hiroute.daemon-startup-failure/v1", "code":"codex_home_conflict"})
        );
    }

    #[test]
    fn startup_failure_frame_exposes_only_the_stable_code() {
        let mut writer = Writer::default();
        StartupFailure {
            schema: "hiroute.daemon-startup-failure/v1",
            code: hiroute_diagnostics::event::StartupFailureCode::StorageUnavailable,
        }
        .write_to(&mut writer)
        .unwrap();
        assert_eq!(writer.flushes, 1);
        assert_eq!(writer.bytes.last(), Some(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&writer.bytes).unwrap(),
            serde_json::json!({
                "schema": "hiroute.daemon-startup-failure/v1",
                "code": "storage_unavailable"
            })
        );
    }

    #[test]
    fn ready_frame_retries_short_and_interrupted_writes() {
        let mut writer = Writer {
            max_write: 2,
            interrupted: true,
            ..Writer::default()
        };
        ready().write_to(&mut writer).unwrap();
        let mut expected = serde_json::to_vec(&ready()).unwrap();
        expected.push(b'\n');
        assert_eq!(writer.bytes, expected);
        assert_eq!(writer.flushes, 1);
    }

    #[test]
    fn ready_frame_returns_json_write_failure() {
        let mut writer = Writer {
            limit: 12,
            ..Writer::default()
        };
        assert!(ready().write_to(&mut writer).is_err());
        assert_eq!(writer.bytes.len(), 12);
        assert_eq!(writer.flushes, 0);
    }

    #[test]
    fn ready_frame_returns_newline_failure() {
        let expected = serde_json::to_vec(&ready()).unwrap();
        let mut writer = Writer {
            limit: expected.len(),
            ..Writer::default()
        };
        assert!(ready().write_to(&mut writer).is_err());
        assert_eq!(writer.bytes, expected);
        assert_eq!(writer.flushes, 0);
    }

    #[test]
    fn ready_frame_returns_flush_failure() {
        let mut writer = Writer {
            fail_flush: true,
            ..Writer::default()
        };
        assert!(ready().write_to(&mut writer).is_err());
        assert_eq!(writer.bytes.last(), Some(&b'\n'));
        assert_eq!(writer.flushes, 1);
    }

    #[test]
    fn ready_frame_returns_zero_write_failure() {
        let mut writer = Writer {
            max_write: 0,
            ..Writer::default()
        };
        assert!(ready().write_to(&mut writer).is_err());
        assert!(writer.bytes.is_empty());
        assert_eq!(writer.flushes, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ready_frame_returns_closed_pipe_failure_with_line_buffered_output() {
        let (reader, writer) = nix::unistd::pipe().unwrap();
        drop(reader);
        let mut writer = io::LineWriter::with_capacity(4096, std::fs::File::from(writer));
        assert!(ready().write_to(&mut writer).is_err());
    }
}
