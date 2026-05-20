use anyhow::{Context, Result};
use rand::Rng;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::{
    str::FromStr,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio::time::sleep;

pub const JITO_MAINNET_HTTP_ENDPOINTS: [&str; 8] = [
    "https://amsterdam.mainnet.block-engine.jito.wtf",
    "https://dublin.mainnet.block-engine.jito.wtf",
    "https://frankfurt.mainnet.block-engine.jito.wtf",
    "https://london.mainnet.block-engine.jito.wtf",
    "https://ny.mainnet.block-engine.jito.wtf",
    "https://slc.mainnet.block-engine.jito.wtf",
    "https://singapore.mainnet.block-engine.jito.wtf",
    "https://tokyo.mainnet.block-engine.jito.wtf",
];

pub const JITO_MAINNET_TIP_ACCOUNTS: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

#[derive(Debug, Serialize)]
struct SendBundleRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: (Vec<String>, BundleEncoding),
}

#[derive(Debug, Serialize)]
struct BundleEncoding {
    encoding: &'static str,
}

#[derive(Debug, Serialize)]
struct SendTransactionRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: (String, TransactionEncoding),
}

#[derive(Debug, Serialize)]
struct TransactionEncoding {
    encoding: &'static str,
    #[serde(rename = "skipPreflight")]
    skip_preflight: bool,
}

#[derive(Debug, Deserialize)]
struct SendBundleResponse {
    jsonrpc: String,
    result: String,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    jsonrpc: String,
    error: ErrorDetail,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    code: i64,
    message: String,
}

pub struct JitoClient {
    client: Client,
    uuid: Option<String>,
    endpoints: Vec<String>,
    endpoint_index: AtomicUsize,
    min_txn_request_interval: Duration,
    last_txn_request: Mutex<Instant>,
}

impl JitoClient {
    pub fn new(uuid: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .build()
            .context("Failed to create HTTP client")?;
        let endpoints = configured_jito_endpoints();
        let min_txn_request_interval = configured_jito_txn_request_min_interval();
        let now = Instant::now();
        let last_txn_request = now.checked_sub(min_txn_request_interval).unwrap_or(now);
        tracing::info!("Jito 节点：{}", endpoints.join(","));
        tracing::info!(
            "Jito 请求限速：最小间隔={}毫秒",
            min_txn_request_interval.as_millis()
        );
        warm_jito_connections(client.clone(), uuid.clone(), endpoints.clone());

        Ok(Self {
            client,
            uuid,
            endpoints,
            endpoint_index: AtomicUsize::new(0),
            min_txn_request_interval,
            last_txn_request: Mutex::new(last_txn_request),
        })
    }

    async fn wait_for_txn_request_slot(&self) {
        if self.min_txn_request_interval.is_zero() {
            return;
        }
        loop {
            let wait_for = {
                let mut last_request = self.last_txn_request.lock().await;
                let elapsed = last_request.elapsed();
                if elapsed >= self.min_txn_request_interval {
                    *last_request = Instant::now();
                    return;
                }
                self.min_txn_request_interval - elapsed
            };
            tracing::debug!(
                "Jito request throttle: waiting {}ms before next request",
                wait_for.as_millis()
            );
            sleep(wait_for).await;
        }
    }

    fn get_next_endpoint(&self) -> &str {
        let index = self.endpoint_index.fetch_add(1, Ordering::Relaxed);
        self.endpoints[index % self.endpoints.len()].as_str()
    }

    pub fn random_tip_account() -> Result<Pubkey> {
        let index = rand::thread_rng().gen_range(0..JITO_MAINNET_TIP_ACCOUNTS.len());
        Pubkey::from_str(JITO_MAINNET_TIP_ACCOUNTS[index])
            .with_context(|| format!("invalid Jito tip account at index {}", index))
    }

    fn post_json(&self, url: &str) -> RequestBuilder {
        let request = self
            .client
            .post(url)
            .header("Content-Type", "application/json");
        if let Some(uuid) = &self.uuid {
            request.header("x-jito-auth", uuid)
        } else {
            request
        }
    }

    pub async fn send_bundle(&self, transactions: Vec<String>) -> Result<String> {
        if transactions.is_empty() {
            anyhow::bail!("Cannot send empty bundle");
        }

        let endpoint = self.get_next_endpoint();
        tracing::debug!(
            "发送 Jito 包：交易数={}，节点={}",
            transactions.len(),
            endpoint
        );

        let request = SendBundleRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "sendBundle".to_string(),
            params: (transactions, BundleEncoding { encoding: "base64" }),
        };

        let url = format!("{}/api/v1/bundles", endpoint);

        self.wait_for_txn_request_slot().await;
        let response = self
            .post_json(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send bundle to Jito")?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            tracing::error!("Jito 返回错误：状态={}，内容={}", status, body);
            anyhow::bail!("Jito API returned error: {}", body);
        }

        if let Ok(success) = serde_json::from_str::<SendBundleResponse>(&body) {
            tracing::debug!("Jito 包已发送：{}", success.result);
            return Ok(success.result);
        }

        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&body) {
            anyhow::bail!(
                "Jito error: {} (code: {})",
                error.error.message,
                error.error.code
            );
        }

        anyhow::bail!("Unexpected Jito response: {}", body);
    }

    pub async fn send_transaction(&self, transaction_base64: String) -> Result<String> {
        let endpoint = self.get_next_endpoint();
        tracing::debug!("通过 Jito 发送交易：节点={}", endpoint);

        let request = SendTransactionRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "sendTransaction".to_string(),
            params: (
                transaction_base64,
                TransactionEncoding {
                    encoding: "base64",
                    skip_preflight: true,
                },
            ),
        };

        let url = format!("{}/api/v1/transactions?bundleOnly=true", endpoint);
        self.wait_for_txn_request_slot().await;
        let response = self
            .post_json(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send transaction to Jito")?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            tracing::error!("Jito 交易返回错误：状态={}，内容={}", status, body);
            anyhow::bail!("Jito tx API returned error: {}", body);
        }

        if let Ok(success) = serde_json::from_str::<SendBundleResponse>(&body) {
            tracing::debug!("Jito 交易已提交：{}", success.result);
            return Ok(success.result);
        }

        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&body) {
            anyhow::bail!(
                "Jito tx error: {} (code: {})",
                error.error.message,
                error.error.code
            );
        }

        anyhow::bail!("Unexpected Jito transaction response: {}", body);
    }
}

fn configured_jito_endpoints() -> Vec<String> {
    let configured = std::env::var("JITO_ENDPOINTS")
        .ok()
        .or_else(|| std::env::var("JITO_ENDPOINT").ok())
        .map(|value| {
            value
                .split(|ch| matches!(ch, ',' | ';' | ' '))
                .map(str::trim)
                .filter(|endpoint| !endpoint.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|endpoints| !endpoints.is_empty());

    configured.unwrap_or_else(|| {
        JITO_MAINNET_HTTP_ENDPOINTS
            .iter()
            .map(|endpoint| (*endpoint).to_string())
            .collect()
    })
}

fn configured_jito_txn_request_min_interval() -> Duration {
    Duration::from_millis(
        std::env::var("JITO_TXN_REQUEST_MIN_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(550),
    )
}

fn warm_jito_connections(client: Client, uuid: Option<String>, endpoints: Vec<String>) {
    if std::env::var("JITO_DISABLE_WARMUP")
        .ok()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(false)
    {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    handle.spawn(async move {
        for endpoint in endpoints {
            let started = Instant::now();
            let url = format!("{}/api/v1/bundles", endpoint);
            let mut request = client.get(&url);
            if let Some(uuid) = &uuid {
                request = request.header("x-jito-auth", uuid);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let _ = response.bytes().await;
                    tracing::debug!(
                        "Jito 连接预热完成：节点={}，状态={}，耗时={}毫秒",
                        endpoint,
                        status,
                        started.elapsed().as_millis()
                    );
                }
                Err(error) => {
                    tracing::debug!(
                        "Jito 连接预热失败：节点={}，耗时={}毫秒，原因={}",
                        endpoint,
                        started.elapsed().as_millis(),
                        error
                    );
                }
            }
        }
    });
}
