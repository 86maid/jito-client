use anyhow::{Context, anyhow};
use base64::prelude::*;
use futures::future::{join_all, select_ok};
use load_balancer::{LoadBalancer, time::TimeLoadBalancer};
use reqwest::header::HeaderName;
use reqwest::{Client, ClientBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::IpAddr, sync::Arc, time::Duration};

pub use get_if_addrs::get_if_addrs;
pub use load_balancer;
pub use reqwest;
pub use reqwest::Proxy;
pub use reqwest::StatusCode;
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
    headers_with_separator: Option<String>,
    broadcast_status: Option<StatusCode>,
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
            headers_with_separator: None,
            broadcast_status: None,
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
    pub fn ip<T: IntoIterator<Item = IpAddr>>(mut self, ip: T) -> Self {
        self.ip = ip.into_iter().collect();
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

    /// Sets the status code considered successful in broadcast mode for select_ok.
    pub fn broadcast_status(mut self, broadcast_status: StatusCode) -> Self {
        self.broadcast_status = Some(broadcast_status);
        self
    }

    /// Sets custom headers encoded in the URL using a separator.
    ///
    /// Example: `.headers_with_separator(":").url(&["https://google.com:key1=value1&key2=value2"])`
    ///
    /// parses `key1=value1&key2=value2` as headers.
    pub fn headers_with_separator(mut self, headers_with_separator: impl AsRef<str>) -> Self {
        self.headers_with_separator = Some(headers_with_separator.as_ref().to_string());
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
                lb: TimeLoadBalancer::new(entries),
                broadcast_status: self.broadcast_status,
                headers_with_separator: self.headers_with_separator,
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
                lb: TimeLoadBalancer::new(entries),
                broadcast_status: self.broadcast_status,
                headers_with_separator: self.headers_with_separator,
            }
        };

        Ok(JitoClient {
            inner: inner.into(),
        })
    }
}

struct JitoClientRef {
    lb: TimeLoadBalancer<Arc<(Vec<String>, Client)>>,
    broadcast_status: Option<StatusCode>,
    headers_with_separator: Option<String>,
}

/// Jito client for sending transactions and bundles.
#[derive(Clone)]
pub struct JitoClient {
    inner: Arc<JitoClientRef>,
}

#[derive(Clone)]
pub struct JitoClientOnce {
    inner: Arc<JitoClientRef>,
    entry: Arc<(Vec<String>, Client)>,
}

impl JitoClientOnce {
    fn split_url<'a>(&'a self, url: &'a str) -> anyhow::Result<(&'a str, HeaderMap)> {
        let mut headers = HeaderMap::new();

        if let Some(v) = &self.inner.headers_with_separator {
            if let Some((a, b)) = url.split_once(v) {
                for (k, v) in form_urlencoded::parse(b.as_bytes()) {
                    headers.insert(HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
                }

                return Ok((a, headers));
            }
        }

        Ok((url, headers))
    }

    pub fn url(&self) -> &Vec<String> {
        &self.entry.0
    }

    pub fn client(&self) -> &Client {
        &self.entry.1
    }

    /// Sends a raw request.
    pub async fn raw_send(&self, body: &Value) -> anyhow::Result<Response> {
        let (ref url, ref client) = *self.entry;

        if url.len() > 1 {
            Ok(select_ok(url.iter().map(|v| {
                Box::pin(async move {
                    let (url, headers) = self.split_url(v)?;
                    let response = client.post(url).headers(headers).json(&body).send().await?;

                    if let Some(v) = self.inner.broadcast_status {
                        if response.status() == v {
                            Ok(response)
                        } else {
                            Err(anyhow!(
                                "status code mismatch: expected {}, found {}",
                                v,
                                response.status()
                            ))
                        }
                    } else {
                        Ok(response)
                    }
                })
            }))
            .await?
            .0)
        } else {
            let (url, headers) = self.split_url(&url[0])?;

            Ok(client.post(url).headers(headers).json(body).send().await?)
        }
    }

    /// Sends a raw request, use base_url + api_url.
    pub async fn raw_send_api(
        &self,
        api_url: impl AsRef<str>,
        body: &Value,
    ) -> anyhow::Result<Response> {
        let (ref url, ref client) = *self.entry;
        let api_url = api_url.as_ref();

        if url.len() > 1 {
            Ok(select_ok(url.iter().map(|v| {
                Box::pin(async move {
                    let (base, headers) = self.split_url(v)?;
                    let full_url = format!("{}{}", base, api_url);
                    let response = client
                        .post(&full_url)
                        .headers(headers)
                        .json(&body)
                        .send()
                        .await?;

                    if let Some(v) = self.inner.broadcast_status {
                        if response.status() == v {
                            Ok(response)
                        } else {
                            Err(anyhow!(
                                "status code mismatch: expected {}, found {}",
                                v,
                                response.status()
                            ))
                        }
                    } else {
                        Ok(response)
                    }
                })
            }))
            .await?
            .0)
        } else {
            let (base, headers) = self.split_url(&url[0])?;
            let full_url = format!("{}{}", base, api_url);

            Ok(client
                .post(&full_url)
                .headers(headers)
                .json(body)
                .send()
                .await?)
        }
    }

    /// Sends a single transaction and returns the HTTP response.
    pub async fn send_transaction(&self, tx: impl Serialize) -> anyhow::Result<Response> {
        let data = serialize_tx(tx)?;

        let body = &json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        let (ref url, ref client) = *self.entry;

        if url.len() > 1 {
            Ok(select_ok(url.iter().map(|v| {
                Box::pin(async move {
                    let (base, headers) = self.split_url(v)?;
                    let full_url = format!("{}/api/v1/transactions", base);
                    let response = client
                        .post(&full_url)
                        .headers(headers)
                        .query(&[("bundleOnly", "true")])
                        .json(body)
                        .send()
                        .await?;

                    if let Some(v) = self.inner.broadcast_status {
                        if response.status() == v {
                            Ok(response)
                        } else {
                            Err(anyhow!(
                                "status code mismatch: expected {}, found {}",
                                v,
                                response.status()
                            ))
                        }
                    } else {
                        Ok(response)
                    }
                })
            }))
            .await?
            .0)
        } else {
            let (base, headers) = self.split_url(&url[0])?;
            let full_url = format!("{}/api/v1/transactions", base);

            Ok(client
                .post(&full_url)
                .headers(headers)
                .query(&[("bundleOnly", "true")])
                .json(body)
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
        let data = serialize_tx(tx)?;
        let body = &json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                data, { "encoding": "base64" }
            ]
        });

        let (ref url, ref client) = *self.entry;

        if url.len() > 1 {
            Ok(select_ok(url.iter().map(|v| {
                Box::pin(async move {
                    let (base, headers) = self.split_url(v)?;
                    let full_url = format!("{}/api/v1/transactions", base);
                    let response = client
                        .post(&full_url)
                        .headers(headers)
                        .json(body)
                        .send()
                        .await?;

                    if let Some(v) = self.inner.broadcast_status {
                        if response.status() == v {
                            Ok(response)
                        } else {
                            Err(anyhow!(
                                "status code mismatch: expected {}, found {}",
                                v,
                                response.status()
                            ))
                        }
                    } else {
                        Ok(response)
                    }
                })
            }))
            .await?
            .0)
        } else {
            let (base, headers) = self.split_url(&url[0])?;
            let full_url = format!("{}/api/v1/transactions", base);

            Ok(client
                .post(&full_url)
                .headers(headers)
                .json(body)
                .send()
                .await?)
        }
    }

    /// Sends multiple transactions as a bundle.
    pub async fn send_bundle<T: IntoIterator<Item = impl Serialize>>(
        &self,
        tx: T,
    ) -> anyhow::Result<Response> {
        let data = serialize_tx_vec(tx)?;

        let body = &json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendBundle",
            "params": [ data, { "encoding": "base64" } ]
        });

        let (ref url, ref client) = *self.entry;

        if url.len() > 1 {
            Ok(select_ok(url.iter().map(|v| {
                Box::pin(async move {
                    let (base, headers) = self.split_url(v)?;
                    let full_url = format!("{}/api/v1/bundles", base);
                    let response = client
                        .post(&full_url)
                        .headers(headers)
                        .json(body)
                        .send()
                        .await?;

                    if let Some(v) = self.inner.broadcast_status {
                        if response.status() == v {
                            Ok(response)
                        } else {
                            Err(anyhow!(
                                "status code mismatch: expected {}, found {}",
                                v,
                                response.status()
                            ))
                        }
                    } else {
                        Ok(response)
                    }
                })
            }))
            .await?
            .0)
        } else {
            let (base, headers) = self.split_url(&url[0])?;
            let full_url = format!("{}/api/v1/bundles", base);

            Ok(client
                .post(&full_url)
                .headers(headers)
                .json(body)
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
            .json::<Value>()
            .await?["result"]
            .as_str()
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing bundle result"))
    }
}

impl JitoClient {
    /// Creates a new client with default settings.
    pub fn new() -> Self {
        JitoClientBuilder::new().build().unwrap()
    }

    // Alloc a new client with Load Balancer.
    pub async fn alloc(&self) -> JitoClientOnce {
        JitoClientOnce {
            inner: self.inner.clone(),
            entry: self.inner.lb.alloc().await,
        }
    }

    // Try alloc a new client with Load Balancer.
    pub fn try_alloc(&self) -> Option<JitoClientOnce> {
        Some(JitoClientOnce {
            inner: self.inner.clone(),
            entry: self.inner.lb.try_alloc()?,
        })
    }

    /// Sends a raw request.
    pub async fn raw_send(&self, body: &Value) -> anyhow::Result<Response> {
        self.alloc().await.raw_send(body).await
    }

    /// Sends a raw request, use base_url + api_url.
    pub async fn raw_send_api(
        &self,
        api_url: impl AsRef<str>,
        body: &Value,
    ) -> anyhow::Result<Response> {
        self.alloc().await.raw_send_api(api_url, body).await
    }

    /// Sends a single transaction and returns the HTTP response.
    pub async fn send_transaction(&self, tx: impl Serialize) -> anyhow::Result<Response> {
        self.alloc().await.send_transaction(tx).await
    }

    /// Sends a transaction and returns the bundle ID from the response headers.
    pub async fn send_transaction_bid(&self, tx: impl Serialize) -> anyhow::Result<String> {
        self.alloc().await.send_transaction_bid(tx).await
    }

    /// Sends a transaction without `bundleOnly` flag.
    pub async fn send_transaction_no_bundle_only(
        &self,
        tx: impl Serialize,
    ) -> anyhow::Result<Response> {
        self.alloc().await.send_transaction_no_bundle_only(tx).await
    }

    /// Sends multiple transactions as a bundle.
    pub async fn send_bundle<T: IntoIterator<Item = impl Serialize>>(
        &self,
        tx: T,
    ) -> anyhow::Result<Response> {
        self.alloc().await.send_bundle(tx).await
    }

    /// Sends a bundle and returns its bundle ID from the JSON response.
    pub async fn send_bundle_bid<T: IntoIterator<Item = impl Serialize>>(
        &self,
        tx: T,
    ) -> anyhow::Result<String> {
        self.alloc().await.send_bundle_bid(tx).await
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
    pub context: Value,
    pub value: Option<Vec<BundleStatus>>,
}

#[derive(Debug, Deserialize)]
pub struct BundleStatus {
    pub bundle_id: String,
    pub transactions: Option<Vec<String>>,
    pub slot: Option<u64>,
    pub confirmation_status: Option<String>,
    pub err: Option<Value>,
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
    pub context: Value,
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

/// Get all non-loopback IP addresses of the machine.
pub fn get_ip_list() -> anyhow::Result<Vec<IpAddr>> {
    Ok(get_if_addrs()?
        .into_iter()
        .filter(|v| !v.is_loopback())
        .map(|v| v.ip())
        .collect::<Vec<_>>())
}

/// Get all non-loopback IPv4 addresses of the machine.
pub fn get_ipv4_list() -> anyhow::Result<Vec<IpAddr>> {
    Ok(get_if_addrs()?
        .into_iter()
        .filter(|v| !v.is_loopback() && v.ip().is_ipv4())
        .map(|v| v.ip())
        .collect::<Vec<_>>())
}

/// Get all non-loopback IPv6 addresses of the machine.
pub fn get_ipv6_list() -> anyhow::Result<Vec<IpAddr>> {
    Ok(get_if_addrs()?
        .into_iter()
        .filter(|v| !v.is_loopback() && v.ip().is_ipv6())
        .map(|v| v.ip())
        .collect::<Vec<_>>())
}

pub async fn test_ip(ip: IpAddr) -> anyhow::Result<IpAddr> {
    reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(3))
        .local_address(ip)
        .build()?
        .get("https://apple.com")
        .send()
        .await?;

    Ok(ip)
}

pub async fn test_all_ip() -> Vec<anyhow::Result<IpAddr>> {
    match get_ip_list() {
        Ok(v) => join_all(v.into_iter().map(|v| test_ip(v))).await,
        Err(_) => Vec::new(),
    }
}

pub async fn test_all_ipv4() -> Vec<anyhow::Result<IpAddr>> {
    match get_ipv4_list() {
        Ok(v) => join_all(v.into_iter().map(|v| test_ip(v))).await,
        Err(_) => Vec::new(),
    }
}

pub async fn test_all_ipv6() -> Vec<anyhow::Result<IpAddr>> {
    match get_ipv6_list() {
        Ok(v) => join_all(v.into_iter().map(|v| test_ip(v))).await,
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
        .map(|tx| Ok(BASE64_STANDARD.encode(bincode::serialize(&tx)?)))
        .collect::<anyhow::Result<Vec<_>>>()
}
