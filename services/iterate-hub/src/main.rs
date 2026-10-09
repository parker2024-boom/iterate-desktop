use iterate_hub::{
    protocol::MAX_BUDGET,
    server::{router, Hub},
    store::Store,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path = None;
    let mut listen = "127.0.0.1:8444".to_string();
    let mut budget = MAX_BUDGET;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => {
                path = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--db requires path"))?,
                ))
            }
            "--listen" => {
                listen = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--listen requires address"))?
                    .clone()
            }
            "--budget-bytes" => {
                budget = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--budget-bytes requires value"))?
                    .parse()?
            }
            "--help" => {
                println!("iterate-hub --db PATH [--listen 127.0.0.1:8444] [--budget-bytes <=1073741824]\nITERATE_HUB_ADMIN_HASH must contain sha256:<64hex>. TLS terminates at the existing verified HTTPS endpoint. No plaintext token CLI arguments.");
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument"),
        }
    }
    let path = path.ok_or_else(|| anyhow::anyhow!("--db is required"))?;
    // Deliberately loopback-only: deployment TLS must retain certificate identity
    // validation. No insecure TLS or public-cleartext escape switch is provided.
    let address: std::net::SocketAddr = listen.parse()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "listen must be loopback behind TLS proxy"
    );
    let admin_hash = std::env::var("ITERATE_HUB_ADMIN_HASH")
        .map_err(|_| anyhow::anyhow!("ITERATE_HUB_ADMIN_HASH is required"))?;
    let hub = Hub::new(Store::open(&path, budget)?, admin_hash)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("iterate-hub ready {}", listener.local_addr()?);
    axum::serve(listener, router(hub))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
