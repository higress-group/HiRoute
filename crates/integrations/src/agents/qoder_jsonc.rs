//! Bounded JSON-with-comments edits preserve every byte outside the selected object member.
//! Native syntax policy preserves each consumer's grammar; duplicate keys always fail.
use std::ops::Range;

use serde_json::{Map, Value};
use zeroize::Zeroizing;

use super::super::{QoderNativeError, qoder_error};

pub(super) const LIMIT: usize = 1024 * 1024;

pub(super) struct Document<'a> {
    bytes: &'a [u8],
    pub root: Node,
    trailing_commas: bool,
}

pub(super) struct Node {
    pub value: Value,
    pub range: Range<usize>,
    members: Vec<Member>,
}

struct Member {
    key: String,
    start: usize,
    value: Range<usize>,
    comma: Option<usize>,
}

impl<'a> Document<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, QoderNativeError> {
        Self::parse_with_trailing_commas(bytes, false)
    }

    pub fn parse_with_trailing_commas(
        bytes: &'a [u8],
        trailing_commas: bool,
    ) -> Result<Self, QoderNativeError> {
        if bytes.len() > LIMIT || std::str::from_utf8(bytes).is_err() {
            return Err(invalid());
        }
        let mut parser = Parser {
            bytes,
            pos: 0,
            trailing_commas,
        };
        // Match Qoder's BOM handling without changing the original bytes.
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            parser.pos = 3;
        }
        let root = parser.value(0)?;
        parser.skip()?;
        if parser.pos != bytes.len() || !root.value.is_object() {
            return Err(invalid());
        }
        Ok(Self {
            bytes,
            root,
            trailing_commas,
        })
    }

    pub fn object(&self, parent: &Node, key: &str) -> Result<Option<Node>, QoderNativeError> {
        let Some(member) = parent.members.iter().find(|member| member.key == key) else {
            return Ok(None);
        };
        let mut parser = Parser {
            bytes: self.bytes,
            pos: member.value.start,
            trailing_commas: self.trailing_commas,
        };
        let node = parser.value(0)?;
        if !node.value.is_object() {
            return Err(invalid());
        }
        Ok(Some(node))
    }

    pub fn edit(
        &self,
        object: &Node,
        key: &str,
        value: Option<&Value>,
    ) -> Result<Zeroizing<Vec<u8>>, QoderNativeError> {
        let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::new();
        if let Some((index, member)) = object
            .members
            .iter()
            .enumerate()
            .find(|(_, member)| member.key == key)
        {
            if let Some(value) = value {
                edits.push((
                    member.value.clone(),
                    serde_json::to_vec(value).map_err(|_| invalid())?,
                ));
            } else {
                edits.push((member.start..member.value.end, Vec::new()));
                if let Some(comma) = member
                    .comma
                    .or_else(|| index.checked_sub(1).and_then(|i| object.members[i].comma))
                {
                    edits.push((comma..comma + 1, Vec::new()));
                }
            }
        } else if let Some(value) = value {
            let trailing = object
                .members
                .last()
                .is_some_and(|member| member.comma.is_some());
            let mut insert = if object.members.is_empty() || trailing {
                Vec::new()
            } else {
                b",".to_vec()
            };
            insert.push(b'\n');
            insert.extend(serde_json::to_vec(key).map_err(|_| invalid())?);
            insert.push(b':');
            insert.extend(serde_json::to_vec(value).map_err(|_| invalid())?);
            if trailing {
                insert.push(b',');
            }
            insert.push(b'\n');
            edits.push((object.range.end - 1..object.range.end - 1, insert));
        }
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut result = Zeroizing::new(self.bytes.to_vec());
        for (range, replacement) in edits {
            result.splice(range, replacement);
        }
        Document::parse_with_trailing_commas(&result, self.trailing_commas)?;
        Ok(result)
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    trailing_commas: bool,
}

impl Parser<'_> {
    fn skip(&mut self) -> Result<(), QoderNativeError> {
        loop {
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            {
                self.pos += 1;
            }
            match self.bytes.get(self.pos..self.pos.saturating_add(2)) {
                Some(b"//") => {
                    self.pos += 2;
                    while self.bytes.get(self.pos).is_some_and(|byte| *byte != b'\n') {
                        // Qoder 1.1.65's settings readers disagree on these line endings:
                        // strip-comments accepts LF/CRLF, while persistence tokenization also
                        // ends comments on standalone CR and Unicode line separators. Never
                        // accept a document whose foreign fields one reader could hide.
                        if (self.bytes[self.pos] == b'\r'
                            && self.bytes.get(self.pos + 1) != Some(&b'\n'))
                            || self.bytes[self.pos..].starts_with(&[0xe2, 0x80, 0xa8])
                            || self.bytes[self.pos..].starts_with(&[0xe2, 0x80, 0xa9])
                        {
                            return Err(invalid());
                        }
                        self.pos += 1;
                    }
                }
                Some(b"/*") => {
                    self.pos += 2;
                    loop {
                        if self.bytes.get(self.pos..self.pos.saturating_add(2)) == Some(b"*/") {
                            self.pos += 2;
                            break;
                        }
                        if self.pos >= self.bytes.len() {
                            return Err(invalid());
                        }
                        self.pos += 1;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn string(&mut self) -> Result<String, QoderNativeError> {
        let start = self.pos;
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(invalid());
        }
        self.pos += 1;
        loop {
            match self.bytes.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 2;
                }
                Some(_) => self.pos += 1,
                None => return Err(invalid()),
            }
        }
        serde_json::from_slice(&self.bytes[start..self.pos]).map_err(|_| invalid())
    }

    fn value(&mut self, depth: usize) -> Result<Node, QoderNativeError> {
        if depth > 64 {
            return Err(invalid());
        }
        self.skip()?;
        let start = self.pos;
        let mut members = Vec::new();
        let value = match self.bytes.get(self.pos) {
            Some(b'{') => {
                self.pos += 1;
                self.skip()?;
                let mut object = Map::new();
                if self.bytes.get(self.pos) != Some(&b'}') {
                    loop {
                        self.skip()?;
                        let key_start = self.pos;
                        let key = self.string()?;
                        self.skip()?;
                        if self.bytes.get(self.pos) != Some(&b':') {
                            return Err(invalid());
                        }
                        self.pos += 1;
                        let child = self.value(depth + 1)?;
                        if object.insert(key.clone(), child.value).is_some() {
                            return Err(invalid());
                        }
                        self.skip()?;
                        let comma = (self.bytes.get(self.pos) == Some(&b',')).then_some(self.pos);
                        members.push(Member {
                            key,
                            start: key_start,
                            value: child.range,
                            comma,
                        });
                        if comma.is_none() {
                            break;
                        }
                        self.pos += 1;
                        self.skip()?;
                        if self.trailing_commas && self.bytes.get(self.pos) == Some(&b'}') {
                            break;
                        }
                    }
                }
                if self.bytes.get(self.pos) != Some(&b'}') {
                    return Err(invalid());
                }
                self.pos += 1;
                Value::Object(object)
            }
            Some(b'[') => {
                self.pos += 1;
                self.skip()?;
                let mut array = Vec::new();
                if self.bytes.get(self.pos) != Some(&b']') {
                    loop {
                        array.push(self.value(depth + 1)?.value);
                        self.skip()?;
                        if self.bytes.get(self.pos) != Some(&b',') {
                            break;
                        }
                        self.pos += 1;
                        self.skip()?;
                        if self.trailing_commas && self.bytes.get(self.pos) == Some(&b']') {
                            break;
                        }
                    }
                }
                if self.bytes.get(self.pos) != Some(&b']') {
                    return Err(invalid());
                }
                self.pos += 1;
                Value::Array(array)
            }
            Some(b'"') => Value::String(self.string()?),
            Some(_) => {
                while self.bytes.get(self.pos).is_some_and(|byte| {
                    !matches!(
                        byte,
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']' | b'/'
                    )
                }) {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(invalid());
                }
                serde_json::from_slice(&self.bytes[start..self.pos]).map_err(|_| invalid())?
            }
            None => return Err(invalid()),
        };
        Ok(Node {
            value,
            range: start..self.pos,
            members,
        })
    }
}

fn invalid() -> QoderNativeError {
    qoder_error("native settings JSON")
}

#[cfg(test)]
mod syntax_tests {
    use super::*;

    #[test]
    fn opencode_trailing_separators_survive_owned_edits() {
        let input = b"{ // unrelated\n\"list\":[1,2,],\n\"provider\":{\"user\":{\"key\":true,},}, // preserve\n}";
        assert!(Document::parse(input).is_err());
        let document = Document::parse_with_trailing_commas(input, true).unwrap();
        let providers = document
            .object(&document.root, "provider")
            .unwrap()
            .unwrap();
        let added = document
            .edit(
                &providers,
                "managed",
                Some(&serde_json::json!({"id":"route"})),
            )
            .unwrap();
        assert!(String::from_utf8_lossy(&added).contains("// preserve"));
        assert!(String::from_utf8_lossy(&added).contains("\"list\":[1,2,]"));
        let document = Document::parse_with_trailing_commas(&added, true).unwrap();
        let providers = document
            .object(&document.root, "provider")
            .unwrap()
            .unwrap();
        let removed = document.edit(&providers, "managed", None).unwrap();
        let parsed = Document::parse_with_trailing_commas(&removed, true).unwrap();
        assert_eq!(
            parsed.root.value["provider"],
            serde_json::json!({"user":{"key":true}})
        );
        assert!(String::from_utf8_lossy(&removed).contains("\"user\":{\"key\":true,}"));
    }

    #[test]
    fn opencode_policy_never_admits_duplicate_or_empty_members() {
        for input in [b"{\"a\":1,\"a\":2,}".as_slice(), b"{\"a\":[1,,]}", b"{,}"] {
            assert!(Document::parse_with_trailing_commas(input, true).is_err());
        }
    }
}
