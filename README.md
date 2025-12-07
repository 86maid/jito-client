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
use jito_client::*;

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
use jito_client::*;

#[tokio::main]
async fn main() {
    // If you have 1 IP, max 3 request/sec
    // If you have 10 IPs, max 30 requests/sec
    let client = JitoClientBuilder::new()
        // Sets the interval duration between requests (0 = unlimited)
        // For example, 5 requests per second = 200 ms interval
        .interval(Duration::from_millis(1000))
        // Enables sending requests via multiple IPs
        .ip(get_ipv4_list().unwrap())
        // Sets the target URLs for the client
        .url(&[
            "https://amsterdam.mainnet.block-engine.jito.wtf",
            "https://tokyo.mainnet.block-engine.jito.wtf",
            "https://london.mainnet.block-engine.jito.wtf",
        ])
        .build()
        .unwrap();

    // If you have 1 IP, max 1 request/sec, broadcasted to all URLs
    // If you have 10 IPs, max 10 requests/sec, broadcasted to all URLs
    let client = JitoClientBuilder::new()
        // Broadcast each request to all configured URLs
        .broadcast(true)
        .broadcast_status(StatusCode::OK)
        .interval(Duration::from_millis(1000))
        .ip(get_ipv4_list().unwrap())
        .url(&[
            "https://amsterdam.mainnet.block-engine.jito.wtf",
            "https://tokyo.mainnet.block-engine.jito.wtf",
            "https://london.mainnet.block-engine.jito.wtf",
        ])
        .build()
        .unwrap();

    let client = JitoClientBuilder::new()
        // Sets a timeout duration for requests
        .timeout(Duration::from_millis(5000))
        // Sets an header for the client
        .headers(HeaderMap::new())
        // Sets an proxy for the client
        // warning: .ip(get_ipv4_list().unwrap()) must be on the same network segment as the proxy
        // 192.168.0.2 ----X----> 127.0.0.1:7890
        // 192.168.0.2 ---------> 192.168.0.2:7890
        .proxy(Proxy::all("http://127.0.0.1:7890"))
        .build()
        .unwrap();

    // Sets custom headers encoded in the URL using a separator.
    //
    // Example: 
    // 
    //   .headers_with_separator("~")
    //   .url(&["https://google.com~key1=value1&key2=value2"])
    //
    // parses `key1=value1&key2=value2` as headers.
    let client = JitoClientBuilder::new()
        .headers_with_separator("~")
        .url(&[
            "https://amsterdam.mainnet.block-engine.jito.wtf~auth=123",
            "https://tokyo.mainnet.block-engine.jito.wtf~auth=123",
            "https://london.mainnet.block-engine.jito.wtf~auth=123",
        ])
        .build()
        .unwrap();

    // Limit request rate and concurrency
    //
    // If the request limit is 25 req/sec:
    //
    // interval = 1000ms / 25 = 40
    // semaphore = 1000ms / interval = 25
    let client = JitoClientBuilder::new()
        .interval(Duration::from_millis(40))
        // No need to set manually. defaults to 1000ms / interval * url.len()
        // .semaphore(Arc::new(Semaphore::new(25)))
        .url(&[
            "https://amsterdam.mainnet.block-engine.jito.wtf",
        ])
        .build()
        .unwrap();

    loop {
        client.clone().spawn(async move {
            // do something...
        }).await; // must await, otherwise the semaphore will not work
    }
}
```

## Tip

The Jito server may enforce rate limits at multiple levels: global, IP, and wallet.

* **Global**: For example, the server may handle up to 10000 requests per second in total, regardless of which IP or wallet the requests come from.

* **IP**: For example, each IP can make up to 5 requests per second.

* **Wallet**: For example, each wallet can submit up to 5 transaction requests per second.