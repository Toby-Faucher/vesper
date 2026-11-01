//! Static serving of the web app: SPA fallback, MIME types, and no escape from the web dir.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vesper_server::config::Config;

/// A fresh temp tree: `<root>/secret.txt` (outside) and `<root>/web/` (served).
fn site() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "vesper-static-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let web = root.join("web");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(root.join("secret.txt"), "TOPSECRET").unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>INDEX").unwrap();
    std::fs::write(web.join("assets/a.wasm"), b"\0asm\x01\0\0\0").unwrap();
    std::fs::write(web.join("assets/w.js"), "export {}").unwrap();
    std::fs::write(web.join(".env"), "DOTSECRET").unwrap();
    web
}

async fn start(web_dir: std::path::PathBuf) -> SocketAddr {
    let data_dir = web_dir.with_file_name("data");
    let (addr, _) = vesper_server::start(Config {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        udp_bind: "127.0.0.1:0".parse().unwrap(),
        public_host: None,
        udp_port_public: 0,
        max_room_size: 12,
        ping_interval: Duration::from_secs(30),
        max_sockets: 4,
        web_dir,
        data_dir,
        origin: "http://localhost:8080".into(),
        secure_cookies: false,
        upload_quota_bytes: 0,
    })
    .await
    .unwrap();
    addr
}

/// Raw GET so the path is sent byte for byte (no client-side normalisation).
async fn get(addr: SocketAddr, path: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut b = Vec::new();
    s.read_to_end(&mut b).await.unwrap();
    String::from_utf8_lossy(&b).into_owned()
}

fn header_has(resp: &str, h: &str) -> bool {
    resp.to_ascii_lowercase().contains(&h.to_ascii_lowercase())
}

#[tokio::test]
async fn serves_index_and_falls_back_for_spa_paths() {
    let addr = start(site()).await;
    for p in ["/", "/index.html", "/room/abc"] {
        let r = get(addr, p).await;
        assert!(r.starts_with("HTTP/1.1 200"), "{p}: {r}");
        assert!(r.ends_with("INDEX"), "{p}: {r}");
        assert!(header_has(&r, "content-type: text/html"), "{p}: {r}");
        assert!(
            header_has(&r, "x-content-type-options: nosniff"),
            "{p}: {r}"
        );
    }
}

#[tokio::test]
async fn health_and_ws_take_precedence() {
    let addr = start(site()).await;
    let r = get(addr, "/health").await;
    assert!(
        r.starts_with("HTTP/1.1 200") && r.ends_with("\r\n\r\nok"),
        "{r}"
    );
    // A plain GET /ws is not an upgrade: the ws route answers (4xx), the SPA index does not.
    let r = get(addr, "/ws").await;
    assert!(
        !r.ends_with("INDEX") && !r.starts_with("HTTP/1.1 200"),
        "{r}"
    );
    // A real upgrade reaches the ws route too: without Origin and session it is refused there.
    match tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 403),
        other => panic!("expected 403, got {other:?}"),
    }
}

#[tokio::test]
async fn mime_types() {
    let addr = start(site()).await;
    let r = get(addr, "/assets/a.wasm").await;
    assert!(header_has(&r, "content-type: application/wasm"), "{r}");
    let r = get(addr, "/assets/w.js").await;
    assert!(header_has(&r, "content-type: text/javascript"), "{r}");
    assert!(header_has(&r, "x-content-type-options: nosniff"), "{r}");
}

#[tokio::test]
async fn cannot_read_outside_dir_or_dotfiles() {
    let addr = start(site()).await;
    for p in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/..%2fsecret.txt",
        "/%2e%2e%2fsecret.txt",
        "/assets/../../secret.txt",
        "/.env",
        "/%2eenv",
        "/assets/%2e./%2e./secret.txt",
    ] {
        let r = get(addr, p).await;
        assert!(
            !r.contains("TOPSECRET") && !r.contains("DOTSECRET"),
            "{p}: {r}"
        );
    }
    // Directories are never listed.
    let r = get(addr, "/assets/").await;
    assert!(!r.contains("a.wasm"), "{r}");
}

#[tokio::test]
async fn cache_control_and_extension_paths_do_not_fall_back() {
    let web = site();
    std::fs::create_dir_all(web.join("dsp")).unwrap();
    std::fs::write(web.join("dsp/g.js"), "x").unwrap();
    let addr = start(web).await;
    let cc = |r: &str| r.to_ascii_lowercase().contains("cache-control: no-cache");
    for p in ["/", "/index.html", "/room/abc"] {
        assert!(cc(&get(addr, p).await), "{p}");
    }
    let r = get(addr, "/assets/w.js").await;
    assert!(
        header_has(&r, "cache-control: public, max-age=31536000, immutable"),
        "{r}"
    );
    let r = get(addr, "/assets/a.wasm").await;
    assert!(header_has(&r, "immutable"), "{r}");
    let r = get(addr, "/assets/old-hash.js").await;
    assert!(
        r.starts_with("HTTP/1.1 404") && !r.contains("INDEX") && !header_has(&r, "immutable"),
        "{r}"
    );
    for p in ["/dsp/missing.wasm", "/dsp/model.tar.gz", "/api/missing"] {
        let r = get(addr, p).await;
        assert!(
            r.starts_with("HTTP/1.1 404") && !r.contains("INDEX"),
            "{p}: {r}"
        );
    }
    assert!(cc(&get(addr, "/dsp/g.js").await));
    // API answers are per user: never stored by any cache.
    let r = get(addr, "/api/me").await;
    assert!(header_has(&r, "cache-control: no-store"), "{r}");
}
