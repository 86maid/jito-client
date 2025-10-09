use anyhow::{Context, anyhow};
use base64::prelude::*;
use load_balancer::{LoadBalancer, interval::IntervalLoadBalancer};
use reqwest::{Client, ClientBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{net::IpAddr, sync::Arc, time::Duration};

pub use load_balancer;
pub use load_balancer::get_if_addrs;
pub use load_balancer::ip::{get_ip_list, get_ipv4_list, get_ipv6_list};
pub use reqwest;
pub use reqwest::Proxy;
pub use reqwest::header::HeaderMap;
pub use serde_json;

/// Builder for configuring and creating a `JitoClient`.
pub struct JitoClientBuilder {
    url: Vec<String>,
    broadcast: bool,
    interval: Duration,
    timeout: Option<Duration>,
    proxy: Option<Proxy>,
    headers: Option<HeaderMap>,
    ip: Vec<IpAddr>,
}

impl JitoClientBuilder {
    /// Creates a new `JitoClientBuilder` with default settings.
    pub fn new() -> Self {
        Self {
            url: vec!["https://mainnet.block-engine.jito.wtf".to_string()],
            broadcast: false,
            interval: Duration::ZERO,
            timeout: None,
            proxy: None,
            headers: None,
            ip: Vec::new(),
        }
    }

    /// Sets the target URLs for the client.
    pub fn url<T: IntoIterator<Item = impl AsRef<str>>>(mut self, url: T) -> Self {
        self.url = url.into_iter().map(|v| v.as_ref().to_string()).collect();
        self
    }

    /// Sets the interval duration between requests (0 = unlimited)
    /// For example, 5 requests per second = 200 ms interval.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Sets the local IP addresses to bind outgoing requests to.
    pub fn ip(mut self, ip: Vec<IpAddr>) -> Self {
        self.ip = ip;
        self
    }

    /// Broadcast each request to all configured URLs.
    pub fn broadcast(mut self, broadcast: bool) -> Self {
        self.broadcast = broadcast;
        self
    }

    /// Sets a timeout duration for requests.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets a proxy for the client.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Sets headers for the client.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Builds the `JitoClient` with the configured options.
    pub fn build(self) -> anyhow::Result<JitoClient> {
        let default_ip = self.ip.is_empty();

        let inner = if self.broadcast {
            let mut entries = Vec::new();

            if default_ip {
                let mut cb = ClientBuilder::new();

                if let Some(v) = self.timeout {
                    cb = cb.timeout(v);
                }

                if let Some(v) = self.proxy {
                    cb = cb.proxy(v);
                }

                if let Some(v) = self.headers {
                    cb = cb.default_headers(v);
                }

                entries.push((self.interval, Arc::new((self.url.clone(), cb.build()?))));
            } else {
                for ip in &self.ip {
                    let mut cb = ClientBuilder::new();

                    if let Some(v) = self.timeout {
                        cb = cb.timeout(v);
                    }

                    if let Some(v) = self.proxy.clone() {
                        cb = cb.proxy(v);
                    }

                    if let Some(v) = self.headers.clone() {
                        cb = cb.default_headers(v);
                    }

                    cb = cb.local_address(*ip);

                    entries.push((self.interval, Arc::new((self.url.clone(), cb.build()?))));
                }
            }

            JitoClientRef {
                broadcast: true,
                lb: IntervalLoadBalancer::new(entries),
            }
        } else {
            let mut entries = Vec::new();

            if default_ip {
                for url in &self.url {
                    let mut cb = ClientBuilder::new();

                    if let Some(v) = self.timeout {
                        cb = cb.timeout(v);
                    }

                    if let Some(v) = self.proxy.clone() {
                        cb = cb.proxy(v);
                    }

                    if let Some(v) = self.headers.clone() {
                        cb = cb.default_headers(v);
                    }

                    entries.push((self.interval, Arc::new((vec![url.clone()], cb.build()?))));
                }
            } else {
                for url in &self.url {
                    for ip in &self.ip {
                        let mut cb = ClientBuilder::new();

                        if let Some(v) = self.timeout {
                            cb = cb.timeout(v);
                        }

                        if let Some(v) = self.proxy.clone() {
                            cb = cb.proxy(v);
                        }

                        if let Some(v) = self.headers.clone() {
                            cb = cb.default_headers(v);
                        }

                        cb = cb.local_address(*ip);

                        entries.push((self.interval, Arc::new((vec![url.clone()], cb.build()?))));
                    }
                }
            }

            JitoClientRef {
                broadcast: false,
                lb: IntervalLoadBalancer::new(entries),
            }
        };

        Ok(JitoClient {
            inner: inner.into(),
        })
    }
}

struct JitoClientRef {
    broadcast: bool,
    lb: IntervalLoadBalancer<Arc<(Vec<String>, Client)>>,
}

/// Jito client for sending transactions and bundles.
#[derive(Clone)]
pub struct JitoClient {
    inner: Arc<JitoClientRef>,
}

impl JitoClient {
    /// Creates a new client with default settings.
    pub fn new() -> Self {
        JitoClientBuilder::new().build().unwrap()
    }

    /// Sends a raw request.
    pub async fn raw_send(&mut self, body: &serde_json::Value) -> anyhow::Result<Response> {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        if self.inner.broadcast {
            Ok(futures::future::select_ok(
                url.iter()
                    .map(|v| client.post(&format!("{}", v)).json(body).send()),
            )
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}", url[0]))
                .json(body)
                .send()
                .await?)
        }
    }

    /// Sends a raw request, use base_url + api_url.
    pub async fn raw_send_api(
        &mut self,
        api_url: impl AsRef<str>,
        body: &serde_json::Value,
    ) -> anyhow::Result<Response> {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}{}", v, api_url.as_ref()))
                    .json(body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}{}", url[0], api_url.as_ref()))
                .json(body)
                .send()
                .await?)
        }
    }

    /// Sends a single transaction and returns the HTTP response.
    pub async fn send_transaction(&self, tx: impl Serialize) -> anyhow::Result<Response> {
        let data = BASE64_STANDARD.encode(bincode::serialize(&tx)?);
        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        let (ref url, ref client) = *self.inner.lb.alloc().await;

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .query(&["bundleOnly", "true"])
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .query(&["bundleOnly", "true"])
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a transaction and returns the bundle ID from the response headers.
    pub async fn send_transaction_bid(&self, tx: impl Serialize) -> anyhow::Result<String> {
        Ok(self
            .send_transaction(tx)
            .await?
            .error_for_status()?
            .headers()
            .get("x-bundle-id")
            .ok_or_else(|| anyhow!("missing `x-bundle-id` header"))?
            .to_str()
            .map_err(|v| anyhow!("invalid `x-bundle-id` header: {}", v))?
            .to_string())
    }

    /// Sends a transaction without `bundleOnly` flag.
    pub async fn send_transaction_no_bundle_only(
        &self,
        tx: impl Serialize,
    ) -> anyhow::Result<Response> {
        let data = BASE64_STANDARD.encode(bincode::serialize(&tx)?);
        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        let (ref url, ref client) = *self.inner.lb.alloc().await;

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends multiple transactions as a bundle.
    pub async fn send_bundle<T: IntoIterator<Item = impl Serialize>>(
        &self,
        tx: T,
    ) -> anyhow::Result<Response> {
        let data = tx
            .into_iter()
            .map(|tx| {
                Ok(BASE64_STANDARD.encode(
                    bincode::serialize(&tx)
                        .map_err(|v| anyhow::anyhow!("failed to serialize tx: {}", v))?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendBundle",
            "params": [ data, { "encoding": "base64" } ]
        });

        let (ref url, ref client) = *self.inner.lb.alloc().await;

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/bundles", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/bundles", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a bundle and returns its bundle ID from the JSON response.
    pub async fn send_bundle_bid<T: IntoIterator<Item = impl Serialize>>(
        &self,
        tx: T,
    ) -> anyhow::Result<String> {
        self.send_bundle(tx)
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?["result"]
            .as_str()
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing bundle result"))
    }

    /// Sends a single transaction and returns the HTTP response, with lazy serialization.
    pub async fn send_transaction_lazy<T>(
        &self,
        tx: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<Response>
    where
        T: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = BASE64_STANDARD.encode(bincode::serialize(&tx.await?)?);

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .query(&["bundleOnly", "true"])
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .query(&["bundleOnly", "true"])
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a transaction and returns the bundle ID from the response headers, with lazy serialization.
    pub async fn send_transaction_bid_lazy<T>(
        &self,
        tx: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<String>
    where
        T: Serialize,
    {
        Ok(self
            .send_transaction_lazy(tx)
            .await?
            .error_for_status()?
            .headers()
            .get("x-bundle-id")
            .ok_or_else(|| anyhow!("missing `x-bundle-id` header"))?
            .to_str()
            .map_err(|v| anyhow!("invalid `x-bundle-id` header: {}", v))?
            .to_string())
    }

    /// Sends a transaction without `bundleOnly` flag, with lazy serialization.
    pub async fn send_transaction_no_bundle_only_lazy<T>(
        &self,
        tx: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<Response>
    where
        T: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = BASE64_STANDARD.encode(bincode::serialize(&tx.await?)?);

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends multiple transactions as a bundle, with lazy serialization.
    pub async fn send_bundle_lazy<T, S>(
        &self,
        tx: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<Response>
    where
        T: IntoIterator<Item = S>,
        S: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = tx
            .await?
            .into_iter()
            .map(|tx| {
                Ok(BASE64_STANDARD.encode(
                    bincode::serialize(&tx)
                        .map_err(|v| anyhow::anyhow!("failed to serialize tx: {}", v))?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendBundle",
            "params": [ data, { "encoding": "base64" } ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/bundles", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/bundles", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a bundle and returns its bundle ID from the JSON response, with lazy serialization.
    pub async fn send_bundle_bid_lazy<T, S>(
        &self,
        tx: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<String>
    where
        T: IntoIterator<Item = S>,
        S: Serialize,
    {
        self.send_bundle_lazy(tx)
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?["result"]
            .as_str()
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing bundle result"))
    }

    /// Sends a single transaction and returns the HTTP response, with lazy serialization.
    #[cfg(rustc_version_1_85_0)]
    pub async fn send_transaction_lazy_fn<F, T>(&self, callback: F) -> anyhow::Result<Response>
    where
        F: AsyncFnOnce(&Vec<String>, &Client) -> anyhow::Result<T>,
        T: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = BASE64_STANDARD.encode(bincode::serialize(&callback(url, client).await?)?);

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .query(&["bundleOnly", "true"])
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .query(&["bundleOnly", "true"])
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a transaction and returns the bundle ID from the response headers, with lazy serialization.
    #[cfg(rustc_version_1_85_0)]
    pub async fn send_transaction_bid_lazy_fn<F, T>(&self, callback: F) -> anyhow::Result<String>
    where
        F: AsyncFnOnce(&Vec<String>, &Client) -> anyhow::Result<T>,
        T: Serialize,
    {
        Ok(self
            .send_transaction_lazy_fn(callback)
            .await?
            .error_for_status()?
            .headers()
            .get("x-bundle-id")
            .ok_or_else(|| anyhow!("missing `x-bundle-id` header"))?
            .to_str()
            .map_err(|v| anyhow!("invalid `x-bundle-id` header: {}", v))?
            .to_string())
    }

    /// Sends a transaction without `bundleOnly` flag, with lazy serialization.
    #[cfg(rustc_version_1_85_0)]
    pub async fn send_transaction_no_bundle_only_lazy_fn<F, T>(
        &self,
        callback: F,
    ) -> anyhow::Result<Response>
    where
        F: AsyncFnOnce(&Vec<String>, &Client) -> anyhow::Result<T>,
        T: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = BASE64_STANDARD.encode(bincode::serialize(&callback(url, client).await?)?);

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/transactions", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/transactions", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends multiple transactions as a bundle, with lazy serialization.
    #[cfg(rustc_version_1_85_0)]
    pub async fn send_bundle_lazy_fn<F, T, S>(&self, callback: F) -> anyhow::Result<Response>
    where
        F: AsyncFnOnce(&Vec<String>, &Client) -> anyhow::Result<T>,
        T: IntoIterator<Item = S>,
        S: Serialize,
    {
        let (ref url, ref client) = *self.inner.lb.alloc().await;

        let data = callback(url, client)
            .await?
            .into_iter()
            .map(|tx| {
                Ok(BASE64_STANDARD.encode(
                    bincode::serialize(&tx)
                        .map_err(|v| anyhow::anyhow!("failed to serialize tx: {}", v))?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendBundle",
            "params": [ data, { "encoding": "base64" } ]
        });

        if self.inner.broadcast {
            Ok(futures::future::select_ok(url.iter().map(|v| {
                client
                    .post(&format!("{}/api/v1/bundles", v))
                    .json(&body)
                    .send()
            }))
            .await?
            .0)
        } else {
            Ok(client
                .post(&format!("{}/api/v1/bundles", url[0]))
                .json(&body)
                .send()
                .await?)
        }
    }

    /// Sends a bundle and returns its bundle ID from the JSON response, with lazy serialization.
    #[cfg(rustc_version_1_85_0)]
    pub async fn send_bundle_bid_lazy_fn<F, T, S>(&self, callback: F) -> anyhow::Result<String>
    where
        F: AsyncFnOnce(&Vec<String>, &Client) -> anyhow::Result<T>,
        T: IntoIterator<Item = S>,
        S: Serialize,
    {
        self.send_bundle_lazy_fn(callback)
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?["result"]
            .as_str()
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing bundle result"))
    }
}

/// Represents Jito tip data.
#[derive(Debug, Clone, Deserialize)]
pub struct JitoTip {
    pub landed_tips_25th_percentile: f64,
    pub landed_tips_50th_percentile: f64,
    pub landed_tips_75th_percentile: f64,
    pub landed_tips_95th_percentile: f64,
    pub landed_tips_99th_percentile: f64,
    pub ema_landed_tips_50th_percentile: f64,
}

/// Fetches the current Jito tip from the public API.
pub async fn get_jito_tip(client: Client) -> anyhow::Result<JitoTip> {
    Ok(client
        .get("https://bundles.jito.wtf/api/v1/bundles/tip_floor")
        .send()
        .await?
        .json::<Vec<JitoTip>>()
        .await?
        .get(0)
        .context("get_jito_tip: empty response")?
        .clone())
}

/// Represents the result of querying bundle statuses.
#[derive(Debug, Deserialize)]
pub struct BundleResult {
    pub context: serde_json::Value,
    pub value: Option<Vec<BundleStatus>>,
}

#[derive(Debug, Deserialize)]
pub struct BundleStatus {
    pub bundle_id: String,
    pub transactions: Option<Vec<String>>,
    pub slot: Option<u64>,
    pub confirmation_status: Option<String>,
    pub err: Option<serde_json::Value>,
}

/// Fetches statuses of multiple bundles.
pub async fn get_bundle_statuses<T: IntoIterator<Item = impl AsRef<str>>>(
    client: Client,
    bundle: T,
) -> anyhow::Result<BundleResult> {
    #[derive(Debug, Deserialize)]
    struct RpcResponse {
        result: BundleResult,
    }

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBundleStatuses",
        "params": [bundle.into_iter().map(|v| v.as_ref().to_string()).collect::<Vec<_>>()],
    });

    Ok(client
        .post("https://mainnet.block-engine.jito.wtf/api/v1/getBundleStatuses")
        .json(&payload)
        .send()
        .await?
        .json::<RpcResponse>()
        .await?
        .result)
}

/// Represents in-flight bundle status.
#[derive(Debug, Deserialize)]
pub struct InflightBundleStatus {
    pub bundle_id: String,
    pub status: String,
    pub landed_slot: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct InflightBundleResult {
    pub context: serde_json::Value,
    pub value: Option<Vec<InflightBundleStatus>>,
}

/// Fetches statuses of in-flight bundles.
pub async fn get_inflight_bundle_statuses<T: IntoIterator<Item = impl AsRef<str>>>(
    client: Client,
    bundle: T,
) -> anyhow::Result<InflightBundleResult> {
    #[derive(Debug, Deserialize)]
    struct InflightRpcResponse {
        result: InflightBundleResult,
    }

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getInflightBundleStatuses",
        "params": [bundle.into_iter().map(|v| v.as_ref().to_string()).collect::<Vec<_>>()],
    });

    Ok(client
        .post("https://mainnet.block-engine.jito.wtf/api/v1/getInflightBundleStatuses")
        .json(&payload)
        .send()
        .await?
        .json::<InflightRpcResponse>()
        .await?
        .result)
}

pub async fn test_ip(ip: IpAddr) -> anyhow::Result<IpAddr> {
    reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(3))
        .local_address(ip)
        .build()?
        .get("https://crates.io")
        .send()
        .await?;

    Ok(ip)
}

pub async fn test_all_ip() -> Vec<anyhow::Result<IpAddr>> {
    match get_ip_list() {
        Ok(v) => futures::future::join_all(v.into_iter().map(|v| test_ip(v))).await,
        Err(_) => Vec::new(),
    }
}

pub async fn test_all_ipv4() -> Vec<anyhow::Result<IpAddr>> {
    match get_ipv4_list() {
        Ok(v) => futures::future::join_all(v.into_iter().map(|v| test_ip(v))).await,
        Err(_) => Vec::new(),
    }
}

pub async fn test_all_ipv6() -> Vec<anyhow::Result<IpAddr>> {
    match get_ipv6_list() {
        Ok(v) => futures::future::join_all(v.into_iter().map(|v| test_ip(v))).await,
        Err(_) => Vec::new(),
    }
}

pub fn serialize_tx(tx: impl Serialize) -> anyhow::Result<String> {
    Ok(BASE64_STANDARD.encode(bincode::serialize(&tx)?))
}

pub fn serialize_tx_vec<T: IntoIterator<Item = impl Serialize>>(
    tx: T,
) -> anyhow::Result<Vec<String>> {
    tx.into_iter()
        .map(|tx| {
            Ok(BASE64_STANDARD.encode(
                bincode::serialize(&tx)
                    .map_err(|v| anyhow::anyhow!("failed to serialize tx: {}", v))?,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()
}
