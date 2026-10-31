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
    let result = match vesper_server::start(cfg).await {
        Ok((_, serving)) => serving
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e))),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        eprintln!("vesper-server: {e}");
        std::process::exit(1);
    }
}
