use std::collections::BTreeMap;
use std::ffi::OsString;

use sha2::{Digest, Sha256};

/// Captured once per runtime. Debug never exposes credentials in proxy URLs.
#[derive(Clone)]
pub(crate) struct ProxyEnvironment(BTreeMap<OsString, OsString>);

impl ProxyEnvironment {
    pub(crate) fn capture(values: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self(
            values
                .into_iter()
                .filter(|(key, _)| {
                    matches!(
                        key.to_str(),
                        Some(
                            "HTTP_PROXY"
                                | "HTTPS_PROXY"
                                | "NO_PROXY"
                                | "http_proxy"
                                | "https_proxy"
                                | "no_proxy"
                        )
                    )
                })
                .collect(),
        )
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&OsString, &OsString)> {
        self.0.iter()
    }

    pub(crate) fn digest(&self) -> String {
        let mut digest = Sha256::new();
        for (key, value) in &self.0 {
            for bytes in [key.as_encoded_bytes(), value.as_encoded_bytes()] {
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
        }
        format!("{:x}", digest.finalize())
    }
}

impl std::fmt::Debug for ProxyEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyEnvironment([REDACTED])")
    }
}
