# Jito Client

A Rust client for interacting with the Jito network, supporting **rate limiting**, **multi-IP usage**, and **broadcasting requests** to multiple endpoints.

## Features

- Send single transactions or transaction bundles.
- Rate limiting (requests per second).
- Support for multiple IPv4 and IPv6 addresses.
- Broadcast requests to multiple URLs simultaneously.
- Easy-to-use builder pattern for client configuration.
- Fetch Jito tips and bundle statuses.

## Usage

### Default

```rust
use jito_client::JitoClient;

#[tokio::main]
async fn main() {
  let client = JitoClient::new();

  // Send a bundle of transactions
  let response = client.send_bundle(&["tx"]).await.unwrap();
  println!("{:?}", response);

  // Send a bundle and get the bundle ID
  let bid = client.send_bundle_bid(&["tx"]).await.unwrap();
  println!("{:?}", bid);
}

```

### Customized

```rust
use jito_client::JitoClientBuilder;

#[tokio::main]
async fn main() {
  // If you have 10 IPs, this is equivalent to sending 30 requests per second
  let client = JitoClientBuilder::new()
      // Sets the request rate limit (requests per second, 0 = unlimited)
      .rate(1)
      // Enables sending requests via multiple IPs (both IPv4 and IPv6)
      .multi_ip(true)
      // Sets the target URLs for the client
      .url(&[
          "https://amsterdam.mainnet.block-engine.jito.wtf",
          "https://tokyo.mainnet.block-engine.jito.wtf",
          "https://london.mainnet.block-engine.jito.wtf",
      ])
      .build()
      .unwrap();
}
```

### Broadcast

```rust
use jito_client::JitoClientBuilder;

#[tokio::main]
async fn main() {
  // Each transaction/bundle sent will go to all URLs in parallel
  let client = JitoClientBuilder::new()
      // Configures the client to send requests to all URLs in parallel
      .broadcast(true)
      // Sets the target URLs for the client
      .url(&[
          "https://amsterdam.mainnet.block-engine.jito.wtf",
          "https://tokyo.mainnet.block-engine.jito.wtf",
          "https://london.mainnet.block-engine.jito.wtf",
      ])
      .build()
      .unwrap();
}
```