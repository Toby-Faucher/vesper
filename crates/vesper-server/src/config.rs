use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub http_bind: SocketAddr,
    pub udp_bind: SocketAddr,
    /// Hostname resolved (and re-resolved) to the addresses the SFU advertises. Unset: the SFU
    /// advertises its own bound address, which needs a concrete `udp_bind` IP (not 0.0.0.0).
    pub public_host: Option<String>,
    /// UDP port clients reach the SFU on through the router forward.
    pub udp_port_public: u16,
    pub max_room_size: usize,
    /// Server ping period; a socket silent for two periods is dropped.
    pub ping_interval: Duration,
    /// Concurrent WebSockets; upgrades beyond this get 503.
    pub max_sockets: usize,
    /// Built web app served at `/` (with SPA fallback to `index.html`).
    pub web_dir: PathBuf,
    /// Holds `vesper.db` (and uploads later).
    pub data_dir: PathBuf,
    /// Scheme, host and port the browser sees, normalised to match an `Origin` header byte for byte.
    pub origin: String,
    /// True when `origin` is https: cookies get `Secure` and the `__Host-` name.
    pub secure_cookies: bool,
    pub upload_quota_bytes: u64,
    /// VAPID contact (`mailto:` or `https:`) sent to push services. None turns Web Push off.
    pub push_subject: Option<String>,
}

impl Config {
    /// `VESPER_HTTP_BIND` (default `0.0.0.0:8080`), `VESPER_UDP_BIND` (default `0.0.0.0:40000`),
    /// `VESPER_PUBLIC_HOST` (optional), `VESPER_UDP_PORT_PUBLIC` (default: the `VESPER_UDP_BIND`
    /// port), `VESPER_MAX_SOCKETS` (default 256), `VESPER_WEB_DIR` (default `web/dist`),
    /// `VESPER_DATA_DIR` (default `data`), `VESPER_ORIGIN` (default `http://localhost:8080`) and
    /// `VESPER_UPLOAD_QUOTA_MB` (default 10240) and `VESPER_PUSH_SUBJECT` (default: the origin
    /// when it is https, else unset, which turns push off).
    pub fn from_env() -> Result<Config, String> {
        Config::from_lookup(|k| std::env::var(k).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let addr = |k: &str, default: &str| {
            let v = get(k).unwrap_or_else(|| default.into());
            v.parse::<SocketAddr>().map_err(|e| format!("{k}={v}: {e}"))
        };
        let udp_bind = addr("VESPER_UDP_BIND", "0.0.0.0:40000")?;
        let num = |k: &str, default: u64| match get(k).filter(|v| !v.is_empty()) {
            Some(v) => v.parse::<u64>().map_err(|e| format!("{k}={v}: {e}")),
            None => Ok(default),
        };
        let udp_port_public = u16::try_from(num("VESPER_UDP_PORT_PUBLIC", udp_bind.port().into())?)
            .map_err(|e| format!("VESPER_UDP_PORT_PUBLIC: {e}"))?;
        let origin = get("VESPER_ORIGIN").unwrap_or_else(|| "http://localhost:8080".into());
        let origin =
            normalise_origin(&origin).map_err(|e| format!("VESPER_ORIGIN={origin}: {e}"))?;
        let quota_mb = num("VESPER_UPLOAD_QUOTA_MB", 10240)?;
        let push_subject = match get("VESPER_PUSH_SUBJECT").filter(|s| !s.is_empty()) {
            Some(s) if valid_push_subject(&s) => Some(s),
            Some(s) => {
                return Err(format!(
                    "VESPER_PUSH_SUBJECT={s}: must be mailto:<address> or https://<host>"
                ));
            }
            None => origin.starts_with("https://").then(|| origin.clone()),
        };
        Ok(Config {
            http_bind: addr("VESPER_HTTP_BIND", "0.0.0.0:8080")?,
            udp_bind,
            public_host: get("VESPER_PUBLIC_HOST").filter(|h| !h.is_empty()),
            udp_port_public,
            max_room_size: 12,
            ping_interval: Duration::from_secs(25),
            max_sockets: num("VESPER_MAX_SOCKETS", 256)? as usize,
            web_dir: get("VESPER_WEB_DIR")
                .filter(|d| !d.is_empty())
                .map_or_else(|| "web/dist".into(), PathBuf::from),
            data_dir: get("VESPER_DATA_DIR")
                .filter(|d| !d.is_empty())
                .map_or_else(|| "data".into(), PathBuf::from),
            secure_cookies: origin.starts_with("https://"),
            origin,
            upload_quota_bytes: quota_mb
                .checked_mul(1024 * 1024)
                .ok_or("VESPER_UPLOAD_QUOTA_MB is too large")?,
            push_subject,
        })
    }
}

/// RFC 8292 contact: `mailto:` or `https://`, printable ASCII without spaces or quotes, short.
fn valid_push_subject(s: &str) -> bool {
    (s.starts_with("mailto:") || s.starts_with("https://"))
        && s.len() <= 200
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

/// `http(s)://host[:port]` with no path, query or user part, in the form browsers send as
/// `Origin`: lowercase, no trailing slash, no default port.
pub(crate) fn normalise_origin(s: &str) -> Result<String, &'static str> {
    let lower = s.to_ascii_lowercase();
    let (scheme, rest, default_port) = if let Some(r) = lower.strip_prefix("https://") {
        ("https", r, "443")
    } else if let Some(r) = lower.strip_prefix("http://") {
        ("http", r, "80")
    } else {
        return Err("must start with http:// or https://");
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty()
        || rest.contains(['/', '?', '#', '@', '\\'])
        || rest.contains(char::is_whitespace)
    {
        return Err("must be scheme, host and optional port, with no path");
    }
    // The port is after the last ':' that is not inside an IPv6 literal's brackets.
    let (host, port) = match rest.rfind(':').filter(|&i| !rest[i..].contains(']')) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    if !valid_host(host) {
        return Err("host must be letters, digits, '.' and '-', or an [IPv6] literal");
    }
    match port {
        Some(p)
            if p.starts_with('0')
                || !p.bytes().all(|b| b.is_ascii_digit())
                || p.parse::<u16>().is_err() =>
        {
            Err("port is not a number from 1 to 65535")
        }
        Some(p) if p == default_port => Ok(format!("{scheme}://{host}")),
        _ => Ok(format!("{scheme}://{rest}")),
    }
}

/// A lowercase host name of `[a-z0-9.-]`, or a bracketed IPv6 literal of hex digits, `:` and `.`.
/// The origin goes into the CSP, so no `;`, quote, comma or space may get through.
fn valid_host(host: &str) -> bool {
    let ipv6 = |h: &str| {
        !h.is_empty()
            && h.bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
    };
    match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(inner) => ipv6(inner),
        None => {
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> Result<Config, String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
    }

    #[test]
    fn defaults_need_no_variables() {
        let c = lookup(&[]).unwrap();
        assert_eq!(c.udp_port_public, 40000);
        assert_eq!(c.max_room_size, 12);
        assert!(c.public_host.is_none());
        assert_eq!(c.max_sockets, 256);
        assert_eq!(c.web_dir, PathBuf::from("web/dist"));
    }

    #[test]
    fn origin_must_be_scheme_host_port() {
        let with = |o: &str| lookup(&[("VESPER_ORIGIN", o)]);
        let c = with("https://vesper.tfaucher.com").unwrap();
        assert_eq!(c.origin, "https://vesper.tfaucher.com");
        assert!(c.secure_cookies);
        for bad in [
            "ftp://x",
            "https://x/path",
            "x",
            "https://",
            "https://u@x",
            "http://x:port",
            "http://x?a",
        ] {
            assert!(with(bad).is_err(), "{bad}");
        }
        let c = lookup(&[]).unwrap();
        assert_eq!(c.origin, "http://localhost:8080");
        assert!(!c.secure_cookies);
        assert_eq!(c.data_dir, PathBuf::from("data"));
        assert_eq!(c.upload_quota_bytes, 10240 * 1024 * 1024);
    }

    #[test]
    fn origin_is_normalised_like_the_browser_header() {
        let o = |s: &str| normalise_origin(s).unwrap();
        assert_eq!(
            o("HTTPS://Vesper.TFaucher.com/"),
            "https://vesper.tfaucher.com"
        );
        assert_eq!(o("https://x:443"), "https://x");
        assert_eq!(o("http://x:80/"), "http://x");
        assert_eq!(o("http://x:443"), "http://x:443");
        assert_eq!(o("http://localhost:5173"), "http://localhost:5173");
        assert_eq!(o("http://[::1]:8080"), "http://[::1]:8080");
        assert_eq!(o("http://[::1]"), "http://[::1]");
        for bad in [
            "http://x:+80",
            "http://x:080",
            "http://x:0",
            "http://x:99999",
            // Anything that could add a CSP directive or source, or is not a bare origin.
            "https://a;img-src",
            "https://a;b",
            "https://a'b",
            "https://a,b",
            "https://a\"b",
            "https://a b",
            "https://a\tb",
            "https://a%62",
            "https://a*b",
            "https://*.example",
            "https://a\\b",
            "https://a\u{0}b",
            "https://é.example",
            "https://user@a",
            "https://user:pw@a",
            "https://a/path",
            "https://a?q=1",
            "https://a#frag",
            "https://",
            "https://:8080",
            "https://[]",
            "https://[::1;]",
            "https://[::1",
            "ftp://a",
            "ws://a",
        ] {
            assert!(normalise_origin(bad).is_err(), "{bad}");
        }
        let q = lookup(&[("VESPER_UPLOAD_QUOTA_MB", "")]).unwrap();
        assert_eq!(q.upload_quota_bytes, 10240 * 1024 * 1024);
    }

    #[test]
    fn push_subject_defaults_to_an_https_origin() {
        assert_eq!(lookup(&[]).unwrap().push_subject, None);
        let https = lookup(&[("VESPER_ORIGIN", "https://v.example")]).unwrap();
        assert_eq!(https.push_subject.as_deref(), Some("https://v.example"));
        let set = |v: &str| lookup(&[("VESPER_PUSH_SUBJECT", v)]);
        assert_eq!(
            set("mailto:ops@v.example").unwrap().push_subject.as_deref(),
            Some("mailto:ops@v.example")
        );
        for bad in [
            "http://v.example",
            "ops@v.example",
            "mailto:a b",
            "mailto:\"x\"",
        ] {
            assert!(set(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bad_address_is_an_error() {
        assert!(lookup(&[("VESPER_HTTP_BIND", "nope")]).is_err());
    }
}
