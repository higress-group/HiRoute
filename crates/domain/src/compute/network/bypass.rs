use super::{NativeNetworkDestinationV1, NativeNetworkErrorV1};
use ipnet::IpNet;
use std::net::IpAddr;
use url::{Host, Url};

pub(super) enum Rule {
    All,
    Host { host: String, port: Option<u16> },
    Network(IpNet),
}

pub(super) struct Rules(Vec<Rule>);

impl Rules {
    pub fn parse(value: &str) -> Result<Self, NativeNetworkErrorV1> {
        let invalid = NativeNetworkErrorV1::InvalidBypass;
        if value.len() > 4096 || value.contains(['\0', '\r', '\n']) {
            return Err(invalid);
        }
        let mut rules = Vec::new();
        for value in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if value == "*" {
                rules.push(Rule::All);
                continue;
            }
            if value.contains('/') {
                rules.push(Rule::Network(value.parse().map_err(|_| invalid)?));
                continue;
            }
            let (host, port) = if value.starts_with('[') {
                let close = value.find(']').ok_or(invalid)?;
                let host = &value[1..close];
                if !matches!(host.parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
                    return Err(invalid);
                }
                let suffix = &value[close + 1..];
                let port = if suffix.is_empty() {
                    None
                } else {
                    Some(parse_port(suffix.strip_prefix(':').ok_or(invalid)?)?)
                };
                (host, port)
            } else if value.parse::<IpAddr>().is_ok() {
                (value, None)
            } else if let Some((host, port)) = value.rsplit_once(':') {
                if host.contains(':') {
                    return Err(invalid);
                }
                (host, Some(parse_port(port)?))
            } else {
                (value, None)
            };
            let host = canonical_host(host.strip_prefix('.').unwrap_or(host)).ok_or(invalid)?;
            rules.push(Rule::Host { host, port });
        }
        Ok(Self(rules))
    }

    pub fn matches(&self, destination: &NativeNetworkDestinationV1) -> bool {
        let ip = destination.host.parse::<IpAddr>().ok();
        self.0.iter().any(|rule| match rule {
            Rule::All => true,
            Rule::Network(net) => ip.is_some_and(|ip| net.contains(&ip)),
            Rule::Host { host, port } => {
                port.is_none_or(|p| p == destination.port)
                    && (destination.host == *host
                        || (ip.is_none()
                            && host.parse::<IpAddr>().is_err()
                            && destination
                                .host
                                .strip_suffix(host)
                                .is_some_and(|prefix| prefix.ends_with('.'))))
            }
        })
    }
}

fn parse_port(value: &str) -> Result<u16, NativeNetworkErrorV1> {
    value
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or(NativeNetworkErrorV1::InvalidBypass)
}

pub(super) fn url_host(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(v) => canonical_host(v),
        Host::Ipv4(v) => Some(v.to_string()),
        Host::Ipv6(v) => Some(v.to_string()),
    }
}

pub(super) fn canonical_host(value: &str) -> Option<String> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    let value = value.strip_suffix('.').unwrap_or(value);
    if value.is_empty()
        || value.contains(['/', ':', '@', '?', '#', '%', '*', '\\'])
        || value.chars().any(char::is_whitespace)
    {
        return None;
    }
    match Host::parse(value).ok()? {
        Host::Domain(domain)
            if domain.len() <= 253
                && domain
                    .split('.')
                    .all(|part| !part.is_empty() && part.len() <= 63) =>
        {
            Some(domain)
        }
        Host::Ipv4(ip) => Some(ip.to_string()),
        _ => None,
    }
}

pub(super) fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}
