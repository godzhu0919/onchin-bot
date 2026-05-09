use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};

const RPC_RETRY_ATTEMPTS: usize = 3;
const RPC_RETRY_BASE_DELAY_MS: u64 = 150;

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: (&'a [&'a str], RpcConfig),
}

#[derive(Debug, Serialize)]
struct RpcConfig {
    encoding: &'static str,
    commitment: &'static str,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<RpcResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RpcResult {
    value: Vec<Option<RpcAccount>>,
}

#[derive(Debug, Deserialize)]
struct RpcAccount {
    owner: String,
    data: (String, String),
}

#[derive(Debug, Deserialize)]
struct BalanceResponse {
    result: Option<BalanceResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct BalanceResult {
    value: u64,
}

#[derive(Debug, Deserialize)]
struct SlotResponse {
    result: Option<u64>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    result: Option<String>,
    error: Option<serde_json::Value>,
}

fn is_retryable_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

async fn post_rpc_json_with_retries<T: DeserializeOwned>(
    client: &reqwest::Client,
    rpc_url: &str,
    request: &impl Serialize,
    request_context: &str,
    http_context: &str,
    parse_context: &str,
) -> Result<T> {
    for attempt in 0..RPC_RETRY_ATTEMPTS {
        let response = client
            .post(rpc_url)
            .json(request)
            .send()
            .await
            .with_context(|| request_context.to_string());

        match response {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    if attempt + 1 < RPC_RETRY_ATTEMPTS && is_retryable_http_status(status) {
                        sleep(Duration::from_millis(
                            RPC_RETRY_BASE_DELAY_MS * (attempt as u64 + 1),
                        ))
                        .await;
                        continue;
                    }
                    anyhow::bail!("{}: {}", http_context, status);
                }

                return response
                    .json::<T>()
                    .await
                    .with_context(|| parse_context.to_string());
            }
            Err(error) => {
                if attempt + 1 < RPC_RETRY_ATTEMPTS {
                    sleep(Duration::from_millis(
                        RPC_RETRY_BASE_DELAY_MS * (attempt as u64 + 1),
                    ))
                    .await;
                    continue;
                }
                return Err(error);
            }
        }
    }

    unreachable!("RPC retry loop should always return")
}

pub async fn get_multiple_accounts_data(
    rpc_url: &str,
    accounts: &[String],
) -> Result<HashMap<String, Vec<u8>>> {
    let client = reqwest::Client::new();
    let mut out = HashMap::new();

    for chunk in accounts.chunks(100) {
        let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getMultipleAccounts",
            params: (
                &refs,
                RpcConfig {
                    encoding: "base64",
                    commitment: "processed",
                },
            ),
        };

        let response: RpcResponse = post_rpc_json_with_retries(
            &client,
            rpc_url,
            &request,
            &format!("request getMultipleAccounts chunk size {}", refs.len()),
            "getMultipleAccounts HTTP status",
            "parse getMultipleAccounts response",
        )
        .await?;

        if let Some(error) = response.error {
            anyhow::bail!("getMultipleAccounts RPC error: {}", error);
        }

        let values = response
            .result
            .context("getMultipleAccounts missing result")?
            .value;

        for (account, value) in chunk.iter().zip(values.into_iter()) {
            let Some(value) = value else {
                continue;
            };

            if value.data.1 != "base64" {
                anyhow::bail!("unexpected account data encoding for {}", account);
            }

            let data = general_purpose::STANDARD
                .decode(value.data.0)
                .with_context(|| format!("decode account data for {}", account))?;
            out.insert(account.clone(), data);
        }
    }

    Ok(out)
}

pub async fn get_multiple_accounts_owners(
    rpc_url: &str,
    accounts: &[String],
) -> Result<HashMap<String, String>> {
    let client = reqwest::Client::new();
    let mut out = HashMap::new();

    for chunk in accounts.chunks(100) {
        let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getMultipleAccounts",
            params: (
                &refs,
                RpcConfig {
                    encoding: "base64",
                    commitment: "processed",
                },
            ),
        };

        let response: RpcResponse = post_rpc_json_with_retries(
            &client,
            rpc_url,
            &request,
            &format!("request getMultipleAccounts chunk size {}", refs.len()),
            "getMultipleAccounts HTTP status",
            "parse getMultipleAccounts response",
        )
        .await?;

        if let Some(error) = response.error {
            anyhow::bail!("getMultipleAccounts RPC error: {}", error);
        }

        let values = response
            .result
            .context("getMultipleAccounts missing result")?
            .value;

        for (account, value) in chunk.iter().zip(values.into_iter()) {
            let Some(value) = value else {
                continue;
            };
            out.insert(account.clone(), value.owner);
        }
    }

    Ok(out)
}

pub async fn get_balance_lamports(rpc_url: &str, account: &str) -> Result<u64> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBalance",
        "params": [
            account,
            {
                "commitment": "processed"
            }
        ]
    });

    let response: BalanceResponse = post_rpc_json_with_retries(
        &client,
        rpc_url,
        &request,
        "request getBalance",
        "getBalance HTTP status",
        "parse getBalance response",
    )
    .await?;

    if let Some(error) = response.error {
        anyhow::bail!("getBalance RPC error: {}", error);
    }

    response
        .result
        .context("getBalance missing result")
        .map(|result| result.value)
}

pub async fn get_slot(rpc_url: &str) -> Result<u64> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSlot",
        "params": [
            {
                "commitment": "processed"
            }
        ]
    });

    let response: SlotResponse = post_rpc_json_with_retries(
        &client,
        rpc_url,
        &request,
        "request getSlot",
        "getSlot HTTP status",
        "parse getSlot response",
    )
    .await?;

    if let Some(error) = response.error {
        anyhow::bail!("getSlot RPC error: {}", error);
    }

    response.result.context("getSlot missing result")
}

pub async fn get_health(rpc_url: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getHealth"
    });

    let response: HealthResponse = post_rpc_json_with_retries(
        &client,
        rpc_url,
        &request,
        "request getHealth",
        "getHealth HTTP status",
        "parse getHealth response",
    )
    .await?;

    if let Some(error) = response.error {
        anyhow::bail!("getHealth RPC error: {}", error);
    }

    response.result.context("getHealth missing result")
}
