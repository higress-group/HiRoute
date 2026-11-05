//! Process-owned model connection budget, frozen before publications serve requests.
use super::PublicationInstallError;
use std::{ffi::OsStr, sync::OnceLock, time::Duration};

pub(super) const DEFAULT_MODEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODEL_CONNECT_TIMEOUT_MS: u64 = 60_000;
static MODEL_CONNECT_TIMEOUT: OnceLock<Result<Duration, ()>> = OnceLock::new();

pub(super) fn model_connect_timeout() -> Result<Duration, PublicationInstallError> {
    (*MODEL_CONNECT_TIMEOUT
        .get_or_init(|| parse(std::env::var_os("HIROUTE_MODEL_CONNECT_TIMEOUT_MS").as_deref())))
    .map_err(|()| PublicationInstallError::InvalidModelConnectTimeout)
}

fn parse(value: Option<&OsStr>) -> Result<Duration, ()> {
    let Some(value) = value else {
        return Ok(DEFAULT_MODEL_CONNECT_TIMEOUT);
    };
    let value = value.to_str().ok_or(())?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    let millis: u64 = value.parse().map_err(|_| ())?;
    if !(1..=MAX_MODEL_CONNECT_TIMEOUT_MS).contains(&millis) {
        return Err(());
    }
    Ok(Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_connection_budget_is_independent_finite_and_strict() {
        assert_eq!(parse(None), Ok(Duration::from_secs(10)));
        for (text, millis) in [("1", 1), ("2000", 2000), ("15000", 15000), ("60000", 60000)] {
            assert_eq!(
                parse(Some(OsStr::new(text))),
                Ok(Duration::from_millis(millis))
            );
        }
        for text in [
            "",
            "0",
            "60001",
            "-1",
            "+1000",
            " 1000",
            "1000 ",
            "1.5",
            "18446744073709551616",
        ] {
            assert!(parse(Some(OsStr::new(text))).is_err(), "{text}");
        }
    }
}
