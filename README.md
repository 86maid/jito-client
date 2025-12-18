# Jito Client — High-Performance Rust Client (Refactored)

A fast, async Rust client for Jito Block-Engine with built-in **per-IP rate limiting**, **multi-IP binding**, **broadcast mode**, and **flexible header injection**.

## Features

- Send single transactions or bundles (auto base64 + bincode serialization)
- Precise **per-IP / per-endpoint rate limiting** via `interval`
- **Broadcast mode** — fire the same request to all endpoints simultaneously
- Bind to multiple local IPv4/IPv6 addresses (bypass IP-based limits)
- Proxy, timeout, custom headers, auth-key via URL
- Header injection directly in URL (e.g. `https://ny.mainnet...:auth=secret`)
- Zero-cost `alloc()` for high concurrency while respecting limits
- Helper functions: tip floor, bundle status, inflight status, etc.

# Quick Start

```rust
use jito_client::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // default mainnet endpoint, no rate limit
    let client = JitoClient::new();

    let response = client
        .send_transaction(tx)
        .await
        .unwrap();

    let bundle_id = client
        .send_bundle_bid(&[tx1, tx2, tx3])
        .await
        .unwrap();
    
    println!("{}", bundle_id);

    Ok(())
}
```

# Advanced Configuration (Builder)

```rust
use jito_client::*;
use std::time::Duration;
use reqwest::{Proxy, StatusCode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // If you have 1 IP, → max 15 req/sec (5req/s * 1 IP * 3 URL)
    // If you have 10 IPs → max 150 req/sec (5req/s * 10 IP * 3 URL)
    let client = JitoClientBuilder::new()
        .interval(Duration::from_millis(200))        // 5 req/s per IP
        .ip(get_ipv4_list()?)
        .url(&[
            "https://mainnet.block-engine.jito.wtf",
            "https://amsterdam.mainnet.block-engine.jito.wtf",
            "https://tokyo.mainnet.block-engine.jito.wtf",
        ])
        .build()?;

    // Broadcasted to all URLs
    // If you have 1 IP  → max 5 req/sec (5 req/s * 1 IP)
    let client = JitoClientBuilder::new()
        .broadcast(true)
        .interval(Duration::from_millis(200))        // 5 req/s per IP
        .ip(get_ipv4_list()?)
        .url(&[
            "https://amsterdam.mainnet.block-engine.jito.wtf",
            "https://frankfurt.mainnet.block-engine.jito.wtf",
            "https://ny.mainnet.block-engine.jito.wtf",
        ])
        .build()?;

    // Set headers
    // separator is `~`
    // parses `key1=value1&key2=value2` as headers.
    let client = JitoClientBuilder::new()
        .headers_with_separator("~")
        .url(&[
            "https://mainnet.block-engine.jito.wtf~key=value",
            "https://amsterdam.mainnet.block-engine.jito.wtf~key=value",
        ])
        .build()?;

    // Proxy setup (your IP list must be on same subnet as proxy!)
    let client = JitoClientBuilder::new()
        .timeout(Duration::from_secs(8))
        .proxy(Proxy::all("http://127.0.0.1:7890")?)
        .interval(Duration::from_millis(200))
        .ip(get_ipv4_list()?)
        .build()?;

    // High-frequency spam loop
    loop {
        let once = client.alloc().await;

        tokio::spawn(async move {
            once.send_bundle(&[tx]).await;
        });
    }
}
```