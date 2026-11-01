#![forbid(unsafe_code)]

use tracing_subscriber::EnvFilter;
use vesper_server::config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // stderr, so subcommand output on stdout stays clean for pipes.
        .with_writer(std::io::stderr)
        .init();
    let cfg = Config::from_env().unwrap_or_else(|e| {
        eprintln!("vesper-server: {e}");
        std::process::exit(1);
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    match vesper_server::cli::run(&args, &cfg).await {
        Ok(None) => {}
        Ok(Some(out)) => {
            println!("{out}");
            return;
        }
        Err(e) => {
            eprintln!("vesper-server: {e}");
            std::process::exit(1);
        }
    }
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
