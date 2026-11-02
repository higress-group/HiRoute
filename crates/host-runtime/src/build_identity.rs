//! Offline identity supplied by the binary at compile time; never inspect runtime Git or HOME.
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct BuildIdentityV1 {
    pub schema: &'static str,
    pub component: &'static str,
    pub version: &'static str,
    pub source_revision: Option<&'static str>,
    pub os: &'static str,
    pub arch: &'static str,
}

impl BuildIdentityV1 {
    pub fn compiled(
        component: &'static str,
        version: &'static str,
        revision: Option<&'static str>,
    ) -> Self {
        Self {
            schema: "hiroute.build-identity/v1",
            component,
            version,
            source_revision: revision.filter(|value| {
                matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }

    pub fn text(&self) -> String {
        format!(
            "HiRoute {} {}\nSource revision: {}\nTarget: {}/{}\n",
            self.component,
            self.version,
            self.source_revision.unwrap_or("unknown"),
            self.os,
            self.arch
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalid_source_metadata_stays_unknown_without_exposing_build_paths() {
        for revision in [
            None,
            Some(""),
            Some("/private/source/checkout"),
            Some("not-a-source-sha"),
        ] {
            let identity = BuildIdentityV1::compiled("cli", "0.3.2-dev.1", revision);
            assert!(identity.source_revision.is_none());
            assert!(identity.text().contains("Source revision: unknown"));
            assert!(
                !serde_json::to_string(&identity)
                    .unwrap()
                    .contains("/private/")
            );
        }
        let revision = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            BuildIdentityV1::compiled("daemon", "0.3.2-dev.1", Some(revision)).source_revision,
            Some(revision)
        );
    }
}
