#![forbid(unsafe_code)]
use url::Url;
#[cfg(any(test, all(windows, feature = "shell-runtime")))]
mod handoff;
#[cfg(any(test, all(windows, feature = "shell-runtime")))]
#[path = "runtime/startup_failure.rs"]
mod startup_failure;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleOrigin(String);
impl ConsoleOrigin {
    pub fn launch(value: &str) -> Result<(Self, Url), &'static str> {
        let url = Url::parse(value).map_err(|_| "CONSOLE_LAUNCH_INVALID")?;
        let query: Vec<_> = url.query_pairs().collect();
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port_or_known_default().is_none_or(|p| p == 0)
            || url.path() != "/launch"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || query.len() != 1
            || query[0].0 != "code"
            || query[0].1.len() != 64
            || !query[0]
                .1
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("CONSOLE_LAUNCH_INVALID");
        }
        Ok((Self(url.origin().ascii_serialization()), url))
    }
    pub fn allows(&self, url: &Url) -> bool {
        url.origin().ascii_serialization() == self.0
            && url.username().is_empty()
            && url.password().is_none()
    }
}
pub fn external_url(value: &str) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "EXTERNAL_URL_INVALID")?;
    if value.len() > 2048
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("EXTERNAL_URL_SCHEME_DENIED");
    }
    Ok(url)
}
#[cfg(all(windows, feature = "shell-runtime"))]
pub mod runtime;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_and_ipc_scope_use_exact_console_origin_and_never_accept_remote_credentials() {
        let link = format!("http://127.0.0.1:5838/launch?code={}", "a".repeat(64));
        let (origin, _) = ConsoleOrigin::launch(&link).unwrap();
        assert!(origin.allows(&Url::parse("http://127.0.0.1:5838/").unwrap()));
        for url in [
            "http://localhost:5838/",
            "http://127.0.0.1:5839/",
            "https://127.0.0.1:5838/",
            "http://example.test:5838/",
        ] {
            assert!(!origin.allows(&Url::parse(url).unwrap()));
        }
        for value in [
            link.replace("127.0.0.1", "example.test"),
            link.clone() + "&code=duplicate",
            link.clone() + "#fragment",
            link.replace("/launch", "/other"),
        ] {
            assert!(ConsoleOrigin::launch(&value).is_err());
        }
    }
    #[test]
    fn host_external_open_cannot_execute_schemes_or_embed_account_credentials() {
        for value in [
            "file:///C:/test",
            "javascript:alert(1)",
            "http://user:password@example.test",
            "https://example.test/\nnext",
        ] {
            assert!(external_url(value).is_err());
        }
        assert!(external_url("https://example.test/guide").is_ok());
    }
}
