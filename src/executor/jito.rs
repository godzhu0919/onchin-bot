use anyhow::{Context, Result};
use rand::Rng;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

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
    endpoint_index: std::sync::atomic::AtomicUsize,
}

impl JitoClient {
    pub fn new(uuid: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            uuid,
            endpoint_index: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn get_next_endpoint(&self) -> &'static str {
        let index = self
            .endpoint_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        JITO_MAINNET_HTTP_ENDPOINTS[index % JITO_MAINNET_HTTP_ENDPOINTS.len()]
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
