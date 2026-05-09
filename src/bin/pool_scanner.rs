use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use futures::{stream, StreamExt};
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

#[allow(dead_code)]
#[path = "../config.rs"]
mod config;

#[allow(dead_code)]
#[path = "../rpc.rs"]
mod rpc;

const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const PUMPSWAP_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const METEORA_LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
const PUMPSWAP_POOL_DATA_SIZE: usize = 245;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_AMOUNT_LEN: usize = 8;
const METEORA_BINS_PER_ARRAY: i32 = 70;
const METEORA_BIN_ARRAY_HEADER_LEN: usize = 56;
const METEORA_BIN_LEN: usize = 144;
const METEORA_BIN_ARRAY_CHECK_EACH_SIDE: i64 = 2;
const RPC_RETRY_ATTEMPTS: usize = 3;
const RPC_RETRY_BASE_DELAY_MS: u64 = 150;

const DEFAULT_OUTPUT_PATH: &str = "dynamic_market_addresses.txt";
const DEFAULT_STATE_PATH: &str = "dynamic_market_addresses.state.json";
const DEFAULT_POLL_SECS: u64 = 300;
const DEFAULT_API_BASE_URL: &str = "https://api.geckoterminal.com/api/v2";
const DEFAULT_API_NETWORK: &str = "solana";
const DEFAULT_API_PAGES: usize = 8;
const DEFAULT_API_PAGES_PER_CYCLE: usize = 1;
const DEFAULT_API_REQUEST_DELAY_MS: u64 = 5_000;
const DEFAULT_DEXSCREENER_BASE_URL: &str = "https://api.dexscreener.com";
const DEFAULT_DEXSCREENER_ENABLED: bool = true;
const DEFAULT_DEXSCREENER_TOKEN_LIMIT: usize = 40;
const DEFAULT_DEXSCREENER_REQUEST_DELAY_MS: u64 = 250;
const DEFAULT_API_MIN_M15_TRADES: usize = 5;
const DEFAULT_API_MIN_M15_VOLUME_USD: f64 = 50.0;
const DEFAULT_MAX_MISSES: u32 = 16;
const DEFAULT_MAX_TOKENS: usize = 50;
const DEFAULT_MAX_PUMPSWAP_PER_TOKEN: usize = 2;
const DEFAULT_MAX_METEORA_PER_TOKEN: usize = 5;
const DEFAULT_MIN_QUOTE_LIQUIDITY_USDC: f64 = 3_000.0;
const DEFAULT_RECENT_WINDOW_SECS: u64 = 300;
const DEFAULT_RECENT_WINDOW_SLOTS: u64 = 1_000;
const API_RETRY_ATTEMPTS: usize = 3;
const API_RETRY_BASE_DELAY_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerSource {
    Api,
    Onchain,
}

impl ScannerSource {
    fn from_env(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "onchain" => ScannerSource::Onchain,
            _ => ScannerSource::Api,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ScannerSource::Api => "api",
            ScannerSource::Onchain => "onchain",
        }
    }
}

#[derive(Debug, Clone)]
struct ScannerConfig {
    source: ScannerSource,
    rpc_url: String,
    api_base_url: String,
    api_network: String,
    api_pages: usize,
    api_pages_per_cycle: usize,
    api_request_delay_ms: u64,
    dexscreener_enabled: bool,
    dexscreener_base_url: String,
    dexscreener_token_limit: usize,
    dexscreener_request_delay_ms: u64,
    api_min_m15_trades: usize,
    api_min_m15_volume_usd: f64,
    output_path: String,
    state_path: String,
    poll_secs: u64,
    max_misses: u32,
    max_tokens: usize,
    max_pumpswap_per_token: usize,
    max_meteora_per_token: usize,
    min_quote_liquidity_usdc: f64,
    recent_window_secs: u64,
    recent_window_slots: u64,
    sol_usdc_price: f64,
    sol_mint: String,
    pumpswap_program_id: String,
    meteora_program_id: String,
    excluded_token_mints: HashSet<String>,
    excluded_market_addresses: HashSet<String>,
}

impl ScannerConfig {
    fn from_env() -> Result<Self> {
        let app_config = config::Config::from_file_or_default().context("读取 config.toml")?;
        let source = ScannerSource::from_env(&env_string_any(&["POOL_SCAN_SOURCE"], "api"));
        let output_path = env_string_any(
            &["POOL_SCAN_OUTPUT"],
            &non_empty_or_default(
                &app_config.discovery.dynamic_market_addresses_path,
                DEFAULT_OUTPUT_PATH,
            ),
        );
        let sol_usdc_price = env_f64_any(
            &["POOL_SCAN_SOL_USDC_PRICE"],
            positive_or_default(app_config.strategy.sol_usdc_price, 85.44),
        );
        let pumpswap_program_id = app_config
            .program_by_kind(config::ProgramKind::Pumpswap)
            .map(|program| program.program_id.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| PUMPSWAP_PROGRAM_ID.to_string());
        let meteora_program_id = app_config
            .program_by_kind(config::ProgramKind::MeteoraDlmm)
            .map(|program| program.program_id.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| METEORA_DLMM_PROGRAM_ID.to_string());

        let mut excluded_token_mints = app_config
            .discovery
            .excluded_target_token_mints
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        excluded_token_mints.extend(env_csv_values_any(&["POOL_SCAN_EXCLUDED_TOKEN_MINTS"]));

        Ok(Self {
            source,
            rpc_url: env_string_any(
                &["POOL_SCAN_RPC_URL", "RPC_HTTP_URL"],
                &app_config.rpc.http_url,
            ),
            api_base_url: env_string_any(&["POOL_SCAN_API_BASE_URL"], DEFAULT_API_BASE_URL),
            api_network: env_string_any(&["POOL_SCAN_API_NETWORK"], DEFAULT_API_NETWORK),
            api_pages: env_usize_any(&["POOL_SCAN_API_PAGES"], DEFAULT_API_PAGES),
            api_pages_per_cycle: env_usize_any(
                &["POOL_SCAN_API_PAGES_PER_CYCLE"],
                DEFAULT_API_PAGES_PER_CYCLE,
            ),
            api_request_delay_ms: env_u64_any(
                &["POOL_SCAN_API_REQUEST_DELAY_MS"],
                DEFAULT_API_REQUEST_DELAY_MS,
            ),
            dexscreener_enabled: env_bool_any(
                &["POOL_SCAN_DEXSCREENER_ENABLED"],
                DEFAULT_DEXSCREENER_ENABLED,
            ),
            dexscreener_base_url: env_string_any(
                &["POOL_SCAN_DEXSCREENER_BASE_URL"],
                DEFAULT_DEXSCREENER_BASE_URL,
            ),
            dexscreener_token_limit: env_usize_any(
                &["POOL_SCAN_DEXSCREENER_TOKEN_LIMIT"],
                DEFAULT_DEXSCREENER_TOKEN_LIMIT,
            ),
            dexscreener_request_delay_ms: env_u64_any(
                &["POOL_SCAN_DEXSCREENER_REQUEST_DELAY_MS"],
                DEFAULT_DEXSCREENER_REQUEST_DELAY_MS,
            ),
            api_min_m15_trades: env_usize_any(
                &["POOL_SCAN_MIN_M15_TRADES"],
                DEFAULT_API_MIN_M15_TRADES,
            ),
            api_min_m15_volume_usd: env_f64_any(
                &["POOL_SCAN_MIN_M15_VOLUME_USD"],
                DEFAULT_API_MIN_M15_VOLUME_USD,
            ),
            output_path,
            state_path: env_string_any(&["POOL_SCAN_STATE"], DEFAULT_STATE_PATH),
            poll_secs: env_u64_any(&["POOL_SCAN_POLL_SECS"], DEFAULT_POLL_SECS),
            max_misses: env_u32_any(&["POOL_SCAN_MAX_MISSES"], DEFAULT_MAX_MISSES),
            max_tokens: env_usize_any(&["POOL_SCAN_MAX_TOKENS"], DEFAULT_MAX_TOKENS),
            max_pumpswap_per_token: env_usize_any(
                &["POOL_SCAN_MAX_PUMPSWAP_PER_TOKEN"],
                DEFAULT_MAX_PUMPSWAP_PER_TOKEN,
            ),
            max_meteora_per_token: env_usize_any(
                &["POOL_SCAN_MAX_METEORA_PER_TOKEN"],
                DEFAULT_MAX_METEORA_PER_TOKEN,
            ),
            min_quote_liquidity_usdc: env_f64_any(
                &["POOL_SCAN_MIN_QUOTE_LIQUIDITY_USDC"],
                DEFAULT_MIN_QUOTE_LIQUIDITY_USDC,
            ),
            recent_window_secs: env_u64_any(
                &["POOL_SCAN_RECENT_WINDOW_SECS"],
                DEFAULT_RECENT_WINDOW_SECS,
            ),
            recent_window_slots: env_u64_any(
                &["POOL_SCAN_RECENT_WINDOW_SLOTS"],
                DEFAULT_RECENT_WINDOW_SLOTS,
            ),
            sol_usdc_price,
            sol_mint: app_config.tokens.sol_mint,
            pumpswap_program_id,
            meteora_program_id,
            excluded_token_mints,
            excluded_market_addresses: app_config
                .discovery
                .excluded_market_addresses
                .iter()
                .cloned()
                .collect(),
        })
    }
}

#[derive(Debug, Clone)]
struct ParsedPool {
    address: String,
    venue: &'static str,
    token_mint: String,
    quote_mint: String,
    quote_vault: String,
    quote_liquidity_usdc: f64,
    latest_slot: u64,
    hits: u64,
    meteora_active_id: Option<i32>,
}

#[derive(Debug, Clone)]
struct FreshPool {
    address: String,
    venue: &'static str,
    token_mint: String,
    quote_mint: String,
    quote_liquidity_usdc: f64,
    last_seen_slot: u64,
    hits: u64,
}

#[derive(Debug, Clone)]
struct ApiPoolCandidate {
    address: String,
    venue: &'static str,
    token_mint: String,
    quote_mint: String,
    reserve_usd: f64,
    m15_trades: usize,
    m15_volume_usd: f64,
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
    #[serde(default)]
    quote_liquidity_usdc: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ScannerState {
    #[serde(default)]
    source: String,
    updated_unix: u64,
    pools: Vec<StatePool>,
}

#[derive(Debug, Default)]
struct PollReport {
    program_accounts: usize,
    quote_pools: usize,
    liquidity_passed: usize,
    recent_passed: usize,
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
struct ProgramAccount {
    pubkey: String,
    account: RpcAccount,
}

#[derive(Debug, Deserialize)]
struct RpcAccount {
    data: (String, String),
}

#[derive(Debug, Deserialize)]
struct SignatureInfo {
    #[serde(default)]
    slot: Option<u64>,
    #[serde(default, rename = "blockTime")]
    block_time: Option<i64>,
    #[serde(default)]
    err: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct GeckoPoolsResponse {
    #[serde(default)]
    data: Vec<GeckoPool>,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoPool {
    #[serde(default)]
    attributes: GeckoPoolAttributes,
    #[serde(default)]
    relationships: GeckoRelationships,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoPoolAttributes {
    #[serde(default)]
    address: String,
    #[serde(default)]
    reserve_in_usd: Option<String>,
    #[serde(default)]
    volume_usd: Option<GeckoVolumeUsd>,
    #[serde(default)]
    transactions: Option<GeckoTransactions>,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoVolumeUsd {
    #[serde(default)]
    m15: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoTransactions {
    #[serde(default)]
    m15: Option<GeckoTransactionWindow>,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoTransactionWindow {
    #[serde(default)]
    buys: usize,
    #[serde(default)]
    sells: usize,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoRelationships {
    #[serde(default)]
    base_token: Option<GeckoRelationship>,
    #[serde(default)]
    quote_token: Option<GeckoRelationship>,
    #[serde(default)]
    dex: Option<GeckoRelationship>,
}

#[derive(Debug, Default, Deserialize)]
struct GeckoRelationship {
    #[serde(default)]
    data: Option<GeckoRelationshipData>,
}

#[derive(Debug, Deserialize)]
struct GeckoRelationshipData {
    id: String,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerToken {
    #[serde(default)]
    address: String,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerPair {
    #[serde(default, rename = "chainId")]
    chain_id: String,
    #[serde(default, rename = "dexId")]
    dex_id: String,
    #[serde(default, rename = "pairAddress")]
    pair_address: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default, rename = "baseToken")]
    base_token: DexScreenerToken,
    #[serde(default, rename = "quoteToken")]
    quote_token: DexScreenerToken,
    #[serde(default)]
    txns: Option<DexScreenerTxns>,
    #[serde(default)]
    volume: Option<DexScreenerVolume>,
    #[serde(default)]
    liquidity: Option<DexScreenerLiquidity>,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerTxns {
    #[serde(default)]
    m5: Option<DexScreenerTxnWindow>,
    #[serde(default)]
    h1: Option<DexScreenerTxnWindow>,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerTxnWindow {
    #[serde(default)]
    buys: usize,
    #[serde(default)]
    sells: usize,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerVolume {
    #[serde(default)]
    m5: Option<f64>,
    #[serde(default)]
    h1: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct DexScreenerLiquidity {
    #[serde(default)]
    usd: Option<f64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    let config = ScannerConfig::from_env()?;
    let run_once = env::args().any(|arg| arg == "--once");

    tracing::info!(
        "池子扫描器启动：来源={}，间隔={}秒，输出={}，最低流动性=${:.0}，近期窗口={}秒",
        config.source.label(),
        config.poll_secs,
        config.output_path,
        config.min_quote_liquidity_usdc,
        config.recent_window_secs
    );

    loop {
        match run_once_cycle(&config).await {
            Ok(report) => tracing::info!(
                "池子扫描完成：候选={}，SOL池={}，流动性通过={}，活跃通过={}，双边币种={}，本轮池子={}，保留池子={}",
                report.program_accounts,
                report.quote_pools,
                report.liquidity_passed,
                report.recent_passed,
                report.routeable_tokens,
                report.fresh_pools,
                report.active_pools
            ),
            Err(error) => tracing::warn!("池子扫描失败：{}", error),
        }

        if run_once {
            break;
        }
        sleep(Duration::from_secs(config.poll_secs.max(10))).await;
    }

    Ok(())
}

async fn run_once_cycle(config: &ScannerConfig) -> Result<PollReport> {
    if config.source == ScannerSource::Api {
        return run_api_once_cycle(config).await;
    }

    let current_slot = rpc::get_slot(&config.rpc_url).await?;
    let now_unix = unix_now();

    let (pumpswap_accounts, meteora_x_accounts, meteora_y_accounts) = tokio::try_join!(
        fetch_pumpswap_accounts(config),
        fetch_meteora_accounts(config, 88),
        fetch_meteora_accounts(config, 120),
    )?;

    let program_account_count =
        pumpswap_accounts.len() + meteora_x_accounts.len() + meteora_y_accounts.len();
    let parsed = parse_program_accounts(
        config,
        pumpswap_accounts,
        meteora_x_accounts,
        meteora_y_accounts,
    );
    let quote_pools = parsed.len();

    let quote_vaults = parsed
        .values()
        .map(|pool| pool.quote_vault.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let vault_data = rpc::get_multiple_accounts_data(&config.rpc_url, &quote_vaults)
        .await
        .context("读取池子金库余额")?;

    let liquidity_passed = apply_liquidity_filter(config, parsed, &vault_data);
    let liquidity_passed_count = liquidity_passed.len();
    let recent_passed = filter_recent_pools(config, liquidity_passed, current_slot, now_unix).await;
    let recent_passed_count = recent_passed.len();
    let bin_checked = filter_meteora_bin_arrays(config, recent_passed)
        .await
        .context("检查 Meteora bin array")?;
    let selected = select_routeable_pools(config, bin_checked);
    let fresh_pool_count = selected.len();

    let state = update_state(config, selected)?;
    write_active_addresses(&config.output_path, &state)?;
    write_state(&config.state_path, &state)?;
    log_routeable_pool_summary(&state);

    Ok(PollReport {
        program_accounts: program_account_count,
        quote_pools,
        liquidity_passed: liquidity_passed_count,
        recent_passed: recent_passed_count,
        routeable_tokens: routeable_token_count(&state),
        fresh_pools: fresh_pool_count,
        active_pools: state.pools.len(),
    })
}

async fn run_api_once_cycle(config: &ScannerConfig) -> Result<PollReport> {
    let now_unix = unix_now();
    let (total_pools, api_candidates) = fetch_api_candidates(config).await?;
    let quote_pool_count = api_candidates.len();

    let liquidity_passed = api_candidates
        .into_iter()
        .filter(|pool| pool.reserve_usd >= config.min_quote_liquidity_usdc)
        .collect::<Vec<_>>();
    let liquidity_passed_count = liquidity_passed.len();

    let active_passed = liquidity_passed
        .into_iter()
        .filter(|pool| {
            pool.m15_trades >= config.api_min_m15_trades
                && pool.m15_volume_usd >= config.api_min_m15_volume_usd
        })
        .collect::<Vec<_>>();
    let active_passed_count = active_passed.len();

    let selected = select_routeable_api_pools(config, active_passed, now_unix);
    let fresh_pool_count = selected.len();
    let state = update_state(config, selected)?;
    write_active_addresses(&config.output_path, &state)?;
    write_state(&config.state_path, &state)?;
    log_routeable_pool_summary(&state);

    Ok(PollReport {
        program_accounts: total_pools,
        quote_pools: quote_pool_count,
        liquidity_passed: liquidity_passed_count,
        recent_passed: active_passed_count,
        routeable_tokens: routeable_token_count(&state),
        fresh_pools: fresh_pool_count,
        active_pools: state.pools.len(),
    })
}

async fn fetch_api_candidates(config: &ScannerConfig) -> Result<(usize, Vec<ApiPoolCandidate>)> {
    let mut fetched_total = 0usize;
    let mut by_address: HashMap<String, ApiPoolCandidate> = HashMap::new();
    let mut request_count = 0usize;
    let pages = api_pages_for_cycle(config);

    tracing::info!(
        "本轮 API 页码：页码={:?}，总页数={}，每轮页数={}",
        pages,
        config.api_pages.max(1),
        config.api_pages_per_cycle.max(1)
    );

    for dex_id in ["pumpswap", "meteora"] {
        for page in pages.iter().copied() {
            if request_count > 0 && config.api_request_delay_ms > 0 {
                sleep(Duration::from_millis(config.api_request_delay_ms)).await;
            }
            let pools = match fetch_gecko_dex_pools_page(config, dex_id, page).await {
                Ok(pools) => pools,
                Err(error) => {
                    tracing::warn!("跳过 API 页：来源={}，页={}，原因={}", dex_id, page, error);
                    break;
                }
            };
            request_count += 1;
            fetched_total += pools.len();
            for pool in pools {
                let Some(candidate) = api_pool_candidate(config, pool) else {
                    continue;
                };
                by_address
                    .entry(candidate.address.clone())
                    .and_modify(|existing| {
                        existing.reserve_usd = existing.reserve_usd.max(candidate.reserve_usd);
                        existing.m15_trades = existing.m15_trades.max(candidate.m15_trades);
                        existing.m15_volume_usd =
                            existing.m15_volume_usd.max(candidate.m15_volume_usd);
                    })
                    .or_insert(candidate);
            }
        }
    }

    if config.dexscreener_enabled && !by_address.is_empty() {
        match fetch_dexscreener_candidates(config, &mut by_address).await {
            Ok(extra_pairs) => {
                fetched_total += extra_pairs;
            }
            Err(error) => {
                tracing::warn!("DexScreener 补充池失败：{}", error);
            }
        }
    }

    if fetched_total == 0 {
        anyhow::bail!("池子 API 没有返回可用数据");
    }

    Ok((fetched_total, by_address.into_values().collect()))
}

fn api_pages_for_cycle(config: &ScannerConfig) -> Vec<usize> {
    let total_pages = config.api_pages.max(1);
    let pages_per_cycle = config.api_pages_per_cycle.max(1).min(total_pages);
    if pages_per_cycle >= total_pages {
        return (1..=total_pages).collect();
    }

    let group_count = (total_pages + pages_per_cycle - 1) / pages_per_cycle;
    let poll_secs = config.poll_secs.max(1);
    let group_index = ((unix_now() / poll_secs) % group_count as u64) as usize;
    let start = group_index * pages_per_cycle + 1;
    let end = (start + pages_per_cycle - 1).min(total_pages);
    (start..=end).collect()
}

async fn fetch_dexscreener_candidates(
    config: &ScannerConfig,
    by_address: &mut HashMap<String, ApiPoolCandidate>,
) -> Result<usize> {
    let mut token_liquidity: HashMap<String, f64> = HashMap::new();
    for candidate in by_address.values() {
        token_liquidity
            .entry(candidate.token_mint.clone())
            .and_modify(|liquidity| *liquidity = liquidity.max(candidate.reserve_usd))
            .or_insert(candidate.reserve_usd);
    }

    let mut tokens = token_liquidity.into_iter().collect::<Vec<_>>();
    tokens.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    tokens.truncate(config.dexscreener_token_limit.max(1));

    let mut fetched_total = 0usize;
    let mut request_count = 0usize;
    for (token_mint, _) in tokens {
        if request_count > 0 && config.dexscreener_request_delay_ms > 0 {
            sleep(Duration::from_millis(config.dexscreener_request_delay_ms)).await;
        }
        request_count += 1;

        let pairs = match fetch_dexscreener_token_pairs(config, &token_mint).await {
            Ok(pairs) => pairs,
            Err(error) => {
                tracing::debug!(
                    "跳过 DexScreener token：币种={}，原因={}",
                    token_mint,
                    error
                );
                continue;
            }
        };
        fetched_total += pairs.len();
        for pair in pairs {
            let Some(candidate) = dexscreener_pool_candidate(config, pair) else {
                continue;
            };
            by_address
                .entry(candidate.address.clone())
                .and_modify(|existing| {
                    existing.reserve_usd = existing.reserve_usd.max(candidate.reserve_usd);
                    existing.m15_trades = existing.m15_trades.max(candidate.m15_trades);
                    existing.m15_volume_usd = existing.m15_volume_usd.max(candidate.m15_volume_usd);
                })
                .or_insert(candidate);
        }
    }

    Ok(fetched_total)
}

async fn fetch_dexscreener_token_pairs(
    config: &ScannerConfig,
    token_mint: &str,
) -> Result<Vec<DexScreenerPair>> {
    let base = config.dexscreener_base_url.trim_end_matches('/');
    let network = config.api_network.trim();
    let url = format!("{base}/token-pairs/v1/{network}/{token_mint}");
    let client = reqwest::Client::new();
    for attempt in 0..API_RETRY_ATTEMPTS {
        let response = client
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await
            .with_context(|| format!("请求 DexScreener：{}", token_mint))?;

        let status = response.status();
        if status.is_success() {
            return response
                .json::<Vec<DexScreenerPair>>()
                .await
                .with_context(|| format!("解析 DexScreener：{}", token_mint));
        }

        if (status.as_u16() == 429 || status.is_server_error()) && attempt + 1 < API_RETRY_ATTEMPTS
        {
            let delay_ms = API_RETRY_BASE_DELAY_MS * (attempt as u64 + 1);
            tracing::warn!(
                "DexScreener 暂时限流：币种={}，等待={}毫秒后重试",
                token_mint,
                delay_ms
            );
            sleep(Duration::from_millis(delay_ms)).await;
            continue;
        }

        anyhow::bail!("DexScreener 返回状态码：{} {}", token_mint, status);
    }

    unreachable!("DexScreener retry loop should always return")
}

async fn fetch_gecko_dex_pools_page(
    config: &ScannerConfig,
    dex_id: &str,
    page: usize,
) -> Result<Vec<GeckoPool>> {
    let base = config.api_base_url.trim_end_matches('/');
    let network = config.api_network.trim();
    let url = format!("{base}/networks/{network}/dexes/{dex_id}/pools?page={page}");
    let client = reqwest::Client::new();
    for attempt in 0..API_RETRY_ATTEMPTS {
        let response = client
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await
            .with_context(|| format!("请求池子 API：{} 第 {} 页", dex_id, page))?;

        let status = response.status();
        if status.is_success() {
            let response = response
                .json::<GeckoPoolsResponse>()
                .await
                .with_context(|| format!("解析池子 API：{} 第 {} 页", dex_id, page))?;
            return Ok(response.data);
        }

        if (status.as_u16() == 429 || status.is_server_error()) && attempt + 1 < API_RETRY_ATTEMPTS
        {
            let delay_ms = API_RETRY_BASE_DELAY_MS * (attempt as u64 + 1);
            tracing::warn!(
                "池子 API 暂时限流：来源={}，页={}，等待={}毫秒后重试",
                dex_id,
                page,
                delay_ms
            );
            sleep(Duration::from_millis(delay_ms)).await;
            continue;
        }

        anyhow::bail!("池子 API 返回状态码：{} {}", dex_id, status);
    }

    unreachable!("API retry loop should always return")
}

fn api_pool_candidate(config: &ScannerConfig, pool: GeckoPool) -> Option<ApiPoolCandidate> {
    let dex_id = relationship_id(&pool.relationships.dex)?;
    let venue = match dex_id.as_str() {
        "pumpswap" => "pumpswap",
        "meteora" => "meteora",
        _ => return None,
    };

    let base_mint = gecko_token_mint(&relationship_id(&pool.relationships.base_token)?);
    let raw_quote_mint = gecko_token_mint(&relationship_id(&pool.relationships.quote_token)?);
    let (token_mint, quote_mint) =
        if base_mint == config.sol_mint && raw_quote_mint != config.sol_mint {
            (raw_quote_mint, config.sol_mint.clone())
        } else if raw_quote_mint == config.sol_mint && base_mint != config.sol_mint {
            (base_mint, config.sol_mint.clone())
        } else {
            return None;
        };

    if pool.attributes.address.trim().is_empty()
        || config.excluded_token_mints.contains(&token_mint)
        || config
            .excluded_market_addresses
            .contains(&pool.attributes.address)
    {
        return None;
    }

    let reserve_usd = parse_optional_f64(pool.attributes.reserve_in_usd.as_deref())?;
    let m15_trades = pool
        .attributes
        .transactions
        .as_ref()
        .and_then(|value| value.m15.as_ref())
        .map(|value| value.buys + value.sells)
        .unwrap_or_default();
    let m15_volume_usd = pool
        .attributes
        .volume_usd
        .as_ref()
        .and_then(|value| parse_optional_f64(value.m15.as_deref()))
        .unwrap_or_default();

    Some(ApiPoolCandidate {
        address: pool.attributes.address,
        venue,
        token_mint,
        quote_mint,
        reserve_usd,
        m15_trades,
        m15_volume_usd,
    })
}

fn dexscreener_pool_candidate(
    config: &ScannerConfig,
    pair: DexScreenerPair,
) -> Option<ApiPoolCandidate> {
    if pair.chain_id != config.api_network {
        return None;
    }

    let venue = match pair.dex_id.to_ascii_lowercase().as_str() {
        "pumpswap" => "pumpswap",
        "meteora" => {
            let is_dlmm = pair
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case("dlmm"));
            if !is_dlmm {
                return None;
            }
            "meteora"
        }
        _ => return None,
    };

    let base_mint = pair.base_token.address;
    let raw_quote_mint = pair.quote_token.address;
    let (token_mint, quote_mint) =
        if base_mint == config.sol_mint && raw_quote_mint != config.sol_mint {
            (raw_quote_mint, config.sol_mint.clone())
        } else if raw_quote_mint == config.sol_mint && base_mint != config.sol_mint {
            (base_mint, config.sol_mint.clone())
        } else {
            return None;
        };

    if pair.pair_address.trim().is_empty()
        || config.excluded_token_mints.contains(&token_mint)
        || config
            .excluded_market_addresses
            .contains(&pair.pair_address)
    {
        return None;
    }

    let reserve_usd = pair.liquidity.and_then(|liquidity| liquidity.usd)?;
    if !reserve_usd.is_finite() {
        return None;
    }

    let m5_trades = pair
        .txns
        .as_ref()
        .and_then(|txns| txns.m5.as_ref())
        .map(|window| window.buys + window.sells)
        .unwrap_or_default();
    let h1_trades = pair
        .txns
        .as_ref()
        .and_then(|txns| txns.h1.as_ref())
        .map(|window| window.buys + window.sells)
        .unwrap_or_default();
    let m15_trades = if m5_trades > 0 {
        m5_trades
    } else {
        h1_trades / 12
    };

    let m5_volume_usd = pair
        .volume
        .as_ref()
        .and_then(|volume| volume.m5)
        .unwrap_or_default();
    let h1_volume_usd = pair
        .volume
        .as_ref()
        .and_then(|volume| volume.h1)
        .unwrap_or_default();
    let m15_volume_usd = if m5_volume_usd > 0.0 {
        m5_volume_usd
    } else {
        h1_volume_usd / 12.0
    };

    Some(ApiPoolCandidate {
        address: pair.pair_address,
        venue,
        token_mint,
        quote_mint,
        reserve_usd,
        m15_trades,
        m15_volume_usd,
    })
}

fn select_routeable_api_pools(
    config: &ScannerConfig,
    pools: Vec<ApiPoolCandidate>,
    now_unix: u64,
) -> HashMap<String, FreshPool> {
    let mut by_token: HashMap<String, Vec<ApiPoolCandidate>> = HashMap::new();
    for pool in pools {
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
                let liquidity = pools
                    .iter()
                    .map(|pool| pool.reserve_usd)
                    .fold(0.0, f64::max);
                let activity = pools.iter().map(|pool| pool.m15_trades).sum::<usize>();
                (token, liquidity, activity, pools)
            })
        })
        .collect::<Vec<_>>();

    groups.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.cmp(&a.2))
    });

    let mut selected = HashMap::new();
    for (_token, _liquidity, _activity, mut pools) in groups.into_iter().take(config.max_tokens) {
        pools.sort_by(|a, b| {
            b.reserve_usd
                .partial_cmp(&a.reserve_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.m15_trades.cmp(&a.m15_trades))
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
            selected.insert(
                pool.address.clone(),
                FreshPool {
                    address: pool.address,
                    venue: pool.venue,
                    token_mint: pool.token_mint,
                    quote_mint: pool.quote_mint,
                    quote_liquidity_usdc: pool.reserve_usd,
                    last_seen_slot: now_unix,
                    hits: 1,
                },
            );
        }
    }

    selected
}

fn relationship_id(relationship: &Option<GeckoRelationship>) -> Option<String> {
    relationship
        .as_ref()
        .and_then(|relationship| relationship.data.as_ref())
        .map(|data| data.id.clone())
}

fn gecko_token_mint(token_id: &str) -> String {
    token_id
        .strip_prefix("solana_")
        .unwrap_or(token_id)
        .to_string()
}

fn parse_optional_f64(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

async fn fetch_pumpswap_accounts(config: &ScannerConfig) -> Result<Vec<ProgramAccount>> {
    let filters = vec![
        serde_json::json!({ "dataSize": PUMPSWAP_POOL_DATA_SIZE }),
        memcmp_filter(0, &PUMPSWAP_POOL_DISCRIMINATOR),
        memcmp_filter_string(75, &config.sol_mint),
    ];
    fetch_program_accounts(&config.rpc_url, &config.pumpswap_program_id, filters).await
}

async fn fetch_meteora_accounts(
    config: &ScannerConfig,
    quote_mint_offset: usize,
) -> Result<Vec<ProgramAccount>> {
    let filters = vec![
        memcmp_filter(0, &METEORA_LB_PAIR_DISCRIMINATOR),
        memcmp_filter_string(quote_mint_offset, &config.sol_mint),
    ];
    fetch_program_accounts(&config.rpc_url, &config.meteora_program_id, filters).await
}

async fn fetch_program_accounts(
    rpc_url: &str,
    program_id: &str,
    filters: Vec<Value>,
) -> Result<Vec<ProgramAccount>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            program_id,
            {
                "encoding": "base64",
                "commitment": "processed",
                "filters": filters
            }
        ]
    });

    let response: RpcValueResponse<Vec<ProgramAccount>> =
        post_rpc_json_with_retries(rpc_url, &request, "getProgramAccounts").await?;
    if let Some(error) = response.error {
        anyhow::bail!("getProgramAccounts RPC error: {}", error);
    }
    Ok(response.result.unwrap_or_default())
}

fn parse_program_accounts(
    config: &ScannerConfig,
    pumpswap_accounts: Vec<ProgramAccount>,
    meteora_x_accounts: Vec<ProgramAccount>,
    meteora_y_accounts: Vec<ProgramAccount>,
) -> HashMap<String, ParsedPool> {
    let mut out = HashMap::new();

    for account in pumpswap_accounts {
        if config.excluded_market_addresses.contains(&account.pubkey) {
            continue;
        }
        let Ok(data) = decode_account_data(&account.account, &account.pubkey) else {
            continue;
        };
        if let Some(pool) = parse_pumpswap_pool(config, &account.pubkey, &data) {
            out.insert(pool.address.clone(), pool);
        }
    }

    for account in meteora_x_accounts.into_iter().chain(meteora_y_accounts) {
        if config.excluded_market_addresses.contains(&account.pubkey) {
            continue;
        }
        if out.contains_key(&account.pubkey) {
            continue;
        }
        let Ok(data) = decode_account_data(&account.account, &account.pubkey) else {
            continue;
        };
        if let Some(pool) = parse_meteora_pool(config, &account.pubkey, &data) {
            out.insert(pool.address.clone(), pool);
        }
    }

    out
}

fn parse_pumpswap_pool(config: &ScannerConfig, address: &str, data: &[u8]) -> Option<ParsedPool> {
    if data.len() < PUMPSWAP_POOL_DATA_SIZE {
        return None;
    }
    if data.get(..8) != Some(PUMPSWAP_POOL_DISCRIMINATOR.as_slice()) {
        return None;
    }

    let base_mint = pubkey_at(data, 43)?;
    let quote_mint = pubkey_at(data, 75)?;
    if quote_mint != config.sol_mint || base_mint == config.sol_mint {
        return None;
    }
    if config.excluded_token_mints.contains(&base_mint) {
        return None;
    }

    Some(ParsedPool {
        address: address.to_string(),
        venue: "pumpswap",
        token_mint: base_mint,
        quote_mint,
        quote_vault: pubkey_at(data, 171)?,
        quote_liquidity_usdc: 0.0,
        latest_slot: 0,
        hits: 1,
        meteora_active_id: None,
    })
}

fn parse_meteora_pool(config: &ScannerConfig, address: &str, data: &[u8]) -> Option<ParsedPool> {
    if data.len() < 232 {
        return None;
    }
    if data.get(..8) != Some(METEORA_LB_PAIR_DISCRIMINATOR.as_slice()) {
        return None;
    }

    let active_id = read_i32(data, 76)?;
    let token_x_mint = pubkey_at(data, 88)?;
    let token_y_mint = pubkey_at(data, 120)?;
    let reserve_x = pubkey_at(data, 152)?;
    let reserve_y = pubkey_at(data, 184)?;

    let (token_mint, quote_vault) =
        if token_x_mint == config.sol_mint && token_y_mint != config.sol_mint {
            (token_y_mint, reserve_x)
        } else if token_y_mint == config.sol_mint && token_x_mint != config.sol_mint {
            (token_x_mint, reserve_y)
        } else {
            return None;
        };

    if config.excluded_token_mints.contains(&token_mint) {
        return None;
    }

    Some(ParsedPool {
        address: address.to_string(),
        venue: "meteora",
        token_mint,
        quote_mint: config.sol_mint.clone(),
        quote_vault,
        quote_liquidity_usdc: 0.0,
        latest_slot: 0,
        hits: 1,
        meteora_active_id: Some(active_id),
    })
}

fn apply_liquidity_filter(
    config: &ScannerConfig,
    pools: HashMap<String, ParsedPool>,
    vault_data: &HashMap<String, Vec<u8>>,
) -> HashMap<String, ParsedPool> {
    pools
        .into_values()
        .filter_map(|mut pool| {
            let amount = vault_data
                .get(&pool.quote_vault)
                .and_then(|data| parse_token_account_amount(data))?;
            let quote_sol = amount as f64 / 1_000_000_000.0;
            let liquidity_usdc = quote_sol * config.sol_usdc_price;
            if liquidity_usdc < config.min_quote_liquidity_usdc {
                return None;
            }
            pool.quote_liquidity_usdc = liquidity_usdc;
            Some((pool.address.clone(), pool))
        })
        .collect()
}

async fn filter_recent_pools(
    config: &ScannerConfig,
    pools: HashMap<String, ParsedPool>,
    current_slot: u64,
    now_unix: u64,
) -> HashMap<String, ParsedPool> {
    stream::iter(pools.into_values())
        .map(|mut pool| {
            let rpc_url = config.rpc_url.clone();
            let recent_window_secs = config.recent_window_secs;
            let recent_window_slots = config.recent_window_slots;
            async move {
                let latest = match fetch_latest_signature(&rpc_url, &pool.address).await {
                    Ok(Some(latest)) => latest,
                    _ => return None,
                };
                if latest.err.is_some() {
                    return None;
                }
                let slot = latest.slot.unwrap_or_default();
                let recent_by_time = latest
                    .block_time
                    .and_then(|value| u64::try_from(value).ok())
                    .map(|block_time| now_unix.saturating_sub(block_time) <= recent_window_secs)
                    .unwrap_or(false);
                let recent_by_slot = current_slot.saturating_sub(slot) <= recent_window_slots;
                if !recent_by_time && !recent_by_slot {
                    return None;
                }

                pool.latest_slot = slot;
                Some((pool.address.clone(), pool))
            }
        })
        .buffer_unordered(32)
        .filter_map(|value| async move { value })
        .collect()
        .await
}

async fn fetch_latest_signature(rpc_url: &str, address: &str) -> Result<Option<SignatureInfo>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSignaturesForAddress",
        "params": [
            address,
            {
                "limit": 1,
                "commitment": "confirmed"
            }
        ]
    });

    let response: RpcValueResponse<Vec<SignatureInfo>> =
        post_rpc_json_with_retries(rpc_url, &request, "getSignaturesForAddress").await?;
    if let Some(error) = response.error {
        anyhow::bail!("getSignaturesForAddress RPC error: {}", error);
    }
    Ok(response.result.unwrap_or_default().into_iter().next())
}

async fn filter_meteora_bin_arrays(
    config: &ScannerConfig,
    pools: HashMap<String, ParsedPool>,
) -> Result<HashMap<String, ParsedPool>> {
    let mut bin_to_pool = HashMap::new();
    for pool in pools.values() {
        if pool.venue != "meteora" {
            continue;
        }
        let Some(active_id) = pool.meteora_active_id else {
            continue;
        };
        for address in derive_nearby_bin_array_addresses(
            &config.meteora_program_id,
            &pool.address,
            active_id,
            METEORA_BIN_ARRAY_CHECK_EACH_SIDE,
        )? {
            bin_to_pool.insert(address, pool.address.clone());
        }
    }

    let bin_addresses = bin_to_pool.keys().cloned().collect::<Vec<_>>();
    let bin_data = if bin_addresses.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &bin_addresses).await?
    };

    let mut valid_meteora_pools = HashSet::new();
    for (bin_address, pool_address) in bin_to_pool {
        let Some(data) = bin_data.get(&bin_address) else {
            continue;
        };
        if bin_array_matches_pool_with_liquidity(data, &pool_address) {
            valid_meteora_pools.insert(pool_address);
        }
    }

    Ok(pools
        .into_iter()
        .filter(|(_, pool)| pool.venue != "meteora" || valid_meteora_pools.contains(&pool.address))
        .collect())
}

fn select_routeable_pools(
    config: &ScannerConfig,
    pools: HashMap<String, ParsedPool>,
) -> HashMap<String, FreshPool> {
    let mut by_token: HashMap<String, Vec<ParsedPool>> = HashMap::new();
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
                    .map(|pool| pool.latest_slot)
                    .max()
                    .unwrap_or_default();
                let liquidity = pools
                    .iter()
                    .map(|pool| pool.quote_liquidity_usdc)
                    .fold(0.0, f64::max);
                (token, latest_slot, liquidity, pools)
            })
        })
        .collect::<Vec<_>>();

    groups.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });

    let mut selected = HashMap::new();
    for (_token, _latest_slot, _liquidity, mut pools) in groups.into_iter().take(config.max_tokens)
    {
        pools.sort_by(|a, b| {
            b.quote_liquidity_usdc
                .partial_cmp(&a.quote_liquidity_usdc)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.latest_slot.cmp(&a.latest_slot))
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
            selected.insert(
                pool.address.clone(),
                FreshPool {
                    address: pool.address,
                    venue: pool.venue,
                    token_mint: pool.token_mint,
                    quote_mint: pool.quote_mint,
                    quote_liquidity_usdc: pool.quote_liquidity_usdc,
                    last_seen_slot: pool.latest_slot,
                    hits: pool.hits,
                },
            );
        }
    }

    selected
}

fn update_state(
    config: &ScannerConfig,
    fresh_pools: HashMap<String, FreshPool>,
) -> Result<ScannerState> {
    let now = unix_now();
    let source = scanner_state_source(config);
    let state = read_state(&config.state_path)?;
    let state = if state.source == source {
        state
    } else {
        ScannerState::default()
    };
    let mut active = state
        .pools
        .into_iter()
        .filter(|pool| !config.excluded_token_mints.contains(&pool.token_mint))
        .filter(|pool| !config.excluded_market_addresses.contains(&pool.address))
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
                existing.quote_liquidity_usdc = pool.quote_liquidity_usdc;
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
                quote_liquidity_usdc: pool.quote_liquidity_usdc,
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

    Ok(ScannerState {
        source,
        updated_unix: now,
        pools,
    })
}

fn scanner_state_source(config: &ScannerConfig) -> String {
    match config.source {
        ScannerSource::Api if config.dexscreener_enabled => {
            "api_geckoterminal_dexscreener_pumpswap_meteora"
        }
        ScannerSource::Api => "api_geckoterminal_pumpswap_meteora",
        ScannerSource::Onchain => "onchain_pumpswap_meteora",
    }
    .to_string()
}

fn read_state(path: &str) -> Result<ScannerState> {
    if !Path::new(path).exists() {
        return Ok(ScannerState::default());
    }
    let content = fs::read_to_string(path).with_context(|| format!("读取状态文件 {}", path))?;
    serde_json::from_str(&content).with_context(|| format!("解析状态文件 {}", path))
}

fn write_state(path: &str, state: &ScannerState) -> Result<()> {
    let content = serde_json::to_string_pretty(state)?;
    write_atomic(path, &content)
}

fn write_active_addresses(path: &str, state: &ScannerState) -> Result<()> {
    let mut content = String::new();
    content.push_str("# 自动生成：PumpSwap/Meteora 动态池子列表\n");
    content.push_str("# 主程序启动时会把这里的地址合并进静态市场\n");
    content.push_str("# 只保留 SOL 报价、流动性达标且双边可路由的池子\n");
    content.push_str(&format!(
        "# source={} updated_unix={} active_pools={}\n",
        state.source,
        state.updated_unix,
        state.pools.len()
    ));

    let mut current_token = "";
    for pool in &state.pools {
        if current_token != pool.token_mint {
            current_token = &pool.token_mint;
            content.push_str(&format!("\n# token={}\n", current_token));
        }
        content.push_str(&format!(
            "{} # {} quote={} liq=${:.0} hits={} misses={} seen={}\n",
            pool.address,
            pool.venue,
            pool.quote_mint,
            pool.quote_liquidity_usdc,
            pool.hits,
            pool.misses,
            pool.last_seen_slot
        ));
    }

    write_atomic(path, &content)
}

fn write_atomic(path: &str, content: &str) -> Result<()> {
    let tmp_path = format!("{}.tmp", path);
    fs::write(&tmp_path, content).with_context(|| format!("写入临时文件 {}", tmp_path))?;
    fs::rename(&tmp_path, path).with_context(|| format!("替换文件 {}", path))
}

fn routeable_token_count(state: &ScannerState) -> usize {
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

fn log_routeable_pool_summary(state: &ScannerState) {
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
                "保留双边池：币种={}，PumpSwap={}，Meteora={}",
                token,
                pumpswap_count,
                meteora_count
            );
        }
    }
}

fn decode_account_data(account: &RpcAccount, address: &str) -> Result<Vec<u8>> {
    if account.data.1 != "base64" {
        anyhow::bail!("unexpected account data encoding for {}", address);
    }
    general_purpose::STANDARD
        .decode(&account.data.0)
        .with_context(|| format!("decode account data for {}", address))
}

async fn post_rpc_json_with_retries<T: DeserializeOwned>(
    rpc_url: &str,
    request: &Value,
    context: &str,
) -> Result<T> {
    let client = reqwest::Client::new();
    for attempt in 0..RPC_RETRY_ATTEMPTS {
        let response = client.post(rpc_url).json(request).send().await;
        match response {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    if attempt + 1 < RPC_RETRY_ATTEMPTS {
                        sleep(Duration::from_millis(
                            RPC_RETRY_BASE_DELAY_MS * (attempt as u64 + 1),
                        ))
                        .await;
                        continue;
                    }
                    anyhow::bail!("{} HTTP status: {}", context, status);
                }
                return response
                    .json::<T>()
                    .await
                    .with_context(|| format!("解析 {} 响应", context));
            }
            Err(error) => {
                if attempt + 1 < RPC_RETRY_ATTEMPTS {
                    sleep(Duration::from_millis(
                        RPC_RETRY_BASE_DELAY_MS * (attempt as u64 + 1),
                    ))
                    .await;
                    continue;
                }
                return Err(error).with_context(|| format!("请求 {}", context));
            }
        }
    }
    unreachable!("RPC retry loop should always return")
}

fn memcmp_filter(offset: usize, bytes: &[u8]) -> Value {
    serde_json::json!({
        "memcmp": {
            "offset": offset,
            "bytes": bs58::encode(bytes).into_string()
        }
    })
}

fn memcmp_filter_string(offset: usize, value: &str) -> Value {
    serde_json::json!({
        "memcmp": {
            "offset": offset,
            "bytes": value
        }
    })
}

fn pubkey_at(data: &[u8], offset: usize) -> Option<String> {
    data.get(offset..offset + 32)
        .map(|bytes| bs58::encode(bytes).into_string())
}

fn parse_token_account_amount(data: &[u8]) -> Option<u64> {
    let bytes = data
        .get(TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_OFFSET + TOKEN_ACCOUNT_AMOUNT_LEN)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(i32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_i64(data: &[u8], offset: usize) -> Option<i64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u128(data: &[u8], offset: usize) -> Option<u128> {
    let bytes = data.get(offset..offset + 16)?;
    Some(u128::from_le_bytes(bytes.try_into().ok()?))
}

fn meteora_bin_array_index(active_id: i32) -> i64 {
    let mut index = active_id / METEORA_BINS_PER_ARRAY;
    if active_id < 0 && active_id % METEORA_BINS_PER_ARRAY != 0 {
        index -= 1;
    }
    i64::from(index)
}

fn derive_nearby_bin_array_addresses(
    meteora_program_id: &str,
    lb_pair: &str,
    active_id: i32,
    count_each_side: i64,
) -> Result<Vec<String>> {
    let lb_pair = Pubkey::from_str(lb_pair)?;
    let program_id = Pubkey::from_str(meteora_program_id)?;
    let current = meteora_bin_array_index(active_id);
    let mut out = Vec::new();
    for offset in -count_each_side..=count_each_side {
        let index = current + offset;
        let (address, _) = Pubkey::find_program_address(
            &[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()],
            &program_id,
        );
        out.push(address.to_string());
    }
    Ok(out)
}

fn bin_array_matches_pool_with_liquidity(data: &[u8], pool_address: &str) -> bool {
    if data.len() < METEORA_BIN_ARRAY_HEADER_LEN {
        return false;
    }
    let Some(lb_pair) = pubkey_at(data, 24) else {
        return false;
    };
    if lb_pair != pool_address {
        return false;
    }

    let _index = read_i64(data, 8);
    for idx in 0..METEORA_BINS_PER_ARRAY as usize {
        let offset = METEORA_BIN_ARRAY_HEADER_LEN + idx * METEORA_BIN_LEN;
        let Some(amount_x) = read_u64(data, offset) else {
            break;
        };
        let amount_y = read_u64(data, offset + 8).unwrap_or_default();
        let liquidity_supply = read_u128(data, offset + 32).unwrap_or_default();
        if amount_x > 0 || amount_y > 0 || liquidity_supply > 0 {
            return true;
        }
    }

    false
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn non_empty_or_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

fn positive_or_default(value: f64, default: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        default
    }
}

fn env_string_any(keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|key| env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_u64_any(keys: &[&str], default: u64) -> u64 {
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|value| value.parse().ok()))
        .unwrap_or(default)
}

fn env_bool_any(keys: &[&str], default: bool) -> bool {
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|value| value.parse().ok()))
        .unwrap_or(default)
}

fn env_u32_any(keys: &[&str], default: u32) -> u32 {
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|value| value.parse().ok()))
        .unwrap_or(default)
}

fn env_usize_any(keys: &[&str], default: usize) -> usize {
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|value| value.parse().ok()))
        .unwrap_or(default)
}

fn env_f64_any(keys: &[&str], default: f64) -> f64 {
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|value| value.parse().ok()))
        .map(|value: f64| positive_or_default(value, default))
        .unwrap_or(default)
}

fn env_csv_values_any(keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| env::var(key).ok())
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}
