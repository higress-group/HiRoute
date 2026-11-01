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
