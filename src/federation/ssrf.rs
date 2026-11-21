use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use serde::Serialize;

const MAX_URL_LEN: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressClass {
    Public,
    Private,
    Loopback,
}

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub ip: IpAddr,
    pub class: AddressClass,
    pub plaintext: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsrfError {
    Invalid,
    Blocked,
    TlsRequired,
    ResolveFailed,
}

impl SsrfError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "ADDRESS_INVALID",
            Self::Blocked => "ADDRESS_BLOCKED",
            Self::TlsRequired => "TLS_REQUIRED",
            Self::ResolveFailed => "RESOLVE_FAILED",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => "invalid address",
            Self::Blocked => "address is not allowed",
            Self::TlsRequired => "public targets must use https",
            Self::ResolveFailed => "could not resolve host",
        }
    }
}

pub async fn resolve(
    raw: &str,
    allow_loopback: bool,
    allow_plaintext_lan: bool,
) -> Result<ResolvedTarget, SsrfError> {
    if raw.len() > MAX_URL_LEN {
        return Err(SsrfError::Invalid);
    }
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| SsrfError::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(SsrfError::Invalid);
    }
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(SsrfError::Invalid);
    }
    let host = url
        .host_str()
        .ok_or(SsrfError::Invalid)?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || host.contains('%') {
        return Err(SsrfError::Invalid);
    }
    let port = url.port_or_known_default().ok_or(SsrfError::Invalid)?;

    let ips = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else {
        let lookup = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| SsrfError::ResolveFailed)?
        .map_err(|_| SsrfError::ResolveFailed)?;
        let mut ips = Vec::new();
        for address in lookup {
            if !ips.contains(&address.ip()) {
                ips.push(address.ip());
            }
        }
        if ips.is_empty() {
            return Err(SsrfError::ResolveFailed);
        }
        ips
    };

    let mut class = None;
    for ip in &ips {
        let current = classify(*ip, allow_loopback)?;
        match class {
            None => class = Some(current),
            Some(previous) if previous == current => {}
            Some(_) => return Err(SsrfError::Blocked),
        }
    }
    let class = class.ok_or(SsrfError::ResolveFailed)?;
    if class == AddressClass::Public && scheme != "https" {
        return Err(SsrfError::TlsRequired);
    }
    if class != AddressClass::Public && scheme == "http" && !allow_plaintext_lan {
        return Err(SsrfError::Blocked);
    }
    let ip = ips[0];
    let plaintext = scheme == "http";
    Ok(ResolvedTarget {
        scheme,
        host,
        port,
        ip,
        class,
        plaintext,
    })
}

fn classify(ip: IpAddr, allow_loopback: bool) -> Result<AddressClass, SsrfError> {
    match ip {
        IpAddr::V4(ip) => classify_v4(ip, allow_loopback),
        IpAddr::V6(ip) => classify_v6(ip, allow_loopback),
    }
}

fn classify_v4(ip: Ipv4Addr, allow_loopback: bool) -> Result<AddressClass, SsrfError> {
    let octets = ip.octets();
    if ip == Ipv4Addr::new(169, 254, 169, 254) || ip == Ipv4Addr::new(100, 100, 100, 200) {
        return Err(SsrfError::Blocked);
    }
    if ip.is_loopback() {
        return if allow_loopback {
            Ok(AddressClass::Loopback)
        } else {
            Err(SsrfError::Blocked)
        };
    }
    if ip.is_unspecified() || ip.is_multicast() || octets[0] >= 240 {
        return Err(SsrfError::Blocked);
    }
    if ip.is_link_local() {
        return Err(SsrfError::Blocked);
    }
    if ip.is_private() {
        return Ok(AddressClass::Private);
    }
    if octets[0] == 100 && (64..128).contains(&octets[1]) {
        return Ok(AddressClass::Private);
    }
    Ok(AddressClass::Public)
}

fn classify_v6(ip: Ipv6Addr, allow_loopback: bool) -> Result<AddressClass, SsrfError> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4, allow_loopback);
    }
    let segments = ip.segments();
    if ip.is_loopback() {
        return if allow_loopback {
            Ok(AddressClass::Loopback)
        } else {
            Err(SsrfError::Blocked)
        };
    }
    if ip.is_unspecified() || ip.is_multicast() {
        return Err(SsrfError::Blocked);
    }
    // fd00:ec2::254 is the AWS IPv6 metadata endpoint.
    if segments[0] == 0xfd00
        && segments[1] == 0x0ec2
        && segments[2..6].iter().all(|s| *s == 0)
        && segments[6] == 0
        && segments[7] == 0x0254
    {
        return Err(SsrfError::Blocked);
    }
    if (segments[0] & 0xfe00) == 0xfc00 {
        return Ok(AddressClass::Private);
    }
    if (segments[0] & 0xffc0) == 0xfe80 {
        return Err(SsrfError::Blocked);
    }
    Ok(AddressClass::Public)
}

pub fn pin_for(host: &str, port: u16, ip: IpAddr) -> (String, SocketAddr) {
    (host.to_owned(), SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(value: &str) -> Ipv4Addr {
        value.parse().unwrap()
    }

    #[test]
    fn classifies_ipv4() {
        assert_eq!(
            classify_v4(v4("192.168.1.5"), false).unwrap(),
            AddressClass::Private
        );
        assert_eq!(
            classify_v4(v4("10.0.0.1"), false).unwrap(),
            AddressClass::Private
        );
        assert_eq!(
            classify_v4(v4("100.64.0.1"), false).unwrap(),
            AddressClass::Private
        );
        assert_eq!(
            classify_v4(v4("8.8.8.8"), false).unwrap(),
            AddressClass::Public
        );
        assert!(classify_v4(v4("127.0.0.1"), false).is_err());
        assert!(classify_v4(v4("127.0.0.1"), true).is_ok());
        assert!(classify_v4(v4("169.254.1.1"), false).is_err());
        assert!(classify_v4(v4("169.254.169.254"), false).is_err());
        assert!(classify_v4(v4("100.100.100.200"), false).is_err());
        assert!(classify_v4(v4("224.0.0.1"), false).is_err());
    }

    #[test]
    fn classifies_ipv6() {
        let private: Ipv6Addr = "fd00::1".parse().unwrap();
        assert_eq!(classify_v6(private, false).unwrap(), AddressClass::Private);
        let public: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        assert_eq!(classify_v6(public, false).unwrap(), AddressClass::Public);
        let mapped: Ipv6Addr = "::ffff:8.8.8.8".parse().unwrap();
        assert_eq!(classify_v6(mapped, false).unwrap(), AddressClass::Public);
        let metadata: Ipv6Addr = "fd00:ec2::254".parse().unwrap();
        assert!(classify_v6(metadata, false).is_err());
    }

    #[tokio::test]
    async fn rejects_bad_urls() {
        assert_eq!(
            resolve("file:///etc/passwd", false, true)
                .await
                .unwrap_err(),
            SsrfError::Invalid
        );
        assert_eq!(
            resolve("http://8.8.8.8/x", false, true).await.unwrap_err(),
            SsrfError::TlsRequired
        );
        assert_eq!(
            resolve("http://127.0.0.1/", false, true).await.unwrap_err(),
            SsrfError::Blocked
        );
        assert_eq!(
            resolve("http://192.168.1.1/", false, false)
                .await
                .unwrap_err(),
            SsrfError::Blocked
        );
    }
}
