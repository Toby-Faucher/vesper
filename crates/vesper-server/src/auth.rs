//! Passwords, names, session tokens, cookies, the Origin check and the client IP.

use std::net::{IpAddr, SocketAddr};

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use axum::http::{HeaderMap, HeaderValue, header};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use unicode_normalization::UnicodeNormalization;

/// At most this many argon2 hashes run at once (each takes 19 MiB and a core).
static HASHERS: Semaphore = Semaphore::const_new(2);

/// Verified when the user is unknown, so a missing account costs the same time as a wrong
/// password. Same params as `argon2()`; the password behind it was random and is not kept.
const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$pMdIF3sb8XkIiLI4opOZmw$SajFls9rbsD52Iq0BwBsLju02Uh8rlJe2h+MO1OZ2G8";

const COOKIE_SECURE: &str = "__Host-vesper_session";
const COOKIE_PLAIN: &str = "vesper_session";
/// 90 days, the session lifetime.
const COOKIE_MAX_AGE: u32 = 7_776_000;

fn argon2() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("static argon2 params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Trimmed and lowercased; only ASCII `[a-z0-9_]{3,24}` passes, so lookalikes cannot.
pub fn valid_username(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let ok = (3..=24).contains(&s.len())
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'));
    ok.then_some(s)
}

/// Format characters that hide or reorder text (soft hyphen, zero-width, bidi controls, BOM).
pub(crate) fn is_hidden_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// NFC, whitespace runs collapsed to one space, 1 to 32 chars, no control or hidden format
/// characters. Room names use the same rule.
pub fn valid_display_name(s: &str) -> Option<String> {
    clean_line(s, 1..=32)
}

/// A bio: the display-name rule, 0 to 160 chars (empty clears it).
pub fn valid_bio(s: &str) -> Option<String> {
    clean_line(s, 0..=160)
}

fn clean_line(s: &str, len: std::ops::RangeInclusive<usize>) -> Option<String> {
    let nfc: String = s.nfc().collect();
    let s = nfc.split_whitespace().collect::<Vec<_>>().join(" ");
    let ok = len.contains(&s.chars().count())
        && !s.chars().any(|c| c.is_control() || is_hidden_format(c));
    ok.then_some(s)
}

/// `#rrggbb`, lowercased.
pub fn valid_accent(s: &str) -> Option<String> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

pub fn valid_password(s: &str) -> bool {
    (8..=128).contains(&s.chars().count())
}

/// argon2id PHC string of `p`.
pub async fn hash_password(p: String) -> String {
    // The permit moves into the task so a cancelled request still holds it until the hash ends.
    let permit = HASHERS.acquire().await.expect("semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        argon2()
            .hash_password(p.as_bytes())
            .expect("argon2 hash with valid params")
            .to_string()
    })
    .await
    .expect("hash task panicked")
}

/// Checks `p` against `phc`; `None` (unknown user) checks a dummy hash and is always false.
pub async fn verify_password(p: String, phc: Option<String>) -> bool {
    let permit = HASHERS.acquire().await.expect("semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let ok = argon2()
            .verify_password(p.as_bytes(), phc.as_deref().unwrap_or(DUMMY_PHC))
            .is_ok();
        ok && phc.is_some()
    })
    .await
    .expect("verify task panicked")
}

/// A new session token (32 random bytes as 64 lowercase hex) and its hash for the store.
pub fn new_token() -> (String, [u8; 32]) {
    let token = vesper_store::random_hex(32);
    let hash = hash_token(&token).expect("random_hex gives lowercase hex");
    (token, hash)
}

/// SHA-256 of the token text. None unless `hex` is exactly 64 lowercase hex characters.
pub fn hash_token(hex: &str) -> Option<[u8; 32]> {
    let ok = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    ok.then(|| Sha256::digest(hex.as_bytes()).into())
}

fn cookie_name(secure: bool) -> &'static str {
    if secure { COOKIE_SECURE } else { COOKIE_PLAIN }
}

fn cookie(value: &str, max_age: u32, secure: bool) -> HeaderValue {
    let flag = if secure { "Secure; " } else { "" };
    let s = format!(
        "{}={value}; {flag}HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}",
        cookie_name(secure)
    );
    HeaderValue::from_str(&s).expect("cookie is printable ASCII")
}

/// `Set-Cookie` value for a session token from `new_token`. Panics on any other text (a byte that
/// is not visible ASCII), so never pass it a token taken from a request.
pub fn session_cookie(token: &str, secure: bool) -> HeaderValue {
    cookie(token, COOKIE_MAX_AGE, secure)
}

/// `Set-Cookie` value that deletes the session cookie.
pub fn clear_cookie(secure: bool) -> HeaderValue {
    cookie("", 0, secure)
}

/// The session cookie's value. Only the name for `secure` counts, so an http cookie cannot
/// stand in for the `__Host-` one.
pub fn cookie_token(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// True only when the `Origin` header is exactly `origin`.
pub fn origin_ok(headers: &HeaderMap, origin: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.as_bytes() == origin.as_bytes())
}

pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

/// The client address. Only when the TCP peer is loopback or private (the proxy chain:
/// Cloudflare, cloudflared, Caddy) are forwarding headers read: a valid `CF-Connecting-IP` first
/// (Cloudflare overwrites it), then the rightmost `X-Forwarded-For` entry, then the peer.
/// A LAN client that reaches the port without Cloudflare can set either header.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer = peer.ip().to_canonical();
    if !is_private(peer) {
        return peer;
    }
    let parse = |v: &str| v.trim().parse::<IpAddr>().ok();
    let cf = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(parse);
    let xff = || {
        headers
            .get_all("x-forwarded-for")
            .iter()
            .next_back()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(parse)
    };
    cf.or_else(xff).map_or(peer, |ip| ip.to_canonical())
}

/// A short device name such as "Firefox on Linux". Built from fixed words, never from the
/// header text, so it is always short and safe to show.
pub fn device_label(user_agent: Option<&str>) -> String {
    let ua = user_agent.unwrap_or("");
    let browser = [
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("Firefox/", "Firefox"),
        ("FxiOS/", "Firefox"),
        ("CriOS/", "Chrome"),
        ("Chrome/", "Chrome"),
        ("Safari/", "Safari"),
    ]
    .iter()
    .find(|(k, _)| ua.contains(k))
    .map_or("Unknown browser", |(_, v)| v);
    let os = [
        ("Android", "Android"),
        ("iPhone", "iOS"),
        ("iPad", "iOS"),
        ("Windows", "Windows"),
        ("CrOS", "ChromeOS"),
        ("Mac OS X", "macOS"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(k, _)| ua.contains(k));
    match os {
        Some((_, os)) => format!("{browser} on {os}"),
        None => browser.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_rules() {
        assert_eq!(valid_username(" Toby ").as_deref(), Some("toby"));
        for bad in ["ab", "a b", "ｔｏｂｙ", "tоby", "to\u{200B}by"] {
            assert_eq!(valid_username(bad), None, "{bad:?}");
        }
        assert!(valid_username(&"a".repeat(24)).is_some());
        assert_eq!(valid_username(&"a".repeat(25)), None);
    }

    #[test]
    fn display_name_refuses_format_chars() {
        for bad in [
            "Al\u{200B}ice",
            "\u{202E}ecila",
            "a\u{0007}",
            "   ",
            "\u{FEFF}",
        ] {
            assert_eq!(valid_display_name(bad), None, "{bad:?}");
        }
        assert_eq!(
            valid_display_name("Ame\u{0301}lie").as_deref(),
            Some("Am\u{00E9}lie")
        );
        assert_eq!(valid_display_name("  a   b ").as_deref(), Some("a b"));
        assert!(valid_display_name(&"é".repeat(32)).is_some());
        assert_eq!(valid_display_name(&"é".repeat(33)), None);
    }

    #[test]
    fn accent_and_password_rules() {
        assert_eq!(valid_accent("#7C8CFF").as_deref(), Some("#7c8cff"));
        for bad in ["7c8cff", "#7c8cf", "#7c8cffa", "#7c8cfg"] {
            assert_eq!(valid_accent(bad), None, "{bad:?}");
        }
        assert!(!valid_password("1234567"));
        assert!(valid_password("12345678"));
        assert!(valid_password(&"é".repeat(128)));
        assert!(!valid_password(&"a".repeat(129)));
    }

    #[tokio::test]
    async fn password_roundtrip_and_dummy() {
        let p = "test-password-1".to_string();
        let phc = hash_password(p.clone()).await;
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        assert!(verify_password(p.clone(), Some(phc.clone())).await);
        assert!(!verify_password("test-password-2".into(), Some(phc)).await);
        assert!(!verify_password(p, None).await);
        assert!(!verify_password("x".into(), Some("not a phc".into())).await);
    }

    #[test]
    fn dummy_phc_parses_with_our_params() {
        let h = argon2::PasswordHash::new(DUMMY_PHC).unwrap();
        assert_eq!(h.algorithm.as_str(), "argon2id");
        assert_eq!(h.params.to_string(), "m=19456,t=2,p=1");
    }

    #[test]
    fn token_hash_rejects_bad_hex() {
        let (token, hash) = new_token();
        assert_eq!(token.len(), 64);
        assert_eq!(hash_token(&token), Some(hash));
        assert_ne!(new_token().0, token);
        assert_eq!(hash_token(&token[..63]), None);
        assert_eq!(
            hash_token(&token.to_ascii_uppercase().replace('0', "A")),
            None
        );
        assert_eq!(hash_token(&"g".repeat(64)), None);
    }

    #[test]
    fn cookie_flags() {
        let token = "ab".repeat(32);
        let s = session_cookie(&token, true);
        let s = s.to_str().unwrap();
        assert!(
            s.starts_with(&format!("__Host-vesper_session={token};")),
            "{s}"
        );
        assert!(
            s.contains("Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=7776000"),
            "{s}"
        );
        let p = session_cookie(&token, false);
        let p = p.to_str().unwrap();
        assert!(p.starts_with("vesper_session="), "{p}");
        assert!(!p.contains("Secure"), "{p}");
        assert!(p.contains("HttpOnly; SameSite=Strict; Path=/; Max-Age=7776000"));
        let c = clear_cookie(true);
        assert!(c.to_str().unwrap().starts_with("__Host-vesper_session=;"));
        assert!(c.to_str().unwrap().ends_with("Max-Age=0"));
    }

    #[test]
    fn cookie_token_ignores_other_cookies() {
        let token = "cd".repeat(32);
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            format!("a=1; vesper_session={token}; b=2").parse().unwrap(),
        );
        assert_eq!(cookie_token(&h, false), Some(token.clone()));
        assert_eq!(cookie_token(&h, true), None);
        h.append(
            header::COOKIE,
            format!("__Host-vesper_session={token}").parse().unwrap(),
        );
        assert_eq!(cookie_token(&h, true), Some(token));
        assert_eq!(cookie_token(&HeaderMap::new(), false), None);
    }

    #[test]
    fn origin_check() {
        let o = "http://localhost:8080";
        let with = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::ORIGIN, v.parse().unwrap());
            h
        };
        assert!(origin_ok(&with(o), o));
        for bad in [
            "http://localhost:8080.evil.com",
            "null",
            "http://localhost:8081",
        ] {
            assert!(!origin_ok(&with(bad), o), "{bad}");
        }
        assert!(!origin_ok(&HeaderMap::new(), o));
    }

    #[test]
    fn client_ip_trusts_proxy_only_from_private_peer() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        let ip = |peer: &str, h: &HeaderMap| client_ip(peer.parse().unwrap(), h).to_string();
        assert_eq!(ip("127.0.0.1:5000", &h), "5.6.7.8");
        assert_eq!(ip("172.18.0.2:5000", &h), "5.6.7.8");
        assert_eq!(ip("[::ffff:127.0.0.1]:5000", &h), "5.6.7.8");
        assert_eq!(ip("[fd00::1]:5000", &h), "5.6.7.8");
        assert_eq!(ip("8.8.8.8:5000", &h), "8.8.8.8");
        h.insert("x-forwarded-for", "junk".parse().unwrap());
        assert_eq!(ip("127.0.0.1:5000", &h), "127.0.0.1");
        assert_eq!(ip("127.0.0.1:5000", &HeaderMap::new()), "127.0.0.1");
    }

    #[test]
    fn client_ip_prefers_cf_connecting_ip_from_private_peer() {
        let ip = |peer: &str, h: &HeaderMap| client_ip(peer.parse().unwrap(), h).to_string();
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 10.0.0.5".parse().unwrap());
        h.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
        assert_eq!(ip("127.0.0.1:5000", &h), "203.0.113.7");
        h.insert("cf-connecting-ip", " 2001:db8::7 ".parse().unwrap());
        assert_eq!(ip("10.0.0.2:5000", &h), "2001:db8::7");
        // Untrusted peer: every forwarding header is ignored.
        assert_eq!(ip("8.8.8.8:5000", &h), "8.8.8.8");
        // Malformed CF-Connecting-IP falls back to X-Forwarded-For, then the peer.
        for junk in ["junk", "1.2.3.4, 5.6.7.8", "", "1.2.3.4:80"] {
            h.insert("cf-connecting-ip", junk.parse().unwrap());
            assert_eq!(ip("127.0.0.1:5000", &h), "10.0.0.5", "{junk:?}");
        }
        h.remove("x-forwarded-for");
        assert_eq!(ip("127.0.0.1:5000", &h), "127.0.0.1");
    }

    #[test]
    fn device_labels() {
        let firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
        let chrome = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
        let safari = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
        assert_eq!(device_label(Some(firefox)), "Firefox on Linux");
        assert_eq!(device_label(Some(chrome)), "Chrome on Windows");
        assert_eq!(device_label(Some(safari)), "Safari on iOS");
        assert_eq!(device_label(Some("curl/8.0")), "Unknown browser");
        assert_eq!(device_label(None), "Unknown browser");
    }
}
