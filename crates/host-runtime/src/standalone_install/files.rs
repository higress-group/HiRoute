use super::invalid;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn regular(path: &Path) -> io::Result<Option<File>> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(invalid());
                    }
                }
                if metadata.file_type().is_symlink() {
                    return Err(invalid());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    #[cfg(windows)]
    let result = hiroute_platform::windows::security::open_identity(path, false);
    #[cfg(not(windows))]
    let result = File::open(path);
    match result {
        Ok(file) if file.metadata()?.is_file() => Ok(Some(file)),
        Ok(_) => Err(invalid()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub(crate) fn digest(path: &Path) -> io::Result<Option<String>> {
    let Some(mut file) = regular(path)? else {
        return Ok(None);
    };
    let mut sha = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        sha.update(&buffer[..count]);
    }
    Ok(Some(format!("{:x}", sha.finalize())))
}
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    maximum: u64,
) -> io::Result<Option<T>> {
    let Some(mut file) = regular(path)? else {
        return Ok(None);
    };
    if file.metadata()?.len() > maximum {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(invalid());
    }
    let value = unique_json(&bytes)?;
    serde_json::from_value(value)
        .map(Some)
        .map_err(|_| invalid())
}

fn unique_json(bytes: &[u8]) -> io::Result<serde_json::Value> {
    use serde::{
        Deserialize,
        de::{self, MapAccess, SeqAccess, Visitor},
    };
    struct Unique(serde_json::Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON with unique object keys")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                    let mut map = serde_json::Map::new();
                    while let Some((key, value)) = a.next_entry::<String, Unique>()? {
                        if map.insert(key, value.0).is_some() {
                            return Err(de::Error::custom("duplicate JSON key"));
                        }
                    }
                    Ok(Unique(serde_json::Value::Object(map)))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                    let mut values = vec![];
                    while let Some(value) = a.next_element::<Unique>()? {
                        values.push(value.0);
                    }
                    Ok(Unique(serde_json::Value::Array(values)))
                }
                fn visit_bool<E: de::Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Unique(n.into()))
                        .ok_or_else(|| de::Error::custom("non-finite number"))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_none<E: de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(serde_json::Value::Null))
                }
                fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                    self.visit_none()
                }
            }
            d.deserialize_any(V)
        }
    }
    serde_json::from_slice::<Unique>(bytes)
        .map(|v| v.0)
        .map_err(|_| invalid())
}
pub(crate) fn directory(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let _ = hiroute_platform::windows::security::ensure_private_directory(path)?;
    }
    #[cfg(not(windows))]
    {
        fs::create_dir_all(path)?;
    }
    Ok(())
}
pub(crate) fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() as u64 > super::MAX_RECORD {
        return Err(invalid());
    }
    #[cfg(windows)]
    return hiroute_platform::windows::background::write_private_record(path, &bytes);
    #[cfg(not(windows))]
    hiroute_platform::durable::write_private_atomic(path, &bytes)
}
pub(crate) fn nonce() -> io::Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn copy_new(source: &Path, target: &Path) -> io::Result<()> {
    let mut input = regular(source)?.ok_or_else(invalid)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()
}
pub(crate) fn sibling(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(invalid)?.to_string_lossy();
    Ok(path.with_file_name(format!(".{name}.{}.stage", nonce()?)))
}
pub(crate) fn remove_owned(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let file = hiroute_platform::windows::security::open_delete(path, false)?;
        hiroute_platform::windows::security::delete_by_handle(&file)
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(path)?;
        hiroute_platform::durable::directory_barrier(path.parent().ok_or_else(invalid)?)
    }
}
