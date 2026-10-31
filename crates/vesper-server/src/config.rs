use std::net::SocketAddr;

#[derive(Debug, Clone)]
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
}

impl Config {
    /// `VESPER_HTTP_BIND` (default `0.0.0.0:8080`), `VESPER_UDP_BIND` (default `0.0.0.0:40000`),
    /// `VESPER_PUBLIC_HOST` (optional), `VESPER_UDP_PORT_PUBLIC` (default: the `VESPER_UDP_BIND`
    /// port) and `VESPER_DEV_KEY` (required, non-empty).
    pub fn from_env() -> Result<Config, String> {
        Config::from_lookup(|k| std::env::var(k).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let addr = |k: &str, default: &str| {
            let v = get(k).unwrap_or_else(|| default.into());
            v.parse::<SocketAddr>().map_err(|e| format!("{k}={v}: {e}"))
        };
        let udp_bind = addr("VESPER_UDP_BIND", "0.0.0.0:40000")?;
        let udp_port_public = match get("VESPER_UDP_PORT_PUBLIC") {
            Some(v) => v
                .parse()
                .map_err(|e| format!("VESPER_UDP_PORT_PUBLIC={v}: {e}"))?,
            None => udp_bind.port(),
        };
        let dev_key = get("VESPER_DEV_KEY")
            .filter(|k| !k.is_empty())
            .ok_or("VESPER_DEV_KEY must be set to a non-empty shared key")?;
        Ok(Config {
            http_bind: addr("VESPER_HTTP_BIND", "0.0.0.0:8080")?,
            udp_bind,
            public_host: get("VESPER_PUBLIC_HOST").filter(|h| !h.is_empty()),
            udp_port_public,
            dev_key,
            max_room_size: 12,
        })
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
    }

    #[test]
    fn bad_address_is_an_error() {
        assert!(lookup(&[("VESPER_DEV_KEY", "k"), ("VESPER_HTTP_BIND", "nope")]).is_err());
    }
}
