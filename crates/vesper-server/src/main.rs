#![forbid(unsafe_code)]

use tracing_subscriber::EnvFilter;
use vesper_server::config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cfg = Config::from_env().unwrap_or_else(|e| {
        eprintln!("vesper-server: {e}");
        std::process::exit(1);
    });
    let (_, serving) = vesper_server::start(cfg).await.expect("server starts");
    serving.await.expect("server task").expect("server runs");
}
