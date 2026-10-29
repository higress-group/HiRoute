//! Bounded structural sizing before the canonical parser validates JSON.
use super::{IngressDocumentStats, MAX_JSON_DEPTH, ReplayError};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct StructuralScanner {
    structure_units: usize,
    depth: usize,
    max_depth: usize,
    current_string_bytes: usize,
    max_string_bytes: usize,
    in_string: bool,
    escaped: bool,
    unicode_digits: u8,
    unicode_value: u16,
    pending_high_surrogate: bool,
}

impl StructuralScanner {
    pub(super) fn feed(&mut self, mut bytes: &[u8]) -> Result<(), ReplayError> {
        while !bytes.is_empty() {
            if self.in_string && !self.escaped && self.unicode_digits == 0 {
                let run = memchr::memchr2(b'"', b'\\', bytes).unwrap_or(bytes.len());
                self.add_string_bytes(run)?;
                bytes = &bytes[run..];
                if bytes.is_empty() {
                    break;
                }
            }
            self.step(bytes[0])?;
            bytes = &bytes[1..];
        }
        Ok(())
    }

    fn step(&mut self, byte: u8) -> Result<(), ReplayError> {
        if self.in_string {
            if self.unicode_digits > 0 {
                self.unicode_value = self
                    .unicode_value
                    .wrapping_mul(16)
                    .wrapping_add(u16::from((byte as char).to_digit(16).unwrap_or(0) as u8));
                self.unicode_digits -= 1;
                if self.unicode_digits == 0 {
                    let code = self.unicode_value;
                    let width = if (0xd800..=0xdbff).contains(&code) {
                        self.pending_high_surrogate = true;
                        4
                    } else if (0xdc00..=0xdfff).contains(&code) && self.pending_high_surrogate {
                        self.pending_high_surrogate = false;
                        0
                    } else if code <= 0x7f {
                        1
                    } else if code <= 0x7ff {
                        2
                    } else {
                        3
                    };
                    self.add_string_bytes(width)?;
                }
                return Ok(());
            }
            if self.escaped {
                self.escaped = false;
                if byte == b'u' {
                    self.unicode_digits = 4;
                    self.unicode_value = 0;
                } else {
                    self.add_string_bytes(1)?;
                }
                return Ok(());
            } else if byte == b'\\' {
                self.escaped = true;
                return Ok(());
            } else if byte == b'"' {
                self.in_string = false;
                self.max_string_bytes = self.max_string_bytes.max(self.current_string_bytes);
                return Ok(());
            }
            self.add_string_bytes(1)?;
            return Ok(());
        }
        match byte {
            b'"' => {
                self.in_string = true;
                self.current_string_bytes = 0;
            }
            b'{' | b'[' => {
                self.depth = self
                    .depth
                    .checked_add(1)
                    .ok_or(ReplayError::StructureLimit)?;
                self.max_depth = self.max_depth.max(self.depth);
                self.bump()?;
                if self.depth > MAX_JSON_DEPTH {
                    return Err(ReplayError::StructureLimit);
                }
            }
            b'}' | b']' => {
                self.depth = self
                    .depth
                    .checked_sub(1)
                    .ok_or(ReplayError::StructureLimit)?;
            }
            b':' | b',' => self.bump()?,
            _ => {}
        }
        Ok(())
    }

    fn bump(&mut self) -> Result<(), ReplayError> {
        self.structure_units = self
            .structure_units
            .checked_add(1)
            .ok_or(ReplayError::StructureLimit)?;
        Ok(())
    }

    fn add_string_bytes(&mut self, count: usize) -> Result<(), ReplayError> {
        self.current_string_bytes = self
            .current_string_bytes
            .checked_add(count)
            .ok_or(ReplayError::LengthOverflow)?;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<IngressDocumentStats, ReplayError> {
        if self.in_string || self.escaped || self.unicode_digits > 0 || self.depth != 0 {
            return Err(ReplayError::Integrity);
        }
        Ok(IngressDocumentStats {
            structure_units: self.structure_units,
            max_depth: self.max_depth,
            max_string_bytes: self.max_string_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_runs_match_scalar_sizing_for_every_byte_and_split_escape() {
        let bodies = [
            serde_json::to_vec(
                &serde_json::json!({"text":"plain 路由🦀 \\\"\n".repeat(1024),"nested":[{},[]]}),
            )
            .unwrap(),
            br#"{"text":"\uD83D\uDE80 \\"\u0041\n","n":0}"#.to_vec(),
            (0..=255).chain(*b"\"\\u00").collect(),
            br#"{"text":"unfinished\uD8"#.to_vec(),
        ];
        for body in bodies {
            for size in [1, 2, 3, 7, 127, 8192, body.len()] {
                let mut bulk = StructuralScanner::default();
                let mut scalar = StructuralScanner::default();
                for chunk in body.chunks(size) {
                    let actual = bulk.feed(chunk).map_err(|e| std::mem::discriminant(&e));
                    let expected = chunk
                        .iter()
                        .try_for_each(|byte| scalar.step(*byte))
                        .map_err(|e| std::mem::discriminant(&e));
                    assert_eq!(actual, expected, "chunk={size}");
                    assert_eq!(bulk, scalar, "chunk={size}");
                    if actual.is_err() {
                        break;
                    }
                }
                assert_eq!(
                    bulk.finish().map_err(|e| std::mem::discriminant(&e)),
                    scalar.finish().map_err(|e| std::mem::discriminant(&e))
                );
            }
        }
    }
}
