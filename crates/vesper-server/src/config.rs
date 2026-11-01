use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone)]
pub struct Config {
    pub http_bind: SocketAddr,
    pub udp_bind: SocketAddr,
    /// Hostname resolved (and re-resolved) to the addresses the SFU advertises. Unset: the SFU
    /// advertises its own bound address, which needs a concrete `udp_bind` IP (not 0.0.0.0).
    pub public_host: Option<String>,
    /// UDP port clients reach the SFU on through the router forward.
    pub udp_port_public: u16,
    // shortcut: one shared secret stands in for accounts; remove it with the accounts plan.
    pub dev_key: String,
    pub max_room_size: usize,
    /// A socket that has not joined within this long is closed.
    pub join_deadline: Duration,
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
}

/// Like a derived Debug, but `dev_key` prints as `***` so logs never carry the secret.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("http_bind", &self.http_bind)
            .field("udp_bind", &self.udp_bind)
            .field("public_host", &self.public_host)
            .field("udp_port_public", &self.udp_port_public)
            .field("dev_key", &"***")
            .field("max_room_size", &self.max_room_size)
            .field("join_deadline", &self.join_deadline)
            .field("ping_interval", &self.ping_interval)
            .field("max_sockets", &self.max_sockets)
            .field("web_dir", &self.web_dir)
            .field("data_dir", &self.data_dir)
            .field("origin", &self.origin)
            .field("secure_cookies", &self.secure_cookies)
            .field("upload_quota_bytes", &self.upload_quota_bytes)
            .finish()
    }
}

impl Config {
    /// `VESPER_HTTP_BIND` (default `0.0.0.0:8080`), `VESPER_UDP_BIND` (default `0.0.0.0:40000`),
    /// `VESPER_PUBLIC_HOST` (optional), `VESPER_UDP_PORT_PUBLIC` (default: the `VESPER_UDP_BIND`
    /// port), `VESPER_JOIN_DEADLINE_SECS` (default 10), `VESPER_MAX_SOCKETS` (default 256), `VESPER_WEB_DIR` (default `web/dist`) and
    /// `VESPER_DEV_KEY` (required, non-empty), `VESPER_DATA_DIR` (default `data`), `VESPER_ORIGIN`
    /// (default `http://localhost:8080`) and `VESPER_UPLOAD_QUOTA_MB` (default 10240).
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
        let dev_key = get("VESPER_DEV_KEY")
            .filter(|k| !k.is_empty())
            .ok_or("VESPER_DEV_KEY must be set to a non-empty shared key")?;
        let origin = get("VESPER_ORIGIN").unwrap_or_else(|| "http://localhost:8080".into());
        let origin =
            normalise_origin(&origin).map_err(|e| format!("VESPER_ORIGIN={origin}: {e}"))?;
        let quota_mb = num("VESPER_UPLOAD_QUOTA_MB", 10240)?;
        Ok(Config {
            http_bind: addr("VESPER_HTTP_BIND", "0.0.0.0:8080")?,
            udp_bind,
            public_host: get("VESPER_PUBLIC_HOST").filter(|h| !h.is_empty()),
            udp_port_public,
            dev_key,
            max_room_size: 12,
            join_deadline: Duration::from_secs(num("VESPER_JOIN_DEADLINE_SECS", 10)?),
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
        })
    }
}

/// `http(s)://host[:port]` with no path, query or user part, in the form browsers send as
/// `Origin`: lowercase, no trailing slash, no default port.
fn normalise_origin(s: &str) -> Result<String, &'static str> {
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
    if host.is_empty() {
        return Err("host is empty");
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
    fn dev_key_is_required_and_non_empty() {
        assert!(lookup(&[]).is_err());
        assert!(lookup(&[("VESPER_DEV_KEY", "")]).is_err());
        let c = lookup(&[("VESPER_DEV_KEY", "k")]).unwrap();
        assert_eq!(c.dev_key, "k");
        assert_eq!(c.udp_port_public, 40000);
        assert_eq!(c.max_room_size, 12);
        assert!(c.public_host.is_none());
        assert_eq!(c.join_deadline, Duration::from_secs(10));
        assert_eq!(c.max_sockets, 256);
        assert_eq!(c.web_dir, PathBuf::from("web/dist"));
        let c = lookup(&[("VESPER_DEV_KEY", "k"), ("VESPER_JOIN_DEADLINE_SECS", "3")]).unwrap();
        assert_eq!(c.join_deadline, Duration::from_secs(3));
    }

    #[test]
    fn debug_hides_the_dev_key() {
        let c = lookup(&[("VESPER_DEV_KEY", "s3cret-key")]).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains("s3cret-key"), "{d}");
        assert!(d.contains("dev_key: \"***\""), "{d}");
    }

    #[test]
    fn origin_must_be_scheme_host_port() {
        let with = |o: &str| lookup(&[("VESPER_DEV_KEY", "k"), ("VESPER_ORIGIN", o)]);
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
        let c = lookup(&[("VESPER_DEV_KEY", "k")]).unwrap();
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
        ] {
            assert!(normalise_origin(bad).is_err(), "{bad}");
        }
        let q = lookup(&[("VESPER_DEV_KEY", "k"), ("VESPER_UPLOAD_QUOTA_MB", "")]).unwrap();
        assert_eq!(q.upload_quota_bytes, 10240 * 1024 * 1024);
    }

    #[test]
    fn bad_address_is_an_error() {
        assert!(lookup(&[("VESPER_DEV_KEY", "k"), ("VESPER_HTTP_BIND", "nope")]).is_err());
    }
}
