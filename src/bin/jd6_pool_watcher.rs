use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::Path,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;

const DEFAULT_WALLET: &str = "JD6rVaerbyz6wjQ433nrw6bFTgFrp46MiYmi8EtUAfsG";
const DEFAULT_RPC_URL: &str = "http://127.0.0.1:8899";
const DEFAULT_OUTPUT_PATH: &str = "validated_pools.jsonl";
const DEFAULT_STATE_PATH: &str = "validated_pools.snapshot";
const DEFAULT_POLL_SECS: u64 = 300;
const DEFAULT_SIGNATURE_LIMIT: usize = 60;
const DEFAULT_MAX_MISSES: u32 = 2;
const DEFAULT_MAX_TOKENS: usize = 20;
const DEFAULT_MAX_PUMPSWAP_PER_TOKEN: usize = 2;
const DEFAULT_MAX_METEORA_PER_TOKEN: usize = 4;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const USD1_MINT: &str = "USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB";
const JUP_MINT: &str = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";
const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const PUMPSWAP_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const METEORA_LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];

#[derive(Debug, Clone)]
struct WatcherConfig {
    rpc_url: String,
    wallet: String,
    output_path: String,
    state_path: String,
    poll_secs: u64,
    signature_limit: usize,
    max_misses: u32,
    max_tokens: usize,
    max_pumpswap_per_token: usize,
    max_meteora_per_token: usize,
    quote_mints: HashSet<String>,
    excluded_token_mints: HashSet<String>,
}

impl WatcherConfig {
    fn from_env() -> Self {
        Self {
            rpc_url: env_string("RPC_HTTP_URL", DEFAULT_RPC_URL),
            wallet: env_string("JD6_WATCHED_WALLET", DEFAULT_WALLET),
            output_path: env_string("JD6_POOL_OUTPUT", DEFAULT_OUTPUT_PATH),
            state_path: env_string("JD6_POOL_STATE", DEFAULT_STATE_PATH),
            poll_secs: env_u64("JD6_POOL_POLL_SECS", DEFAULT_POLL_SECS),
            signature_limit: env_usize("JD6_POOL_SIGNATURE_LIMIT", DEFAULT_SIGNATURE_LIMIT),
            max_misses: env_u32("JD6_POOL_MAX_MISSES", DEFAULT_MAX_MISSES),
            max_tokens: env_usize("JD6_POOL_MAX_TOKENS", DEFAULT_MAX_TOKENS),
            max_pumpswap_per_token: env_usize(
                "JD6_POOL_MAX_PUMPSWAP_PER_TOKEN",
                DEFAULT_MAX_PUMPSWAP_PER_TOKEN,
            ),
            max_meteora_per_token: env_usize(
                "JD6_POOL_MAX_METEORA_PER_TOKEN",
                DEFAULT_MAX_METEORA_PER_TOKEN,
            ),
            quote_mints: env_csv_set("JD6_POOL_QUOTE_MINTS", &[SOL_MINT]),
            excluded_token_mints: env_csv_set(
                "JD6_POOL_EXCLUDED_TOKEN_MINTS",
                &[USDC_MINT, USDT_MINT, USD1_MINT, JUP_MINT],
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatePool {
    address: String,
    venue: String,
    token_mint: String,
    quote_mint: String,
    first_seen_unix: u64,
    last_seen_unix: u64,
    last_seen_slot: u64,
    hits: u64,
    misses: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WatcherState {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    source: String,
    wallet: String,
    updated_unix: u64,
    pools: Vec<StatePool>,
}

#[derive(Debug, Clone)]
struct FreshPool {
    address: String,
    venue: &'static str,
    token_mint: String,
    quote_mint: String,
    last_seen_slot: u64,
    hits: u64,
}

#[derive(Debug, Default)]
struct PollReport {
    successful_transactions: usize,
    candidate_accounts: usize,
    routeable_tokens: usize,
    fresh_pools: usize,
    active_pools: usize,
}

#[derive(Debug, Deserialize)]
struct RpcValueResponse<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SignatureInfo {
    signature: String,
    #[serde(default)]
    slot: Option<u64>,
    #[serde(default)]
    err: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AccountListResult {
    value: Vec<Option<RpcAccount>>,
}

#[derive(Debug, Deserialize)]
struct RpcAccount {
    owner: String,
    data: (String, String),
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    let config = WatcherConfig::from_env();
    let run_once = env::args().any(|arg| arg == "--once");

    tracing::info!(
        "JD6 验证快照更新器启动：钱包={}，间隔={}秒，输出={}",
        config.wallet,
        config.poll_secs,
        config.output_path
    );

    loop {
        match run_once_cycle(&config).await {
            Ok(report) => {
                tracing::info!(
                    "JD6 池子更新完成：成功交易={}，候选账户={}，可路由币种={}，本轮池子={}，保留池子={}",
                    report.successful_transactions,
                    report.candidate_accounts,
                    report.routeable_tokens,
                    report.fresh_pools,
                    report.active_pools
                );
                tracing::info!(
                    "提示：这个进程只更新验证快照，不扫描利润；交易端只热加载 validated snapshot"
                );
            }
            Err(error) => tracing::warn!("JD6 池子更新失败：{}", error),
        }

        if run_once {
            break;
        }
        sleep(Duration::from_secs(config.poll_secs.max(10))).await;
    }

    Ok(())
}

async fn run_once_cycle(config: &WatcherConfig) -> Result<PollReport> {
    let signatures = fetch_successful_signatures(config).await?;
    let mut candidate_accounts = HashSet::new();
    let mut account_slots: HashMap<String, u64> = HashMap::new();

    for signature in &signatures {
        let Some(transaction) =
            fetch_transaction_json(&config.rpc_url, &signature.signature).await?
        else {
            continue;
        };

        let mut pubkeys = HashSet::new();
        collect_pubkeys_from_json(&transaction, &mut pubkeys);
        let slot = signature.slot.unwrap_or_default();
        for pubkey in pubkeys {
            candidate_accounts.insert(pubkey.clone());
            account_slots
                .entry(pubkey)
                .and_modify(|existing| *existing = (*existing).max(slot))
                .or_insert(slot);
        }
    }

    let accounts = candidate_accounts.into_iter().collect::<Vec<_>>();
    let rpc_accounts = fetch_accounts(&config.rpc_url, &accounts).await?;
    let raw_pools = extract_pools(config, &rpc_accounts, &account_slots);
    let routeable_pools = select_routeable_pools(config, raw_pools);
    let state = update_state(config, routeable_pools)?;
    write_active_addresses(&config.output_path, &state)?;
    write_state(&config.state_path, &state)?;
    log_routeable_pool_summary(&state);

    Ok(PollReport {
        successful_transactions: signatures.len(),
        candidate_accounts: accounts.len(),
        routeable_tokens: routeable_token_count(&state),
        fresh_pools: state.pools.iter().filter(|pool| pool.misses == 0).count(),
        active_pools: state.pools.len(),
    })
}

async fn fetch_successful_signatures(config: &WatcherConfig) -> Result<Vec<SignatureInfo>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSignaturesForAddress",
        "params": [
            config.wallet,
            {
                "limit": config.signature_limit,
                "commitment": "confirmed"
            }
        ]
    });

    let response: RpcValueResponse<Vec<SignatureInfo>> =
        post_rpc_json(&config.rpc_url, &request).await?;
    if let Some(error) = response.error {
        anyhow::bail!("getSignaturesForAddress RPC error: {}", error);
    }

    Ok(response
        .result
        .unwrap_or_default()
        .into_iter()
        .filter(|signature| signature.err.is_none())
        .collect())
}

async fn fetch_transaction_json(rpc_url: &str, signature: &str) -> Result<Option<Value>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0
            }
        ]
    });

    let response: RpcValueResponse<Value> = post_rpc_json(rpc_url, &request).await?;
    if let Some(error) = response.error {
        tracing::debug!("跳过交易：签名={}，原因={}", signature, error);
        return Ok(None);
    }

    Ok(response.result)
}

async fn fetch_accounts(rpc_url: &str, accounts: &[String]) -> Result<HashMap<String, RpcAccount>> {
    let mut out = HashMap::new();
    for chunk in accounts.chunks(100) {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getMultipleAccounts",
            "params": [
                chunk,
                {
                    "encoding": "base64",
                    "commitment": "processed"
                }
            ]
        });

        let response: RpcValueResponse<AccountListResult> =
            post_rpc_json(rpc_url, &request).await?;
        if let Some(error) = response.error {
            anyhow::bail!("getMultipleAccounts RPC error: {}", error);
        }

        let values = response
            .result
            .context("getMultipleAccounts missing result")?
            .value;
        for (address, account) in chunk.iter().zip(values.into_iter()) {
            if let Some(account) = account {
                out.insert(address.clone(), account);
            }
        }
    }
    Ok(out)
}

async fn post_rpc_json<T: DeserializeOwned>(rpc_url: &str, request: &Value) -> Result<T> {
    let client = reqwest::Client::new();
    let response = client
        .post(rpc_url)
        .json(request)
        .send()
        .await
        .context("request RPC")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("RPC HTTP status: {}", status);
    }

    response.json::<T>().await.context("parse RPC response")
}

fn collect_pubkeys_from_json(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(text) => {
            if Pubkey::from_str(text).is_ok() {
                out.insert(text.clone());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_pubkeys_from_json(value, out);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_pubkeys_from_json(value, out);
            }
        }
        _ => {}
    }
}

fn extract_pools(
    config: &WatcherConfig,
    accounts: &HashMap<String, RpcAccount>,
    account_slots: &HashMap<String, u64>,
) -> HashMap<String, FreshPool> {
    let mut out = HashMap::new();

    for (address, account) in accounts {
        let Ok(data) = decode_account_data(account, address) else {
            continue;
        };
        let Some(mut pool) = parse_pool_candidate(config, address, &account.owner, &data) else {
            continue;
        };
        pool.last_seen_slot = account_slots.get(address).copied().unwrap_or_default();
        pool.hits = 1;
        out.entry(address.clone())
            .and_modify(|existing: &mut FreshPool| {
                existing.hits += 1;
                existing.last_seen_slot = existing.last_seen_slot.max(pool.last_seen_slot);
            })
            .or_insert(pool);
    }

    out
}

fn decode_account_data(account: &RpcAccount, address: &str) -> Result<Vec<u8>> {
    if account.data.1 != "base64" {
        anyhow::bail!("unexpected account data encoding for {}", address);
    }
    general_purpose::STANDARD
        .decode(&account.data.0)
        .with_context(|| format!("decode account data for {}", address))
}

fn parse_pool_candidate(
    config: &WatcherConfig,
    address: &str,
    owner: &str,
    data: &[u8],
) -> Option<FreshPool> {
    if owner == PUMPSWAP_PROGRAM_ID
        && data.len() >= 245
        && data.get(..8) == Some(PUMPSWAP_POOL_DISCRIMINATOR.as_slice())
    {
        let base_mint = pubkey_at(data, 43)?;
        let quote_mint = pubkey_at(data, 75)?;
        let (token_mint, quote_mint) =
            traded_token_and_quote(&base_mint, &quote_mint, &config.quote_mints)?;
        if config.excluded_token_mints.contains(&token_mint) {
            return None;
        }
        return Some(FreshPool {
            address: address.to_string(),
            venue: "pumpswap",
            token_mint,
            quote_mint,
            last_seen_slot: 0,
            hits: 0,
        });
    }

    if owner == METEORA_DLMM_PROGRAM_ID
        && data.len() >= 232
        && data.get(..8) == Some(METEORA_LB_PAIR_DISCRIMINATOR.as_slice())
    {
        let token_x_mint = pubkey_at(data, 88)?;
        let token_y_mint = pubkey_at(data, 120)?;
        let (token_mint, quote_mint) =
            traded_token_and_quote(&token_x_mint, &token_y_mint, &config.quote_mints)?;
        if config.excluded_token_mints.contains(&token_mint) {
            return None;
        }
        return Some(FreshPool {
            address: address.to_string(),
            venue: "meteora",
            token_mint,
            quote_mint,
            last_seen_slot: 0,
            hits: 0,
        });
    }

    None
}

fn pubkey_at(data: &[u8], offset: usize) -> Option<String> {
    data.get(offset..offset + 32)
        .map(|bytes| bs58::encode(bytes).into_string())
}

fn traded_token_and_quote(
    mint_a: &str,
    mint_b: &str,
    quote_mints: &HashSet<String>,
) -> Option<(String, String)> {
    let a_is_quote = quote_mints.contains(mint_a);
    let b_is_quote = quote_mints.contains(mint_b);
    match (a_is_quote, b_is_quote) {
        (true, false) => Some((mint_b.to_string(), mint_a.to_string())),
        (false, true) => Some((mint_a.to_string(), mint_b.to_string())),
        _ => None,
    }
}

fn select_routeable_pools(
    config: &WatcherConfig,
    pools: HashMap<String, FreshPool>,
) -> HashMap<String, FreshPool> {
    let mut by_token: HashMap<String, Vec<FreshPool>> = HashMap::new();
    for pool in pools.into_values() {
        by_token
            .entry(pool.token_mint.clone())
            .or_default()
            .push(pool);
    }

    let mut groups = by_token
        .into_iter()
        .filter_map(|(token, pools)| {
            let has_pumpswap = pools.iter().any(|pool| pool.venue == "pumpswap");
            let has_meteora = pools.iter().any(|pool| pool.venue == "meteora");
            (has_pumpswap && has_meteora).then(|| {
                let latest_slot = pools
                    .iter()
                    .map(|pool| pool.last_seen_slot)
                    .max()
                    .unwrap_or_default();
                let hits = pools.iter().map(|pool| pool.hits).sum::<u64>();
                (token, latest_slot, hits, pools)
            })
        })
        .collect::<Vec<_>>();

    groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));

    let mut selected = HashMap::new();
    for (_token, _latest_slot, _hits, mut pools) in groups.into_iter().take(config.max_tokens) {
        pools.sort_by(|a, b| {
            b.hits
                .cmp(&a.hits)
                .then_with(|| b.last_seen_slot.cmp(&a.last_seen_slot))
        });

        let mut pumpswap_count = 0usize;
        let mut meteora_count = 0usize;
        for pool in pools {
            match pool.venue {
                "pumpswap" if pumpswap_count < config.max_pumpswap_per_token => {
                    pumpswap_count += 1;
                }
                "meteora" if meteora_count < config.max_meteora_per_token => {
                    meteora_count += 1;
                }
                _ => continue,
            }
            selected.insert(pool.address.clone(), pool);
        }
    }

    selected
}

fn update_state(
    config: &WatcherConfig,
    fresh_pools: HashMap<String, FreshPool>,
) -> Result<WatcherState> {
    let now = unix_now();
    let mut state = read_state(&config.state_path)?;
    let mut active = state
        .pools
        .into_iter()
        .filter(|pool| !config.excluded_token_mints.contains(&pool.token_mint))
        .map(|pool| (pool.address.clone(), pool))
        .collect::<HashMap<_, _>>();

    for pool in fresh_pools.values() {
        active
            .entry(pool.address.clone())
            .and_modify(|existing| {
                existing.venue = pool.venue.to_string();
                existing.token_mint = pool.token_mint.clone();
                existing.quote_mint = pool.quote_mint.clone();
                existing.last_seen_unix = now;
                existing.last_seen_slot = existing.last_seen_slot.max(pool.last_seen_slot);
                existing.hits = existing.hits.saturating_add(pool.hits.max(1));
                existing.misses = 0;
            })
            .or_insert_with(|| StatePool {
                address: pool.address.clone(),
                venue: pool.venue.to_string(),
                token_mint: pool.token_mint.clone(),
                quote_mint: pool.quote_mint.clone(),
                first_seen_unix: now,
                last_seen_unix: now,
                last_seen_slot: pool.last_seen_slot,
                hits: pool.hits.max(1),
                misses: 0,
            });
    }

    let fresh_addresses = fresh_pools.keys().collect::<HashSet<_>>();
    for (address, pool) in active.iter_mut() {
        if !fresh_addresses.contains(address) {
            pool.misses = pool.misses.saturating_add(1);
        }
    }

    let mut pools = active
        .into_values()
        .filter(|pool| pool.misses < config.max_misses)
        .collect::<Vec<_>>();
    pools.sort_by(|a, b| {
        a.token_mint
            .cmp(&b.token_mint)
            .then_with(|| a.venue.cmp(&b.venue))
            .then_with(|| a.address.cmp(&b.address))
    });

    state = WatcherState {
        schema_version: default_schema_version(),
        source: "jd6_pool_watcher".to_string(),
        wallet: config.wallet.clone(),
        updated_unix: now,
        pools,
    };
    Ok(state)
}

fn read_state(path: &str) -> Result<WatcherState> {
    if !Path::new(path).exists() {
        return Ok(WatcherState::default());
    }
    let content = fs::read_to_string(path).with_context(|| format!("读取状态文件 {}", path))?;
    serde_json::from_str(&content).with_context(|| format!("解析状态文件 {}", path))
}

fn write_state(path: &str, state: &WatcherState) -> Result<()> {
    let content = serde_json::to_string_pretty(state)?;
    write_atomic(path, &content)
}

fn write_active_addresses(path: &str, state: &WatcherState) -> Result<()> {
    let mut content = String::new();
    for pool in &state.pools {
        let record = serde_json::json!({
            "address": pool.address,
            "dex_id": pool.venue,
            "token_mint": pool.token_mint,
            "quote_mint": pool.quote_mint,
            "first_seen_unix": pool.first_seen_unix,
            "last_seen_unix": pool.last_seen_unix,
            "last_seen_slot": pool.last_seen_slot,
            "hits": pool.hits,
            "misses": pool.misses,
            "recent_trades_5m": pool.hits,
            "recent_trades_15m": pool.hits,
            "verified": true,
        });
        content.push_str(&serde_json::to_string(&record)?);
        content.push('\n');
    }

    write_atomic(path, &content)
}

fn write_atomic(path: &str, content: &str) -> Result<()> {
    let tmp_path = format!("{}.tmp", path);
    fs::write(&tmp_path, content).with_context(|| format!("写入临时文件 {}", tmp_path))?;
    fs::rename(&tmp_path, path).with_context(|| format!("替换文件 {}", path))
}

fn routeable_token_count(state: &WatcherState) -> usize {
    let mut by_token: HashMap<&str, HashSet<&str>> = HashMap::new();
    for pool in &state.pools {
        by_token
            .entry(&pool.token_mint)
            .or_default()
            .insert(&pool.venue);
    }
    by_token
        .values()
        .filter(|venues| venues.contains("pumpswap") && venues.contains("meteora"))
        .count()
}

fn log_routeable_pool_summary(state: &WatcherState) {
    let mut by_token: HashMap<&str, (usize, usize)> = HashMap::new();
    for pool in &state.pools {
        let entry = by_token.entry(&pool.token_mint).or_default();
        match pool.venue.as_str() {
            "pumpswap" => entry.0 += 1,
            "meteora" => entry.1 += 1,
            _ => {}
        }
    }

    for (token, (pumpswap_count, meteora_count)) in by_token {
        if pumpswap_count > 0 && meteora_count > 0 {
            tracing::info!(
                "验证池子：币种={}，PumpSwap={}，Meteora={}",
                token,
                pumpswap_count,
                meteora_count
            );
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn default_schema_version() -> u32 {
    1
}

fn env_string(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_csv_set(key: &str, default: &[&str]) -> HashSet<String> {
    let raw = env::var(key).unwrap_or_else(|_| default.join(","));
    let mut out = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    if out.is_empty() {
        out.extend(default.iter().map(|value| (*value).to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_routeable_pumpswap_and_meteora_pools_only() {
        let mut config = WatcherConfig::from_env();
        config.max_tokens = 10;
        config.max_pumpswap_per_token = 2;
        config.max_meteora_per_token = 2;

        let routeable = "routeable_token".to_string();
        let orphan = "orphan_token".to_string();
        let pools = vec![
            FreshPool {
                address: "pump_pool".to_string(),
                venue: "pumpswap",
                token_mint: routeable.clone(),
                quote_mint: SOL_MINT.to_string(),
                last_seen_slot: 10,
                hits: 1,
            },
            FreshPool {
                address: "meteora_pool".to_string(),
                venue: "meteora",
                token_mint: routeable,
                quote_mint: SOL_MINT.to_string(),
                last_seen_slot: 10,
                hits: 1,
            },
            FreshPool {
                address: "orphan_pool".to_string(),
                venue: "meteora",
                token_mint: orphan,
                quote_mint: SOL_MINT.to_string(),
                last_seen_slot: 11,
                hits: 10,
            },
        ]
        .into_iter()
        .map(|pool| (pool.address.clone(), pool))
        .collect::<HashMap<_, _>>();

        let selected = select_routeable_pools(&config, pools);

        assert!(selected.contains_key("pump_pool"));
        assert!(selected.contains_key("meteora_pool"));
        assert!(!selected.contains_key("orphan_pool"));
    }

    #[test]
    fn identifies_quote_side() {
        let quotes = HashSet::from([SOL_MINT.to_string()]);
        let token = "Token111111111111111111111111111111111111111".to_string();

        assert_eq!(
            traded_token_and_quote(&token, SOL_MINT, &quotes),
            Some((token.clone(), SOL_MINT.to_string()))
        );
        assert_eq!(
            traded_token_and_quote(SOL_MINT, &token, &quotes),
            Some((token, SOL_MINT.to_string()))
        );
        let mut config = WatcherConfig::from_env();
        config.excluded_token_mints = HashSet::from([USDC_MINT.to_string()]);
        let (token_mint, _quote_mint) =
            traded_token_and_quote(USDC_MINT, SOL_MINT, &quotes).unwrap();
        assert!(config.excluded_token_mints.contains(&token_mint));
    }
}
