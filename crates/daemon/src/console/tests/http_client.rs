//! This fixture consumes a complete HTTP message, not an extra graceful EOF.
use std::io::{self, BufRead, BufReader, Read};

pub(super) fn response(reader: impl Read) -> io::Result<(String, String)> {
    let mut reader = BufReader::new(reader);
    let mut headers = String::new();
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete response headers",
            ));
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid response length")
            })?);
        }
        headers.push_str(&line);
    }
    let length = length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing full-response length")
    })?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let body = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response is not UTF-8"))?;
    Ok((headers.trim_end_matches("\r\n").into(), body))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ResetAfter(std::io::Cursor<Vec<u8>>);
    impl Read for ResetAfter {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.0.position() == self.0.get_ref().len() as u64 {
                Err(io::ErrorKind::ConnectionReset.into())
            } else {
                self.0.read(bytes)
            }
        }
    }
    #[test]
    fn complete_message_does_not_require_graceful_tcp_eof() {
        let message = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\n\r\n{}".to_vec();
        let mut legacy = ResetAfter(std::io::Cursor::new(message.clone()));
        let mut raw = String::new();
        assert_eq!(
            legacy.read_to_string(&mut raw).unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
        assert_eq!(raw.as_bytes(), message);
        let (headers, body) = response(ResetAfter(std::io::Cursor::new(message))).unwrap();
        assert!(headers.starts_with("HTTP/1.1 403 "));
        assert_eq!(body, "{}");
    }
    #[test]
    fn truncated_message_is_not_accepted_even_if_its_prefix_is_valid_json() {
        for reader in [
            ResetAfter(std::io::Cursor::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n{}".to_vec(),
            )),
            ResetAfter(std::io::Cursor::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n".to_vec(),
            )),
        ] {
            assert!(response(reader).is_err());
        }
    }
}
