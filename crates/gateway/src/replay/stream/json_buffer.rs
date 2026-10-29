//! Coalesce the existing canonical serializer's small writes inside one range.
//! The fixed stack buffer adds no heap admission or retained Replay ownership.
use std::io::{self, Write};

const BUFFER_BYTES: usize = 8192;

pub(super) struct JsonBuffer<'a, W> {
    inner: &'a mut W,
    bytes: [u8; BUFFER_BYTES],
    used: usize,
    failed: bool,
}

impl<'a, W: Write> JsonBuffer<'a, W> {
    pub(super) fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            bytes: [0; BUFFER_BYTES],
            used: 0,
            failed: false,
        }
    }

    fn check(&self) -> io::Result<()> {
        if self.failed {
            Err(io::Error::other("canonical range writer is poisoned"))
        } else {
            Ok(())
        }
    }
}

impl<W: Write> Write for JsonBuffer<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check()?;
        if bytes.len() >= BUFFER_BYTES {
            self.flush()?;
            let result = self.inner.write(bytes);
            if matches!(&result, Err(error) if error.kind() != io::ErrorKind::Interrupted)
                || matches!(&result, Ok(0))
            {
                self.failed = true;
            }
            return result;
        }
        if BUFFER_BYTES - self.used < bytes.len() {
            self.flush()?;
        }
        self.bytes[self.used..self.used + bytes.len()].copy_from_slice(bytes);
        self.used += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        if let Err(error) = self.inner.write_all(&self.bytes[..self.used]) {
            self.failed = true;
            return Err(error);
        }
        self.used = 0;
        let result = self.inner.flush();
        if matches!(&result, Err(error) if error.kind() != io::ErrorKind::Interrupted) {
            self.failed = true;
        }
        result
    }
}

// Deliberately no Drop flush: the caller must observe a failed final flush
// before it can seal a ContentRef. Dropping failed work cannot publish a prefix.
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sink {
        bytes: Vec<u8>,
        calls: usize,
        quantum: Option<usize>,
        fail_after: Option<usize>,
        interrupt: bool,
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if std::mem::take(&mut self.interrupt) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let remaining = self
                .fail_after
                .unwrap_or(usize::MAX)
                .saturating_sub(self.bytes.len());
            if remaining == 0 {
                return Err(io::Error::other("injected write failure"));
            }
            let n = bytes
                .len()
                .min(self.quantum.unwrap_or(usize::MAX))
                .min(remaining);
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn canonical_bytes_are_identical_and_small_writes_are_coalesced() {
        let value = serde_json::json!({"z":"路由🦀\n\\\"".repeat(2000),
            "a":(0..3000).map(|n|serde_json::json!({"v":n,"k":"small"})).collect::<Vec<_>>()});
        let mut direct = Sink::default();
        crate::replay::write_canonical_json(&mut direct, &value).unwrap();
        let mut buffered = Sink::default();
        {
            let mut writer = JsonBuffer::new(&mut buffered);
            crate::replay::write_canonical_json(&mut writer, &value).unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(buffered.bytes, direct.bytes);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&buffered.bytes).unwrap(),
            value
        );
        assert!(direct.calls > 10_000);
        assert!(buffered.calls < 32, "{} writes", buffered.calls);
    }

    #[test]
    fn fragment_boundaries_partial_writes_and_interruptions_keep_every_byte() {
        let bytes = (0..=255u8)
            .cycle()
            .take(5 * BUFFER_BYTES + 3)
            .collect::<Vec<_>>();
        for quantum in [
            1,
            2,
            7,
            127,
            BUFFER_BYTES - 1,
            BUFFER_BYTES,
            BUFFER_BYTES + 1,
            bytes.len(),
        ] {
            let mut sink = Sink {
                quantum: Some(7),
                interrupt: true,
                ..Sink::default()
            };
            {
                let mut writer = JsonBuffer::new(&mut sink);
                for part in bytes.chunks(quantum) {
                    writer.write_all(part).unwrap();
                }
                writer.flush().unwrap();
            }
            assert_eq!(sink.bytes, bytes, "input quantum {quantum}");
        }
    }

    #[test]
    fn failed_final_flush_is_not_retried_or_hidden_by_drop() {
        for len in [100, 3 * BUFFER_BYTES] {
            let bytes = vec![b'x'; len];
            let mut sink = Sink {
                fail_after: Some(17),
                ..Sink::default()
            };
            {
                let mut writer = JsonBuffer::new(&mut sink);
                let result = writer.write_all(&bytes).and_then(|_| writer.flush());
                assert!(result.is_err());
                assert!(writer.flush().is_err());
                assert!(writer.write_all(b"later").is_err());
            }
            assert_eq!(sink.bytes, bytes[..17]);
        }
    }
}
