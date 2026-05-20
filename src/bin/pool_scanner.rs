#![allow(dead_code)]

use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use futures::{stream, StreamExt};
use model::state::{RaydiumState, RaydiumVenue, WhirlpoolState, WhirlpoolTickArrayState};
use orca_whirlpools_client::{Whirlpool as OrcaWhirlpool, WHIRLPOOL_DISCRIMINATOR};
use parser::raydium_clmm::ClmmTickArrayState;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    path::Path,
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, timeout};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::*;

#[allow(dead_code)]
#[path = "../config.rs"]
mod config;

#[allow(dead_code)]
#[path = "../rpc.rs"]
mod rpc;

#[allow(dead_code)]
#[path = "../model/mod.rs"]
mod model;

#[allow(dead_code)]
#[path = "../strategy/quote.rs"]
mod strategy_quote;

#[allow(dead_code)]
#[path = "../strategy/clmm.rs"]
mod strategy_clmm;

#[allow(dead_code)]
mod strategy {
    pub mod quote {
        pub use crate::strategy_quote::*;
    }

    pub mod clmm {
        pub use crate::strategy_clmm::*;
    }
}

#[allow(dead_code)]
#[path = "../parser/mod.rs"]
mod parser;

const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const PUMPSWAP_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const METEORA_LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
const RAYDIUM_CLMM_POOL_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
// PumpSwap pool layouts can be longer on current mainnet accounts (observed 301 bytes).
// The fields we need are in the first 245 bytes, so treat this as the minimum parse size
// and avoid exact dataSize filters when discovering pools.
const PUMPSWAP_POOL_MIN_DATA_SIZE: usize = 245;
const RAYDIUM_CLMM_POOL_MIN_DATA_SIZE: usize = 1_400;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_AMOUNT_LEN: usize = 8;
const TOKEN_MINT_DECIMALS_OFFSET: usize = 44;
const METEORA_BINS_PER_ARRAY: i32 = 70;
const METEORA_BIN_ARRAY_HEADER_LEN: usize = 56;
const METEORA_BIN_LEN: usize = 144;
const METEORA_BIN_ARRAY_CHECK_EACH_SIDE: i64 = 2;
const METEORA_LOCAL_QUOTE_SAFETY_BPS: f64 = 10.0;
const RAYDIUM_CLMM_TICK_ARRAYS_EACH_SIDE: i32 = 8;
const MAX_SCANNER_RAYDIUM_TICK_ARRAYS_PER_POOL: usize = 48;
const WHIRLPOOL_TICK_ARRAYS_EACH_SIDE: i32 = 2;
const RPC_RETRY_ATTEMPTS: usize = 3;
const RPC_RETRY_BASE_DELAY_MS: u64 = 150;
const PUMPSWAP_DISCOVERY_FEE_BPS: f64 = 125.0;
const BASE_SIGNATURE_FEE_LAMPORTS: u64 = 5_000;
const DEFAULT_JITO_MIN_TIP_LAMPORTS: u64 = 1_000;

const DEFAULT_OUTPUT_PATH: &str = "validated_pools.jsonl";
const DEFAULT_STATE_PATH: &str = "validated_pools.snapshot";
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
const DEFAULT_GRPC_FLUSH_INTERVAL_SECS: u64 = 5;
const DEFAULT_GRPC_RUN_ONCE_SECS: u64 = 60;
const GRPC_DIRTY_FLUSH_UPDATES: usize = 2_048;
const GRPC_TX_LOOKUP_FLUSH_MS: u64 = 750;
const GRPC_TX_LOOKUP_BATCH: usize = 512;
const RECENT_5M_SECS: u64 = 300;
const RECENT_15M_SECS: u64 = 900;
const MIN_GRPC_POOL_RECENT_TRADES_5M: u64 = 1;
const DEFAULT_MAX_MISSES: u32 = 16;
const DEFAULT_MAX_TOKENS: usize = 50;
const DEFAULT_MAX_PUMPSWAP_PER_TOKEN: usize = 2;
const DEFAULT_MAX_METEORA_PER_TOKEN: usize = 5;
const DEFAULT_MIN_QUOTE_LIQUIDITY_USDC: f64 = 3_000.0;
const DEFAULT_MIN_TOKEN_RECENT_TRADES_15M: u64 = 3;
const DEFAULT_RECENT_WINDOW_SECS: u64 = 300;
const DEFAULT_RECENT_WINDOW_SLOTS: u64 = 1_000;
const API_RETRY_ATTEMPTS: usize = 3;
const API_RETRY_BASE_DELAY_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerSource {
    Api,
    Grpc,
    Onchain,
}

impl ScannerSource {
    fn from_env(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "api" => ScannerSource::Api,
            "onchain" | "rpc" => ScannerSource::Onchain,
            _ => ScannerSource::Grpc,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ScannerSource::Api => "api",
            ScannerSource::Grpc => "grpc",
            ScannerSource::Onchain => "onchain",
        }
    }
}

#[derive(Debug, Clone)]
struct ScannerConfig {
    source: ScannerSource,
    rpc_url: String,
    grpc_endpoint: String,
    grpc_token: Option<String>,
    grpc_flush_interval_secs: u64,
    grpc_run_once_secs: u64,
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
    min_pair_liquidity_usdc: f64,
    min_token_recent_trades_15m: u64,
    min_edge_pct: f64,
    effective_min_edge_pct: f64,
    cost_floor_edge_pct: f64,
    min_executable_profit_pct: f64,
    debug_quotes: bool,
    fixed_cost_lamports: u64,
    executable_trade_sizes_sol: Vec<f64>,
    buy_slippage_bps: f64,
    buy_slippage_bps_small_pool: f64,
    sell_slippage_bps: f64,
    sell_slippage_bps_small_pool: f64,
    small_pool_liquidity_usdc: f64,
    max_trade_depth_bps: f64,
    recent_window_secs: u64,
    recent_window_slots: u64,
    sol_usdc_price: f64,
    sol_mint: String,
    pumpswap_program_id: String,
    meteora_program_id: String,
    raydium_clmm_program_id: String,
    whirlpool_program_id: String,
    monitored_programs: Vec<MonitoredProgram>,
    excluded_token_mints: HashSet<String>,
    excluded_market_addresses: HashSet<String>,
}

#[derive(Debug, Clone)]
struct MonitoredProgram {
    label: &'static str,
    program_id: String,
}

impl ScannerConfig {
    fn from_env() -> Result<Self> {
        let app_config = config::Config::from_file_or_default().context("读取 config.toml")?;
        let source = ScannerSource::from_env(&env_string_any(&["POOL_SCAN_SOURCE"], "grpc"));
        let output_path = env_string_any(&["POOL_SCAN_OUTPUT"], DEFAULT_OUTPUT_PATH);
        let sol_usdc_price = env_f64_any(
            &["POOL_SCAN_SOL_USDC_PRICE"],
            positive_or_default(app_config.strategy.sol_usdc_price, 85.44),
        );
        let estimated_pumpswap_meteora_cost_pct = (PUMPSWAP_DISCOVERY_FEE_BPS
            + app_config.strategy.pumpswap_meteora_max_meteora_fee_bps
            + app_config.strategy.pumpswap_meteora_buy_slippage_bps
            + app_config.strategy.pumpswap_meteora_sell_slippage_bps)
            / 100.0;
        let default_min_edge_pct =
            positive_or_default(app_config.strategy.min_profit_threshold, 2.0)
                .max(estimated_pumpswap_meteora_cost_pct);
        let configured_min_edge_pct =
            env_f64_any(&["POOL_SCAN_MIN_EDGE_PCT"], default_min_edge_pct);
        let executable_trade_sizes_sol = scanner_executable_trade_sizes_sol(&app_config);
        let cost_floor_edge_pct = scanner_executable_edge_floor_pct(
            &app_config,
            &executable_trade_sizes_sol,
            sol_usdc_price,
            estimated_pumpswap_meteora_cost_pct,
        );
        let fixed_cost_lamports = scanner_fixed_cost_lamports(&app_config);
        let effective_min_edge_pct = if env_bool_any(&["POOL_SCAN_ALLOW_BELOW_COST_EDGE"], false) {
            configured_min_edge_pct
        } else {
            configured_min_edge_pct.max(cost_floor_edge_pct)
        };
        let pumpswap_program_id = resolve_program_id(
            &app_config,
            config::ProgramKind::Pumpswap,
            PUMPSWAP_PROGRAM_ID,
        );
        let meteora_program_id = resolve_program_id(
            &app_config,
            config::ProgramKind::MeteoraDlmm,
            METEORA_DLMM_PROGRAM_ID,
        );
        let raydium_clmm_program_id = resolve_program_id(
            &app_config,
            config::ProgramKind::RaydiumClmm,
            RAYDIUM_CLMM_PROGRAM_ID,
        );
        let whirlpool_program_id = resolve_program_id(
            &app_config,
            config::ProgramKind::Whirlpool,
            WHIRLPOOL_PROGRAM_ID,
        );
        let monitored_programs = vec![
            MonitoredProgram {
                label: config::ProgramKind::Pumpswap.default_label(),
                program_id: pumpswap_program_id.clone(),
            },
            MonitoredProgram {
                label: config::ProgramKind::MeteoraDlmm.default_label(),
                program_id: meteora_program_id.clone(),
            },
            MonitoredProgram {
                label: config::ProgramKind::RaydiumClmm.default_label(),
                program_id: raydium_clmm_program_id.clone(),
            },
            MonitoredProgram {
                label: config::ProgramKind::Whirlpool.default_label(),
                program_id: whirlpool_program_id.clone(),
            },
        ];

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
            grpc_endpoint: env_string_any(
                &["POOL_SCAN_GRPC_ENDPOINT", "GRPC_ENDPOINT"],
                &app_config.grpc.endpoint,
            ),
            grpc_token: env::var("POOL_SCAN_GRPC_TOKEN")
                .ok()
                .or_else(|| env::var("GRPC_TOKEN").ok())
                .or_else(|| app_config.grpc.token.clone())
                .filter(|value| !value.trim().is_empty()),
            grpc_flush_interval_secs: env_u64_any(
                &["POOL_SCAN_GRPC_FLUSH_INTERVAL_SECS"],
                DEFAULT_GRPC_FLUSH_INTERVAL_SECS,
            ),
            grpc_run_once_secs: env_u64_any(
                &["POOL_SCAN_GRPC_RUN_ONCE_SECS"],
                DEFAULT_GRPC_RUN_ONCE_SECS,
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
            state_path: env_string_any(
                &["POOL_SCAN_STATE"],
                &non_empty_or_default(
                    &app_config.discovery.validated_pools_snapshot_path,
                    DEFAULT_STATE_PATH,
                ),
            ),
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
            min_pair_liquidity_usdc: env_f64_any(
                &["POOL_SCAN_MIN_PAIR_LIQUIDITY_USDC"],
                DEFAULT_MIN_QUOTE_LIQUIDITY_USDC,
            ),
            min_token_recent_trades_15m: env_u64_any(
                &["POOL_SCAN_MIN_TOKEN_RECENT_TRADES_15M"],
                app_config
                    .discovery
                    .token_selection_min_recent_trades_15m
                    .max(DEFAULT_MIN_TOKEN_RECENT_TRADES_15M),
            ),
            min_edge_pct: configured_min_edge_pct,
            effective_min_edge_pct,
            cost_floor_edge_pct,
            min_executable_profit_pct: env_f64_any(
                &["POOL_SCAN_MIN_EXEC_NET_PCT"],
                app_config.strategy.min_profit_threshold.max(0.0),
            )
            .max(0.0),
            debug_quotes: env_bool_any(&["POOL_SCAN_DEBUG_QUOTES"], false),
            fixed_cost_lamports,
            executable_trade_sizes_sol,
            buy_slippage_bps: app_config
                .strategy
                .pumpswap_meteora_buy_slippage_bps
                .max(0.0),
            buy_slippage_bps_small_pool: app_config
                .strategy
                .pumpswap_meteora_buy_slippage_bps_small_pool
                .max(0.0),
            sell_slippage_bps: app_config
                .strategy
                .pumpswap_meteora_sell_slippage_bps
                .max(0.0),
            sell_slippage_bps_small_pool: app_config
                .strategy
                .pumpswap_meteora_sell_slippage_bps_small_pool
                .max(0.0),
            small_pool_liquidity_usdc: app_config
                .strategy
                .pumpswap_meteora_small_pool_liquidity_usdc
                .max(1.0),
            max_trade_depth_bps: app_config
                .strategy
                .pumpswap_meteora_max_trade_depth_bps
                .max(1.0),
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
            raydium_clmm_program_id,
            whirlpool_program_id,
            monitored_programs,
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

fn scanner_executable_edge_floor_pct(
    app_config: &config::Config,
    executable_trade_sizes_sol: &[f64],
    sol_usdc_price: f64,
    swap_cost_pct: f64,
) -> f64 {
    let max_trade_sol = executable_trade_sizes_sol
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .fold(0.0, f64::max);
    if max_trade_sol <= 0.0 || !sol_usdc_price.is_finite() || sol_usdc_price <= 0.0 {
        return swap_cost_pct.max(0.0);
    }

    let fixed_cost_sol = scanner_fixed_cost_lamports(app_config) as f64 / 1_000_000_000.0;
    let fixed_cost_pct = (fixed_cost_sol / max_trade_sol) * 100.0;
    (swap_cost_pct + fixed_cost_pct).max(0.0)
}

fn scanner_executable_trade_sizes_sol(app_config: &config::Config) -> Vec<f64> {
    let mut sizes = app_config
        .strategy
        .program_pair_trade_sizes_sol
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect::<Vec<_>>();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes.dedup_by(|left, right| (*left - *right).abs() < 0.0000001);
    if sizes.is_empty() {
        sizes.push(0.001);
    }
    if sizes.iter().copied().fold(0.0_f64, f64::max) < 0.05 {
        sizes.extend([0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 5.0, 8.0]);
        sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sizes.dedup_by(|left, right| (*left - *right).abs() < 0.0000001);
    }
    sizes
}

fn scanner_fixed_cost_lamports(app_config: &config::Config) -> u64 {
    let preflight_lamports =
        (app_config.strategy.preflight_gas_cost_sol.max(0.0) * 1_000_000_000.0).ceil() as u64;
    preflight_lamports
        .saturating_add(scanner_jito_tip_lamports(app_config))
        .saturating_add(BASE_SIGNATURE_FEE_LAMPORTS)
}

fn resolve_program_id(
    app_config: &config::Config,
    kind: config::ProgramKind,
    fallback: &str,
) -> String {
    app_config
        .program_by_kind(kind)
        .map(|program| program.program_id.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn scanner_jito_tip_lamports(app_config: &config::Config) -> u64 {
    env::var("JITO_TIP_LAMPORTS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or(app_config.execution.jito_tip_lamports)
        .filter(|value| *value > 0)
        .or_else(|| {
            env::var("JITO_MIN_TIP_LAMPORTS")
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(DEFAULT_JITO_MIN_TIP_LAMPORTS)
}

#[derive(Debug, Clone)]
struct ParsedPool {
    address: String,
    venue: &'static str,
    token_mint: String,
    quote_mint: String,
    token_vault: String,
    quote_vault: String,
    quote_liquidity_usdc: f64,
    token_decimals: u8,
    quote_decimals: u8,
    spot_price_sol_per_token: Option<f64>,
    clmm_sqrt_price_x64: Option<u128>,
    clmm_quote_is_token_0: Option<bool>,
    latest_slot: u64,
    hits: u64,
    activity_events_unix: VecDeque<u64>,
    meteora_active_id: Option<i32>,
    meteora_bin_step: Option<u16>,
    meteora_quote_is_x: Option<bool>,
    meteora_base_factor: Option<u16>,
    meteora_variable_fee_control: Option<u32>,
    meteora_base_fee_power_factor: Option<u8>,
    meteora_volatility_accumulator: Option<u32>,
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
    recent_trades_5m: u64,
    recent_trades_15m: u64,
    recent_volume_15m_usd: f64,
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
    #[serde(default)]
    recent_trades_5m: u64,
    #[serde(default)]
    recent_trades_15m: u64,
    #[serde(default)]
    recent_volume_15m_usd: f64,
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
        "池子扫描器启动：来源={}，间隔={}秒，输出={}，模式=价差优先，最低流动性=${:.0}，双边流动性>=${:.0}，价差门槛>0.00%，单池15分钟交易>={}，单池15分钟成交额>=${:.0}，近期窗口={}秒",
        config.source.label(),
        config.poll_secs,
        config.output_path,
        config.min_quote_liquidity_usdc,
        config.min_pair_liquidity_usdc,
        min_pool_recent_trades_15m(&config),
        min_pool_recent_volume_15m_usd(&config),
        config.recent_window_secs
    );

    if config.source == ScannerSource::Grpc {
        return run_grpc_scanner(config, run_once).await;
    }

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

    let (pumpswap_accounts, meteora_x_accounts, meteora_y_accounts) = tokio::join!(
        fetch_pumpswap_accounts(config),
        fetch_meteora_accounts(config, 88),
        fetch_meteora_accounts(config, 120),
    );
    let (
        raydium_token0_accounts,
        raydium_token1_accounts,
        whirlpool_a_accounts,
        whirlpool_b_accounts,
    ) = tokio::join!(
        fetch_raydium_clmm_accounts(config, 73),
        fetch_raydium_clmm_accounts(config, 105),
        fetch_whirlpool_accounts(config, 101),
        fetch_whirlpool_accounts(config, 181),
    );
    let pumpswap_accounts = program_accounts_or_empty("PumpSwap", pumpswap_accounts);
    let meteora_x_accounts = program_accounts_or_empty("Meteora tokenX", meteora_x_accounts);
    let meteora_y_accounts = program_accounts_or_empty("Meteora tokenY", meteora_y_accounts);
    let raydium_token0_accounts =
        program_accounts_or_empty("RaydiumCLMM token0", raydium_token0_accounts);
    let raydium_token1_accounts =
        program_accounts_or_empty("RaydiumCLMM token1", raydium_token1_accounts);
    let whirlpool_a_accounts = program_accounts_or_empty("Whirlpool tokenA", whirlpool_a_accounts);
    let whirlpool_b_accounts = program_accounts_or_empty("Whirlpool tokenB", whirlpool_b_accounts);

    let program_account_count = pumpswap_accounts.len()
        + meteora_x_accounts.len()
        + meteora_y_accounts.len()
        + raydium_token0_accounts.len()
        + raydium_token1_accounts.len()
        + whirlpool_a_accounts.len()
        + whirlpool_b_accounts.len();
    let mut parsed = parse_program_accounts(
        config,
        pumpswap_accounts,
        meteora_x_accounts,
        meteora_y_accounts,
        raydium_token0_accounts,
        raydium_token1_accounts,
        whirlpool_a_accounts,
        whirlpool_b_accounts,
    );
    hydrate_pool_mint_decimals(config, &mut parsed)
        .await
        .context("读取池子 mint decimals")?;
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

async fn run_grpc_scanner(config: ScannerConfig, run_once: bool) -> Result<()> {
    let monitored_programs = config
        .monitored_programs
        .iter()
        .map(|program| format!("{}={}", program.label, program.program_id))
        .collect::<Vec<_>>()
        .join(", ");
    tracing::info!(
        "启动 gRPC 池子索引：endpoint={}，flush={}秒，programs=[{}]",
        config.grpc_endpoint,
        config.grpc_flush_interval_secs.max(1),
        monitored_programs
    );

    let mut known_pools = bootstrap_grpc_known_pools(&config)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("gRPC 启动全量池引导失败，将仅使用流式增量：{}", error);
            HashMap::new()
        });
    if !known_pools.is_empty() {
        tracing::info!("gRPC 启动全量池引导完成：候选池={}", known_pools.len());
    }
    let mut reconnect_delay = Duration::from_millis(500);
    loop {
        match run_grpc_stream(&config, &mut known_pools, run_once).await {
            Ok(()) if run_once => return Ok(()),
            Ok(()) => tracing::warn!("gRPC 池子流结束，准备重连"),
            Err(error) if run_once => return Err(error),
            Err(error) => tracing::warn!("gRPC 池子流失败：{}，准备重连", error),
        }

        sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
    }
}

async fn run_grpc_stream(
    config: &ScannerConfig,
    known_pools: &mut HashMap<String, ParsedPool>,
    run_once: bool,
) -> Result<()> {
    let mut grpc_client = create_pool_grpc_client(config).await?;
    let request = build_pool_grpc_subscribe_request(config);
    let (_, mut stream) = grpc_client
        .subscribe_with_request(Some(request))
        .await
        .context("建立 Yellowstone 池子订阅")?;

    let flush_interval = Duration::from_secs(config.grpc_flush_interval_secs.max(1));
    let idle_timeout = Duration::from_secs(60);
    let run_once_deadline =
        run_once.then(|| Instant::now() + Duration::from_secs(config.grpc_run_once_secs.max(1)));
    let mut last_flush = Instant::now();
    let mut dirty_updates = 0usize;
    let mut received_updates = 0usize;
    let mut parsed_updates = 0usize;
    let mut transaction_updates = 0usize;
    let mut hydrated_updates = 0usize;
    let mut max_seen_slot = 0u64;
    let mut pending_lookup_accounts: HashSet<String> = HashSet::new();
    let tx_lookup_flush = Duration::from_millis(GRPC_TX_LOOKUP_FLUSH_MS);
    let mut last_tx_lookup_flush = Instant::now();

    tracing::info!(
        "gRPC 池子流已建立：监听程序=[{}]，接收账户更新和交易账户点查",
        config
            .monitored_programs
            .iter()
            .map(|program| program.label)
            .collect::<Vec<_>>()
            .join(", ")
    );

    loop {
        if let Some(deadline) = run_once_deadline {
            if Instant::now() >= deadline {
                let hydrated = hydrate_pending_grpc_pool_accounts(
                    config,
                    known_pools,
                    &mut pending_lookup_accounts,
                    max_seen_slot,
                )
                .await?;
                if hydrated > 0 {
                    parsed_updates += hydrated;
                    hydrated_updates += hydrated;
                    dirty_updates += hydrated;
                }
                if dirty_updates > 0 {
                    process_grpc_known_pools(config, known_pools, max_seen_slot).await?;
                }
                tracing::info!(
                    "gRPC 单次采样完成：收到={}，交易={}，解析池子={}，交易点查池子={}，候选池={}",
                    received_updates,
                    transaction_updates,
                    parsed_updates,
                    hydrated_updates,
                    known_pools.len()
                );
                return Ok(());
            }
        }

        let wait_for = run_once_deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1))
            })
            .unwrap_or(idle_timeout);

        let message = match timeout(wait_for, stream.next()).await {
            Ok(Some(message)) => message?,
            Ok(None) => return Ok(()),
            Err(_) if run_once => {
                let hydrated = hydrate_pending_grpc_pool_accounts(
                    config,
                    known_pools,
                    &mut pending_lookup_accounts,
                    max_seen_slot,
                )
                .await?;
                if hydrated > 0 {
                    parsed_updates += hydrated;
                    hydrated_updates += hydrated;
                    dirty_updates += hydrated;
                    last_tx_lookup_flush = Instant::now();
                }
                if last_flush.elapsed() >= flush_interval && dirty_updates > 0 {
                    process_grpc_known_pools(config, known_pools, max_seen_slot).await?;
                    dirty_updates = 0;
                    last_flush = Instant::now();
                }
                continue;
            }
            Err(_) => anyhow::bail!(
                "{} 秒内没有收到 Yellowstone 池子更新",
                idle_timeout.as_secs()
            ),
        };

        let Some(update) = message.update_oneof else {
            continue;
        };
        match update {
            subscribe_update::UpdateOneof::Account(account_update) => {
                received_updates += 1;
                max_seen_slot = max_seen_slot.max(account_update.slot);
                let Some(account) = account_update.account else {
                    continue;
                };

                let pubkey = bs58::encode(&account.pubkey).into_string();
                if config.excluded_market_addresses.contains(&pubkey) {
                    known_pools.remove(&pubkey);
                    continue;
                }
                if account.lamports == 0 || account.data.is_empty() {
                    known_pools.remove(&pubkey);
                    continue;
                }

                let owner = bs58::encode(&account.owner).into_string();
                let parsed = parse_grpc_pool_account(config, &pubkey, &owner, &account.data);

                let Some(mut pool) = parsed else {
                    known_pools.remove(&pubkey);
                    continue;
                };
                pool.latest_slot = account_update.slot;

                upsert_known_pool(known_pools, pool);
                if !account_update.is_startup
                    && record_pool_activity(known_pools, &pubkey, unix_now(), account_update.slot)
                {
                    dirty_updates += 1;
                }
                parsed_updates += 1;
                dirty_updates += 1;
            }
            subscribe_update::UpdateOneof::Transaction(transaction_update) => {
                received_updates += 1;
                transaction_updates += 1;
                max_seen_slot = max_seen_slot.max(transaction_update.slot);
                for account in transaction_pool_lookup_candidates(config, &transaction_update) {
                    if known_pools.contains_key(&account) {
                        if record_pool_activity(
                            known_pools,
                            &account,
                            unix_now(),
                            transaction_update.slot,
                        ) {
                            dirty_updates += 1;
                        }
                    } else {
                        pending_lookup_accounts.insert(account);
                    }
                }
            }
            _ => continue,
        }

        if !pending_lookup_accounts.is_empty()
            && (pending_lookup_accounts.len() >= GRPC_TX_LOOKUP_BATCH
                || last_tx_lookup_flush.elapsed() >= tx_lookup_flush)
        {
            let hydrated = hydrate_pending_grpc_pool_accounts(
                config,
                known_pools,
                &mut pending_lookup_accounts,
                max_seen_slot,
            )
            .await?;
            if hydrated > 0 {
                parsed_updates += hydrated;
                hydrated_updates += hydrated;
                dirty_updates += hydrated;
            }
            last_tx_lookup_flush = Instant::now();
        }

        if dirty_updates >= GRPC_DIRTY_FLUSH_UPDATES || last_flush.elapsed() >= flush_interval {
            process_grpc_known_pools(config, known_pools, max_seen_slot).await?;
            dirty_updates = 0;
            last_flush = Instant::now();
        }
    }
}

fn parse_grpc_pool_account(
    config: &ScannerConfig,
    pubkey: &str,
    owner: &str,
    data: &[u8],
) -> Option<ParsedPool> {
    if owner == config.pumpswap_program_id {
        parse_pumpswap_pool(config, pubkey, data)
    } else if owner == config.meteora_program_id {
        parse_meteora_pool(config, pubkey, data)
    } else if owner == config.raydium_clmm_program_id {
        parse_raydium_clmm_pool(config, pubkey, data)
    } else if owner == config.whirlpool_program_id {
        parse_whirlpool_pool(config, pubkey, data)
    } else {
        None
    }
}

fn is_monitored_program_id(config: &ScannerConfig, program_id: &str) -> bool {
    config
        .monitored_programs
        .iter()
        .any(|program| program.program_id == program_id)
}

async fn bootstrap_grpc_known_pools(config: &ScannerConfig) -> Result<HashMap<String, ParsedPool>> {
    let current_slot = rpc::get_slot(&config.rpc_url).await.unwrap_or_default();
    let (pumpswap_accounts, meteora_x_accounts, meteora_y_accounts) = tokio::join!(
        fetch_pumpswap_accounts(config),
        fetch_meteora_accounts(config, 88),
        fetch_meteora_accounts(config, 120),
    );
    let (
        raydium_token0_accounts,
        raydium_token1_accounts,
        whirlpool_a_accounts,
        whirlpool_b_accounts,
    ) = tokio::join!(
        fetch_raydium_clmm_accounts(config, 73),
        fetch_raydium_clmm_accounts(config, 105),
        fetch_whirlpool_accounts(config, 101),
        fetch_whirlpool_accounts(config, 181),
    );
    let pumpswap_accounts = program_accounts_or_empty("PumpSwap", pumpswap_accounts);
    let meteora_x_accounts = program_accounts_or_empty("Meteora tokenX", meteora_x_accounts);
    let meteora_y_accounts = program_accounts_or_empty("Meteora tokenY", meteora_y_accounts);
    let raydium_token0_accounts =
        program_accounts_or_empty("RaydiumCLMM token0", raydium_token0_accounts);
    let raydium_token1_accounts =
        program_accounts_or_empty("RaydiumCLMM token1", raydium_token1_accounts);
    let whirlpool_a_accounts = program_accounts_or_empty("Whirlpool tokenA", whirlpool_a_accounts);
    let whirlpool_b_accounts = program_accounts_or_empty("Whirlpool tokenB", whirlpool_b_accounts);

    let mut pools = parse_program_accounts(
        config,
        pumpswap_accounts,
        meteora_x_accounts,
        meteora_y_accounts,
        raydium_token0_accounts,
        raydium_token1_accounts,
        whirlpool_a_accounts,
        whirlpool_b_accounts,
    );
    for pool in pools.values_mut() {
        pool.latest_slot = current_slot;
    }
    hydrate_pool_mint_decimals(config, &mut pools)
        .await
        .context("gRPC 启动全量池引导：读取 mint decimals")?;

    Ok(pools)
}

fn upsert_known_pool(known_pools: &mut HashMap<String, ParsedPool>, pool: ParsedPool) {
    known_pools
        .entry(pool.address.clone())
        .and_modify(|existing| {
            existing.venue = pool.venue;
            existing.token_mint = pool.token_mint.clone();
            existing.quote_mint = pool.quote_mint.clone();
            existing.token_vault = pool.token_vault.clone();
            existing.quote_vault = pool.quote_vault.clone();
            existing.token_decimals = pool.token_decimals;
            existing.quote_decimals = pool.quote_decimals;
            existing.spot_price_sol_per_token = pool.spot_price_sol_per_token;
            existing.clmm_sqrt_price_x64 = pool.clmm_sqrt_price_x64;
            existing.clmm_quote_is_token_0 = pool.clmm_quote_is_token_0;
            existing.latest_slot = existing.latest_slot.max(pool.latest_slot);
            existing.hits = existing.hits.saturating_add(1);
            existing.meteora_active_id = pool.meteora_active_id;
            existing.meteora_bin_step = pool.meteora_bin_step;
            existing.meteora_quote_is_x = pool.meteora_quote_is_x;
            existing.meteora_base_factor = pool.meteora_base_factor;
            existing.meteora_variable_fee_control = pool.meteora_variable_fee_control;
            existing.meteora_base_fee_power_factor = pool.meteora_base_fee_power_factor;
            existing.meteora_volatility_accumulator = pool.meteora_volatility_accumulator;
        })
        .or_insert(pool);
}

async fn hydrate_pending_grpc_pool_accounts(
    config: &ScannerConfig,
    known_pools: &mut HashMap<String, ParsedPool>,
    pending_lookup_accounts: &mut HashSet<String>,
    latest_slot: u64,
) -> Result<usize> {
    if pending_lookup_accounts.is_empty() {
        return Ok(0);
    }

    let accounts = pending_lookup_accounts.drain().collect::<Vec<_>>();
    let account_data = rpc::get_multiple_accounts_owner_data(&config.rpc_url, &accounts)
        .await
        .context("gRPC 交易账户点查：读取候选池子账户")?;

    let mut parsed = 0usize;
    for (address, account) in account_data {
        if config.excluded_market_addresses.contains(&address) {
            continue;
        }
        let Some(mut pool) =
            parse_grpc_pool_account(config, &address, &account.owner, &account.data)
        else {
            continue;
        };
        pool.latest_slot = latest_slot;
        upsert_known_pool(known_pools, pool);
        parsed += 1;
    }

    Ok(parsed)
}

fn transaction_pool_lookup_candidates(
    config: &ScannerConfig,
    update: &SubscribeUpdateTransaction,
) -> HashSet<String> {
    let Some(info) = update.transaction.as_ref() else {
        return HashSet::new();
    };
    let Some(transaction) = info.transaction.as_ref() else {
        return HashSet::new();
    };
    let Some(message) = transaction.message.as_ref() else {
        return HashSet::new();
    };

    let mut account_keys = message
        .account_keys
        .iter()
        .filter_map(|bytes| pubkey_from_bytes(bytes))
        .collect::<Vec<_>>();
    if let Some(meta) = info.meta.as_ref() {
        account_keys.extend(
            meta.loaded_writable_addresses
                .iter()
                .filter_map(|bytes| pubkey_from_bytes(bytes)),
        );
        account_keys.extend(
            meta.loaded_readonly_addresses
                .iter()
                .filter_map(|bytes| pubkey_from_bytes(bytes)),
        );
    }

    let mut out = HashSet::new();
    for instruction in &message.instructions {
        collect_dex_instruction_accounts(
            config,
            &account_keys,
            instruction.program_id_index,
            &instruction.accounts,
            &mut out,
        );
    }
    if let Some(meta) = info.meta.as_ref() {
        for inner in &meta.inner_instructions {
            for instruction in &inner.instructions {
                collect_dex_instruction_accounts(
                    config,
                    &account_keys,
                    instruction.program_id_index,
                    &instruction.accounts,
                    &mut out,
                );
            }
        }
    }

    out
}

fn collect_dex_instruction_accounts(
    config: &ScannerConfig,
    account_keys: &[String],
    program_id_index: u32,
    instruction_accounts: &[u8],
    out: &mut HashSet<String>,
) {
    let Some(program_id) = account_keys.get(program_id_index as usize) else {
        return;
    };
    if !is_monitored_program_id(config, program_id) {
        return;
    }

    for index in instruction_accounts {
        let Some(account) = account_keys.get(*index as usize) else {
            continue;
        };
        if is_grpc_pool_lookup_candidate(config, account) {
            out.insert(account.clone());
        }
    }
}

fn is_grpc_pool_lookup_candidate(config: &ScannerConfig, account: &str) -> bool {
    !is_monitored_program_id(config, account)
        && account != config.sol_mint
        && !config.excluded_market_addresses.contains(account)
}

fn pubkey_from_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() == 32 {
        Some(bs58::encode(bytes).into_string())
    } else {
        None
    }
}

async fn process_grpc_known_pools(
    config: &ScannerConfig,
    known_pools: &mut HashMap<String, ParsedPool>,
    _max_seen_slot: u64,
) -> Result<PollReport> {
    if known_pools.is_empty() {
        return Ok(PollReport::default());
    }

    hydrate_pool_mint_decimals(config, known_pools)
        .await
        .context("gRPC 池子校验：读取池子 mint decimals")?;

    let parsed = known_pools.clone();
    let quote_pools = parsed.len();
    let candidate_counts = parsed_pool_venue_counts(&parsed);
    let vault_accounts = parsed
        .values()
        .flat_map(|pool| [pool.quote_vault.clone(), pool.token_vault.clone()])
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let vault_data = rpc::get_multiple_accounts_data(&config.rpc_url, &vault_accounts)
        .await
        .context("gRPC 池子校验：读取池子金库余额")?;

    let liquidity_passed = apply_liquidity_filter(config, parsed, &vault_data);
    let liquidity_passed_count = liquidity_passed.len();
    let bin_checked = filter_meteora_bin_arrays(config, liquidity_passed)
        .await
        .context("gRPC 池子校验：检查 Meteora bin array")?;
    let recent_passed_count = bin_checked.len();
    let validated_counts = parsed_pool_venue_counts(&bin_checked);
    let now_unix = unix_now();
    let edge_diagnostics = grpc_edge_diagnostics(config, &bin_checked, &vault_data, now_unix);
    let validated = select_routeable_grpc_pools(config, bin_checked, &vault_data, now_unix);
    let fresh_pool_count = validated.len();

    let state = update_state(config, validated)?;
    write_active_addresses(&config.output_path, &state)?;
    write_state(&config.state_path, &state)?;
    log_routeable_pool_summary(&state);

    let best_spot = edge_diagnostics
        .best_spot_edge
        .as_ref()
        .map(|edge| {
            format!(
                "{:.2}% {} liq=${:.0} token={}",
                edge.edge_pct, edge.route, edge.pair_liquidity_usdc, edge.token_mint
            )
        })
        .unwrap_or_else(|| "无".to_string());

    tracing::info!(
        "gRPC 池子校验完成：候选={}({})，流动性通过={}，验证通过={}({})，双边候选={}，可定价双边={}，价差候选={}，最大价差={}，价差门槛>0.00%，输出池子={}，保留池子={}，输出币种={}",
        quote_pools,
        candidate_counts.compact(),
        liquidity_passed_count,
        recent_passed_count,
        validated_counts.compact(),
        edge_diagnostics.paired_tokens,
        edge_diagnostics.priced_tokens,
        edge_diagnostics.spread_tokens,
        best_spot,
        fresh_pool_count,
        state.pools.len(),
        routeable_token_count(&state)
    );

    Ok(PollReport {
        program_accounts: quote_pools,
        quote_pools,
        liquidity_passed: liquidity_passed_count,
        recent_passed: recent_passed_count,
        routeable_tokens: routeable_token_count(&state),
        fresh_pools: fresh_pool_count,
        active_pools: state.pools.len(),
    })
}

async fn create_pool_grpc_client(
    config: &ScannerConfig,
) -> Result<GeyserGrpcClient<impl yellowstone_grpc_client::Interceptor + Clone>> {
    let mut endpoint = config.grpc_endpoint.clone();
    if !endpoint.starts_with("grpc://") && !endpoint.starts_with("grpcs://") {
        endpoint = format!("grpc://{}", endpoint);
    }

    let mut builder = GeyserGrpcClient::build_from_shared(endpoint)?;
    if let Some(token) = &config.grpc_token {
        builder = builder.x_token(Some(token.clone()))?;
    }
    builder.connect().await.context("连接 Yellowstone gRPC")
}

fn build_pool_grpc_subscribe_request(config: &ScannerConfig) -> SubscribeRequest {
    let mut accounts = HashMap::new();
    let mut transactions = HashMap::new();
    accounts.insert(
        "pumpswap_sol_quote_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.pumpswap_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, PUMPSWAP_POOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(75, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "pumpswap_sol_base_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.pumpswap_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, PUMPSWAP_POOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(43, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "meteora_sol_x_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.meteora_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, METEORA_LB_PAIR_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(88, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "meteora_sol_y_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.meteora_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, METEORA_LB_PAIR_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(120, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "raydium_clmm_sol_token0_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.raydium_clmm_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, RAYDIUM_CLMM_POOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(73, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "raydium_clmm_sol_token1_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.raydium_clmm_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, RAYDIUM_CLMM_POOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(105, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "whirlpool_sol_a_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.whirlpool_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, WHIRLPOOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(101, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    accounts.insert(
        "whirlpool_sol_b_pools".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.whirlpool_program_id.clone()],
            filters: vec![
                grpc_memcmp_bytes_filter(0, WHIRLPOOL_DISCRIMINATOR.as_slice()),
                grpc_memcmp_base58_filter(181, &config.sol_mint),
            ],
            nonempty_txn_signature: None,
        },
    );
    for program in &config.monitored_programs {
        transactions.insert(
            format!("{}_txs", grpc_request_key_slug(program.label)),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(false),
                signature: None,
                account_include: vec![program.program_id.clone()],
                account_exclude: vec![],
                account_required: vec![],
            },
        );
    }

    SubscribeRequest {
        accounts,
        slots: HashMap::new(),
        transactions,
        transactions_status: HashMap::new(),
        blocks: HashMap::new(),
        blocks_meta: HashMap::new(),
        entry: HashMap::new(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: vec![],
        ping: None,
        from_slot: None,
    }
}

fn grpc_request_key_slug(label: &str) -> String {
    label
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn grpc_memcmp_bytes_filter(offset: u64, bytes: &[u8]) -> SubscribeRequestFilterAccountsFilter {
    SubscribeRequestFilterAccountsFilter {
        filter: Some(subscribe_request_filter_accounts_filter::Filter::Memcmp(
            SubscribeRequestFilterAccountsFilterMemcmp {
                offset,
                data: Some(
                    subscribe_request_filter_accounts_filter_memcmp::Data::Bytes(bytes.to_vec()),
                ),
            },
        )),
    }
}

fn grpc_memcmp_base58_filter(offset: u64, value: &str) -> SubscribeRequestFilterAccountsFilter {
    SubscribeRequestFilterAccountsFilter {
        filter: Some(subscribe_request_filter_accounts_filter::Filter::Memcmp(
            SubscribeRequestFilterAccountsFilterMemcmp {
                offset,
                data: Some(
                    subscribe_request_filter_accounts_filter_memcmp::Data::Base58(
                        value.to_string(),
                    ),
                ),
            },
        )),
    }
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
                    recent_trades_5m: (pool.m15_trades as u64) / 3,
                    recent_trades_15m: pool.m15_trades as u64,
                    recent_volume_15m_usd: pool.m15_volume_usd,
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

async fn fetch_raydium_clmm_accounts(
    config: &ScannerConfig,
    token_mint_offset: usize,
) -> Result<Vec<ProgramAccount>> {
    let filters = vec![
        memcmp_filter(0, &RAYDIUM_CLMM_POOL_DISCRIMINATOR),
        memcmp_filter_string(token_mint_offset, &config.sol_mint),
    ];
    fetch_program_accounts(&config.rpc_url, &config.raydium_clmm_program_id, filters).await
}

async fn fetch_whirlpool_accounts(
    config: &ScannerConfig,
    token_mint_offset: usize,
) -> Result<Vec<ProgramAccount>> {
    let filters = vec![
        memcmp_filter(0, WHIRLPOOL_DISCRIMINATOR.as_slice()),
        memcmp_filter_string(token_mint_offset, &config.sol_mint),
    ];
    fetch_program_accounts(&config.rpc_url, &config.whirlpool_program_id, filters).await
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

fn program_accounts_or_empty(
    label: &str,
    result: Result<Vec<ProgramAccount>>,
) -> Vec<ProgramAccount> {
    match result {
        Ok(accounts) => accounts,
        Err(error) => {
            tracing::warn!("{} 账户拉取失败，跳过该分支：{:#}", label, error);
            Vec::new()
        }
    }
}

fn parse_program_accounts(
    config: &ScannerConfig,
    pumpswap_accounts: Vec<ProgramAccount>,
    meteora_x_accounts: Vec<ProgramAccount>,
    meteora_y_accounts: Vec<ProgramAccount>,
    raydium_token0_accounts: Vec<ProgramAccount>,
    raydium_token1_accounts: Vec<ProgramAccount>,
    whirlpool_a_accounts: Vec<ProgramAccount>,
    whirlpool_b_accounts: Vec<ProgramAccount>,
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

    for account in raydium_token0_accounts
        .into_iter()
        .chain(raydium_token1_accounts)
    {
        if config.excluded_market_addresses.contains(&account.pubkey) {
            continue;
        }
        if out.contains_key(&account.pubkey) {
            continue;
        }
        let Ok(data) = decode_account_data(&account.account, &account.pubkey) else {
            continue;
        };
        if let Some(pool) = parse_raydium_clmm_pool(config, &account.pubkey, &data) {
            out.insert(pool.address.clone(), pool);
        }
    }

    for account in whirlpool_a_accounts.into_iter().chain(whirlpool_b_accounts) {
        if config.excluded_market_addresses.contains(&account.pubkey) {
            continue;
        }
        if out.contains_key(&account.pubkey) {
            continue;
        }
        let Ok(data) = decode_account_data(&account.account, &account.pubkey) else {
            continue;
        };
        if let Some(pool) = parse_whirlpool_pool(config, &account.pubkey, &data) {
            out.insert(pool.address.clone(), pool);
        }
    }

    out
}

fn parse_pumpswap_pool(config: &ScannerConfig, address: &str, data: &[u8]) -> Option<ParsedPool> {
    if data.len() < PUMPSWAP_POOL_MIN_DATA_SIZE {
        return None;
    }
    if data.get(..8) != Some(PUMPSWAP_POOL_DISCRIMINATOR.as_slice()) {
        return None;
    }

    let base_mint = pubkey_at(data, 43)?;
    let raw_quote_mint = pubkey_at(data, 75)?;
    let base_vault = pubkey_at(data, 139)?;
    let raw_quote_vault = pubkey_at(data, 171)?;
    let (token_mint, token_vault, quote_vault) =
        if raw_quote_mint == config.sol_mint && base_mint != config.sol_mint {
            (base_mint, base_vault, raw_quote_vault)
        } else if base_mint == config.sol_mint && raw_quote_mint != config.sol_mint {
            (raw_quote_mint, raw_quote_vault, base_vault)
        } else {
            return None;
        };
    if config.excluded_token_mints.contains(&token_mint) {
        return None;
    }
    let token_decimals = mint_decimals(&token_mint, &config.sol_mint) as u8;
    let quote_decimals = mint_decimals(&config.sol_mint, &config.sol_mint) as u8;

    Some(ParsedPool {
        address: address.to_string(),
        venue: "pumpswap",
        token_mint,
        quote_mint: config.sol_mint.clone(),
        token_vault,
        quote_vault,
        quote_liquidity_usdc: 0.0,
        token_decimals,
        quote_decimals,
        spot_price_sol_per_token: None,
        clmm_sqrt_price_x64: None,
        clmm_quote_is_token_0: None,
        latest_slot: 0,
        hits: 1,
        activity_events_unix: VecDeque::new(),
        meteora_active_id: None,
        meteora_bin_step: None,
        meteora_quote_is_x: None,
        meteora_base_factor: None,
        meteora_variable_fee_control: None,
        meteora_base_fee_power_factor: None,
        meteora_volatility_accumulator: None,
    })
}

fn parse_meteora_pool(config: &ScannerConfig, address: &str, data: &[u8]) -> Option<ParsedPool> {
    if data.len() < 232 {
        return None;
    }
    if data.get(..8) != Some(METEORA_LB_PAIR_DISCRIMINATOR.as_slice()) {
        return None;
    }

    let base_factor = read_u16(data, 8)?;
    let variable_fee_control = read_u32(data, 16)?;
    let base_fee_power_factor = read_u8(data, 34)?;
    let volatility_accumulator = read_u32(data, 40)?;
    let active_id = read_i32(data, 76)?;
    let bin_step = read_u16(data, 80)?;
    let token_x_mint = pubkey_at(data, 88)?;
    let token_y_mint = pubkey_at(data, 120)?;
    let reserve_x = pubkey_at(data, 152)?;
    let reserve_y = pubkey_at(data, 184)?;

    let (token_mint, token_vault, quote_vault, quote_is_x) =
        if token_x_mint == config.sol_mint && token_y_mint != config.sol_mint {
            let wsol_vault = reserve_x;
            let meme_vault = reserve_y;
            (token_y_mint, meme_vault, wsol_vault, true)
        } else if token_y_mint == config.sol_mint && token_x_mint != config.sol_mint {
            let wsol_vault = reserve_y;
            let meme_vault = reserve_x;
            (token_x_mint, meme_vault, wsol_vault, false)
        } else {
            return None;
        };

    if config.excluded_token_mints.contains(&token_mint) {
        return None;
    }
    let token_decimals = mint_decimals(&token_mint, &config.sol_mint) as u8;
    let quote_decimals = mint_decimals(&config.sol_mint, &config.sol_mint) as u8;

    Some(ParsedPool {
        address: address.to_string(),
        venue: "meteora",
        token_mint,
        quote_mint: config.sol_mint.clone(),
        token_vault,
        quote_vault,
        quote_liquidity_usdc: 0.0,
        token_decimals,
        quote_decimals,
        spot_price_sol_per_token: None,
        clmm_sqrt_price_x64: None,
        clmm_quote_is_token_0: None,
        latest_slot: 0,
        hits: 1,
        activity_events_unix: VecDeque::new(),
        meteora_active_id: Some(active_id),
        meteora_bin_step: Some(bin_step),
        meteora_quote_is_x: Some(quote_is_x),
        meteora_base_factor: Some(base_factor),
        meteora_variable_fee_control: Some(variable_fee_control),
        meteora_base_fee_power_factor: Some(base_fee_power_factor),
        meteora_volatility_accumulator: Some(volatility_accumulator),
    })
}

fn parse_raydium_clmm_pool(
    config: &ScannerConfig,
    address: &str,
    data: &[u8],
) -> Option<ParsedPool> {
    if data.len() < RAYDIUM_CLMM_POOL_MIN_DATA_SIZE {
        return None;
    }
    if data.get(..8) != Some(RAYDIUM_CLMM_POOL_DISCRIMINATOR.as_slice()) {
        return None;
    }

    let token_0_mint = pubkey_at(data, 73)?;
    let token_1_mint = pubkey_at(data, 105)?;
    let token_0_vault = pubkey_at(data, 137)?;
    let token_1_vault = pubkey_at(data, 169)?;
    let token_0_decimals = data.get(233).copied()?;
    let token_1_decimals = data.get(234).copied()?;
    let sqrt_price_x64 = read_u128(data, 253)?;
    let sqrt_price = sqrt_price_x64 as f64 / 2_f64.powi(64);
    let price_1_per_0 =
        sqrt_price * sqrt_price * 10_f64.powi(token_0_decimals as i32 - token_1_decimals as i32);
    if !price_1_per_0.is_finite() || price_1_per_0 <= 0.0 {
        return None;
    }

    let (
        token_mint,
        token_vault,
        quote_vault,
        token_decimals,
        quote_decimals,
        quote_is_token_0,
        spot_price_sol_per_token,
    ) = if token_0_mint == config.sol_mint && token_1_mint != config.sol_mint {
        (
            token_1_mint,
            token_1_vault,
            token_0_vault,
            token_1_decimals,
            token_0_decimals,
            true,
            1.0 / price_1_per_0,
        )
    } else if token_1_mint == config.sol_mint && token_0_mint != config.sol_mint {
        (
            token_0_mint,
            token_0_vault,
            token_1_vault,
            token_0_decimals,
            token_1_decimals,
            false,
            price_1_per_0,
        )
    } else {
        return None;
    };

    if config.excluded_token_mints.contains(&token_mint)
        || !spot_price_sol_per_token.is_finite()
        || spot_price_sol_per_token <= 0.0
    {
        return None;
    }

    Some(ParsedPool {
        address: address.to_string(),
        venue: "raydium",
        token_mint,
        quote_mint: config.sol_mint.clone(),
        token_vault,
        quote_vault,
        quote_liquidity_usdc: 0.0,
        token_decimals,
        quote_decimals,
        spot_price_sol_per_token: Some(spot_price_sol_per_token),
        clmm_sqrt_price_x64: Some(sqrt_price_x64),
        clmm_quote_is_token_0: Some(quote_is_token_0),
        latest_slot: 0,
        hits: 1,
        activity_events_unix: VecDeque::new(),
        meteora_active_id: None,
        meteora_bin_step: None,
        meteora_quote_is_x: None,
        meteora_base_factor: None,
        meteora_variable_fee_control: None,
        meteora_base_fee_power_factor: None,
        meteora_volatility_accumulator: None,
    })
}

fn parse_whirlpool_pool(config: &ScannerConfig, address: &str, data: &[u8]) -> Option<ParsedPool> {
    if data.get(..8) != Some(WHIRLPOOL_DISCRIMINATOR.as_slice()) {
        return None;
    }
    let state = OrcaWhirlpool::from_bytes(data).ok()?;
    let token_mint_a = state.token_mint_a.to_string();
    let token_mint_b = state.token_mint_b.to_string();
    let token_vault_a = state.token_vault_a.to_string();
    let token_vault_b = state.token_vault_b.to_string();
    let decimals_a = mint_decimals(&token_mint_a, &config.sol_mint) as u8;
    let decimals_b = mint_decimals(&token_mint_b, &config.sol_mint) as u8;
    let price_b_per_a =
        orca_whirlpools_core::sqrt_price_to_price(state.sqrt_price, decimals_a, decimals_b);
    if !price_b_per_a.is_finite() || price_b_per_a <= 0.0 {
        return None;
    }

    let (
        token_mint,
        token_vault,
        quote_vault,
        token_decimals,
        quote_decimals,
        quote_is_token_0,
        spot_price_sol_per_token,
    ) = if token_mint_a == config.sol_mint && token_mint_b != config.sol_mint {
        (
            token_mint_b,
            token_vault_b,
            token_vault_a,
            decimals_b,
            decimals_a,
            true,
            1.0 / price_b_per_a,
        )
    } else if token_mint_b == config.sol_mint && token_mint_a != config.sol_mint {
        (
            token_mint_a,
            token_vault_a,
            token_vault_b,
            decimals_a,
            decimals_b,
            false,
            price_b_per_a,
        )
    } else {
        return None;
    };

    if config.excluded_token_mints.contains(&token_mint)
        || !spot_price_sol_per_token.is_finite()
        || spot_price_sol_per_token <= 0.0
    {
        return None;
    }

    Some(ParsedPool {
        address: address.to_string(),
        venue: "whirlpool",
        token_mint,
        quote_mint: config.sol_mint.clone(),
        token_vault,
        quote_vault,
        quote_liquidity_usdc: 0.0,
        token_decimals,
        quote_decimals,
        spot_price_sol_per_token: Some(spot_price_sol_per_token),
        clmm_sqrt_price_x64: Some(state.sqrt_price),
        clmm_quote_is_token_0: Some(quote_is_token_0),
        latest_slot: 0,
        hits: 1,
        activity_events_unix: VecDeque::new(),
        meteora_active_id: None,
        meteora_bin_step: None,
        meteora_quote_is_x: None,
        meteora_base_factor: None,
        meteora_variable_fee_control: None,
        meteora_base_fee_power_factor: None,
        meteora_volatility_accumulator: None,
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

async fn hydrate_pool_mint_decimals(
    config: &ScannerConfig,
    pools: &mut HashMap<String, ParsedPool>,
) -> Result<()> {
    let mint_addresses = pools
        .values()
        .flat_map(|pool| [pool.token_mint.clone(), pool.quote_mint.clone()])
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if mint_addresses.is_empty() {
        return Ok(());
    }

    let mint_data = rpc::get_multiple_accounts_data(&config.rpc_url, &mint_addresses).await?;
    for pool in pools.values_mut() {
        if let Some(decimals) = mint_data
            .get(&pool.token_mint)
            .and_then(|data| parse_mint_decimals(data))
        {
            pool.token_decimals = decimals;
        }
        if let Some(decimals) = mint_data
            .get(&pool.quote_mint)
            .and_then(|data| parse_mint_decimals(data))
        {
            pool.quote_decimals = decimals;
        }
        refresh_pool_spot_price(pool);
    }

    Ok(())
}

fn parse_mint_decimals(data: &[u8]) -> Option<u8> {
    data.get(TOKEN_MINT_DECIMALS_OFFSET).copied()
}

fn refresh_pool_spot_price(pool: &mut ParsedPool) {
    if !matches!(pool.venue, "raydium" | "whirlpool") {
        return;
    }

    let Some(sqrt_price_x64) = pool.clmm_sqrt_price_x64 else {
        pool.spot_price_sol_per_token = None;
        return;
    };
    let Some(quote_is_token_0) = pool.clmm_quote_is_token_0 else {
        pool.spot_price_sol_per_token = None;
        return;
    };
    pool.spot_price_sol_per_token = clmm_spot_price_sol_per_token(
        sqrt_price_x64,
        pool.token_decimals,
        pool.quote_decimals,
        quote_is_token_0,
    );
}

fn clmm_spot_price_sol_per_token(
    sqrt_price_x64: u128,
    token_decimals: u8,
    quote_decimals: u8,
    quote_is_token_0: bool,
) -> Option<f64> {
    let sqrt_price = sqrt_price_x64 as f64 / 2_f64.powi(64);
    if !sqrt_price.is_finite() || sqrt_price <= 0.0 {
        return None;
    }

    let (decimals_0, decimals_1) = if quote_is_token_0 {
        (quote_decimals, token_decimals)
    } else {
        (token_decimals, quote_decimals)
    };
    let price_1_per_0 =
        sqrt_price * sqrt_price * 10_f64.powi(decimals_0 as i32 - decimals_1 as i32);
    if !price_1_per_0.is_finite() || price_1_per_0 <= 0.0 {
        return None;
    }

    let spot = if quote_is_token_0 {
        1.0 / price_1_per_0
    } else {
        price_1_per_0
    };
    (spot.is_finite() && spot > 0.0).then_some(spot)
}

#[allow(dead_code)]
fn pool_price_ratio(
    pool: &ParsedPool,
    vault_data: &HashMap<String, Vec<u8>>,
    _sol_mint: &str,
) -> Option<f64> {
    match pool.venue {
        "pumpswap" => {
            let quote_amount = vault_data
                .get(&pool.quote_vault)
                .and_then(|data| parse_token_account_amount(data))?;
            let token_amount = vault_data
                .get(&pool.token_vault)
                .and_then(|data| parse_token_account_amount(data))?;
            if quote_amount == 0 || token_amount == 0 {
                return None;
            }

            normalize_pool_price(
                quote_amount as f64 / token_amount as f64,
                pool.token_decimals,
                pool.quote_decimals,
            )
        }
        "meteora" => {
            let active_id = pool.meteora_active_id?;
            let bin_step = pool.meteora_bin_step?;
            let quote_is_x = pool.meteora_quote_is_x?;
            if bin_step == 0 || bin_step > 1000 || active_id.abs() > 100000 {
                return None;
            }

            let base = 1.0 + (bin_step as f64 / 10_000.0);
            let price_y_per_x = base.powf(active_id as f64);
            if !price_y_per_x.is_finite() || price_y_per_x <= 0.0 {
                return None;
            }

            let raw_price_in_quote = if quote_is_x {
                1.0 / price_y_per_x
            } else {
                price_y_per_x
            };

            normalize_pool_price(raw_price_in_quote, pool.token_decimals, pool.quote_decimals)
        }
        "raydium" | "whirlpool" => pool.spot_price_sol_per_token,
        _ => None,
    }
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

async fn fetch_meteora_bin_array_states(
    config: &ScannerConfig,
    pools: &HashMap<String, ParsedPool>,
) -> Result<HashMap<String, Vec<ScannerMeteoraBinArray>>> {
    let mut pool_to_addresses: HashMap<String, Vec<String>> = HashMap::new();
    for pool in pools.values() {
        if pool.venue != "meteora" {
            continue;
        }
        let Some(active_id) = pool.meteora_active_id else {
            continue;
        };
        let addresses = derive_nearby_bin_array_addresses(
            &config.meteora_program_id,
            &pool.address,
            active_id,
            METEORA_BIN_ARRAY_CHECK_EACH_SIDE,
        )?;
        pool_to_addresses
            .entry(pool.address.clone())
            .or_default()
            .extend(addresses);
    }

    let mut unique_addresses = HashSet::new();
    for addresses in pool_to_addresses.values() {
        for address in addresses {
            unique_addresses.insert(address.clone());
        }
    }

    let bin_data = if unique_addresses.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(
            &config.rpc_url,
            &unique_addresses.into_iter().collect::<Vec<_>>(),
        )
        .await
        .context("读取 Meteora bin array 账户")?
    };

    let mut parsed_by_pool: HashMap<String, Vec<ScannerMeteoraBinArray>> = HashMap::new();
    for (pool_address, addresses) in pool_to_addresses {
        let mut arrays = Vec::new();
        for address in addresses {
            let Some(data) = bin_data.get(&address) else {
                continue;
            };
            if let Some(array) = parse_scanner_meteora_bin_array(data) {
                if array.lb_pair == pool_address {
                    arrays.push(array);
                }
            }
        }
        if !arrays.is_empty() {
            parsed_by_pool.insert(pool_address, arrays);
        }
    }

    Ok(parsed_by_pool)
}

async fn fetch_grpc_quote_context(
    config: &ScannerConfig,
    pools: &HashMap<String, ParsedPool>,
) -> Result<GrpcQuoteContext> {
    let mut context = GrpcQuoteContext::default();
    let raydium_pool_addresses = pools
        .values()
        .filter(|pool| pool.venue == "raydium")
        .map(|pool| pool.address.clone())
        .collect::<Vec<_>>();
    let whirlpool_addresses = pools
        .values()
        .filter(|pool| pool.venue == "whirlpool")
        .map(|pool| pool.address.clone())
        .collect::<Vec<_>>();

    let raydium_pool_data = if raydium_pool_addresses.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &raydium_pool_addresses)
            .await
            .context("读取 Raydium CLMM pool 账户")?
    };
    let whirlpool_data = if whirlpool_addresses.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &whirlpool_addresses)
            .await
            .context("读取 Whirlpool pool 账户")?
    };

    for (address, data) in &raydium_pool_data {
        let Ok(state) = parser::raydium::parse_raydium_state(data, address) else {
            continue;
        };
        if matches!(state.venue, RaydiumVenue::Clmm) {
            context.raydium_states.insert(address.clone(), state);
        }
    }
    for (address, data) in &whirlpool_data {
        let Ok(state) = parser::whirlpool::parse_whirlpool_state(data, address) else {
            continue;
        };
        context.whirlpool_states.insert(address.clone(), state);
    }

    fetch_raydium_tick_arrays(config, &raydium_pool_data, &mut context).await?;
    fetch_whirlpool_tick_arrays(config, &mut context).await?;

    Ok(context)
}

async fn fetch_raydium_tick_arrays(
    config: &ScannerConfig,
    raydium_pool_data: &HashMap<String, Vec<u8>>,
    context: &mut GrpcQuoteContext,
) -> Result<()> {
    let mut tick_array_to_pool = HashMap::new();
    let mut tick_array_count_by_pool: HashMap<String, usize> = HashMap::new();
    let mut bitmap_extension_to_pool = HashMap::new();

    for (pool_address, state) in &context.raydium_states {
        let Some(tick_current) = state.tick_current else {
            continue;
        };
        let Some(tick_spacing) = state.tick_spacing else {
            continue;
        };

        if let Ok(addresses) = parser::raydium_clmm::derive_nearby_tick_array_addresses(
            pool_address,
            tick_current,
            tick_spacing,
            RAYDIUM_CLMM_TICK_ARRAYS_EACH_SIDE,
        ) {
            insert_scanner_raydium_tick_addresses(
                &mut tick_array_to_pool,
                &mut tick_array_count_by_pool,
                pool_address,
                addresses,
            );
        }

        if let Some(pool_data) = raydium_pool_data.get(pool_address) {
            if let Ok(mut start_indexes) =
                parser::raydium_clmm::initialized_tick_array_start_indexes_from_pool_state(
                    pool_data,
                    tick_spacing,
                )
            {
                sort_tick_array_starts_by_current(&mut start_indexes, tick_current, tick_spacing);
                if let Ok(addresses) =
                    parser::raydium_clmm::derive_tick_array_addresses_for_start_indexes(
                        pool_address,
                        &start_indexes,
                    )
                {
                    insert_scanner_raydium_tick_addresses(
                        &mut tick_array_to_pool,
                        &mut tick_array_count_by_pool,
                        pool_address,
                        addresses,
                    );
                }
            }
        }

        if let Ok(extension_address) =
            parser::raydium_clmm::derive_tickarray_bitmap_extension_address(pool_address)
        {
            bitmap_extension_to_pool.insert(extension_address, pool_address.clone());
        }
    }

    let extension_accounts = bitmap_extension_to_pool.keys().cloned().collect::<Vec<_>>();
    let extension_data = if extension_accounts.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &extension_accounts)
            .await
            .context("读取 Raydium CLMM bitmap extension 账户")?
    };
    for (extension_address, pool_address) in bitmap_extension_to_pool {
        let Some(data) = extension_data.get(&extension_address) else {
            continue;
        };
        let Some(state) = context.raydium_states.get(&pool_address) else {
            continue;
        };
        let (Some(tick_current), Some(tick_spacing)) = (state.tick_current, state.tick_spacing)
        else {
            continue;
        };
        let Ok(mut start_indexes) =
            parser::raydium_clmm::initialized_tick_array_start_indexes_from_bitmap_extension(
                data,
                tick_spacing,
            )
        else {
            continue;
        };
        sort_tick_array_starts_by_current(&mut start_indexes, tick_current, tick_spacing);
        if let Ok(addresses) = parser::raydium_clmm::derive_tick_array_addresses_for_start_indexes(
            &pool_address,
            &start_indexes,
        ) {
            insert_scanner_raydium_tick_addresses(
                &mut tick_array_to_pool,
                &mut tick_array_count_by_pool,
                &pool_address,
                addresses,
            );
        }
    }

    let tick_accounts = tick_array_to_pool.keys().cloned().collect::<Vec<_>>();
    let tick_data = if tick_accounts.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &tick_accounts)
            .await
            .context("读取 Raydium CLMM tick array 账户")?
    };

    for (tick_address, pool_address) in tick_array_to_pool {
        let Some(data) = tick_data.get(&tick_address) else {
            continue;
        };
        let Ok(tick_array) = parser::raydium_clmm::parse_tick_array(data) else {
            continue;
        };
        if tick_array.pool_id == pool_address {
            context.raydium_tick_arrays.insert(tick_address, tick_array);
        }
    }

    Ok(())
}

fn insert_scanner_raydium_tick_addresses(
    tick_array_to_pool: &mut HashMap<String, String>,
    tick_array_count_by_pool: &mut HashMap<String, usize>,
    pool_address: &str,
    addresses: Vec<(String, i32)>,
) {
    for (address, _) in addresses {
        let count = tick_array_count_by_pool
            .entry(pool_address.to_string())
            .or_default();
        if *count >= MAX_SCANNER_RAYDIUM_TICK_ARRAYS_PER_POOL {
            break;
        }
        if tick_array_to_pool.contains_key(&address) {
            continue;
        }
        tick_array_to_pool.insert(address, pool_address.to_string());
        *count += 1;
    }
}

fn sort_tick_array_starts_by_current(
    start_indexes: &mut [i32],
    tick_current: i32,
    tick_spacing: u16,
) {
    let current_start = parser::raydium_clmm::tick_array_start_index(tick_current, tick_spacing);
    start_indexes.sort_by_key(|start| {
        i64::from(*start)
            .saturating_sub(i64::from(current_start))
            .abs()
    });
}

async fn fetch_whirlpool_tick_arrays(
    config: &ScannerConfig,
    context: &mut GrpcQuoteContext,
) -> Result<()> {
    let mut tick_array_to_pool = HashMap::new();
    for (pool_address, state) in &context.whirlpool_states {
        let Ok(addresses) = parser::whirlpool::derive_nearby_tick_array_addresses(
            pool_address,
            state.tick_current_index,
            state.tick_spacing,
            WHIRLPOOL_TICK_ARRAYS_EACH_SIDE,
        ) else {
            continue;
        };
        for (address, _) in addresses {
            tick_array_to_pool.insert(address, pool_address.clone());
        }
    }

    let tick_accounts = tick_array_to_pool.keys().cloned().collect::<Vec<_>>();
    let tick_data = if tick_accounts.is_empty() {
        HashMap::new()
    } else {
        rpc::get_multiple_accounts_data(&config.rpc_url, &tick_accounts)
            .await
            .context("读取 Whirlpool tick array 账户")?
    };
    for (tick_address, pool_address) in tick_array_to_pool {
        let Some(data) = tick_data.get(&tick_address) else {
            continue;
        };
        let Ok(tick_array) = parser::whirlpool::parse_tick_array(data) else {
            continue;
        };
        if tick_array.whirlpool == pool_address {
            context
                .whirlpool_tick_arrays
                .insert(tick_address, tick_array);
        }
    }

    Ok(())
}

fn parse_scanner_meteora_bin_array(data: &[u8]) -> Option<ScannerMeteoraBinArray> {
    if data.len() < METEORA_BIN_ARRAY_HEADER_LEN + METEORA_BIN_LEN * METEORA_BINS_PER_ARRAY as usize
    {
        return None;
    }
    let index = read_i64(data, 8)?;
    let lb_pair = pubkey_at(data, 24)?;
    let mut bins = Vec::new();
    for local_index in 0..METEORA_BINS_PER_ARRAY as usize {
        let offset = METEORA_BIN_ARRAY_HEADER_LEN + local_index * METEORA_BIN_LEN;
        let amount_x = read_u64(data, offset)?;
        let amount_y = read_u64(data, offset + 8)?;
        let price = read_u128(data, offset + 16)?;
        let liquidity_supply = read_u128(data, offset + 32)?;
        let bin_id = index
            .checked_mul(METEORA_BINS_PER_ARRAY as i64)?
            .checked_add(local_index as i64)?
            .try_into()
            .ok()?;
        if amount_x > 0 || amount_y > 0 || liquidity_supply > 0 {
            bins.push(ScannerMeteoraBin {
                bin_id,
                amount_x,
                amount_y,
                price,
            });
        }
    }

    Some(ScannerMeteoraBinArray {
        lb_pair,
        index,
        bins,
    })
}

fn scanner_meteora_total_fee_rate(pool: &ParsedPool) -> u64 {
    const FEE_PRECISION: u128 = 1_000_000_000;
    const MAX_FEE_RATE: u128 = 100_000_000;
    let Some(bin_step) = pool.meteora_bin_step else {
        return 0;
    };
    let base_factor = pool.meteora_base_factor.unwrap_or(0) as u128;
    let base_fee_power_factor = pool.meteora_base_fee_power_factor.unwrap_or(0) as u32;
    let volatility_accumulator = pool.meteora_volatility_accumulator.unwrap_or(0) as u128;
    let bin_step = bin_step as u128;

    let base_fee = base_factor
        .saturating_mul(bin_step)
        .saturating_mul(10)
        .saturating_mul(10u128.saturating_pow(base_fee_power_factor));
    let variable_fee = if pool.meteora_variable_fee_control.unwrap_or(0) > 0 {
        let vfa_bin = volatility_accumulator.saturating_mul(bin_step);
        (pool.meteora_variable_fee_control.unwrap_or(0) as u128)
            .saturating_mul(vfa_bin.saturating_mul(vfa_bin))
            .saturating_add(99_999_999_999)
            / 100_000_000_000
    } else {
        0
    };

    base_fee
        .saturating_add(variable_fee)
        .min(MAX_FEE_RATE)
        .min(FEE_PRECISION) as u64
}

fn scanner_meteora_bin_price_y_per_x(pool: &ParsedPool, bin: &ScannerMeteoraBin) -> f64 {
    if bin.price > 0 {
        let scaled = (bin.price as f64) / 18_446_744_073_709_551_616.0;
        if scaled.is_finite() && scaled > 0.0 {
            return scaled;
        }
    }

    let Some(bin_step) = pool.meteora_bin_step else {
        return 0.0;
    };
    let base = 1.0 + (bin_step as f64 / 10_000.0);
    let price = base.powf(bin.bin_id as f64);
    if price.is_finite() && price > 0.0 {
        price
    } else {
        0.0
    }
}

fn scanner_meteora_fee_from_gross_input(total_fee_rate: u64, gross_amount: u64) -> u64 {
    const FEE_PRECISION: u128 = 1_000_000_000;
    if total_fee_rate == 0 || gross_amount == 0 {
        return 0;
    }
    (((gross_amount as u128) * (total_fee_rate as u128)).saturating_add(FEE_PRECISION - 1)
        / FEE_PRECISION)
        .min(u64::MAX as u128) as u64
}

fn scanner_meteora_fee_for_net_input(total_fee_rate: u64, net_input: u64) -> u64 {
    const FEE_PRECISION: u128 = 1_000_000_000;
    if total_fee_rate == 0 || net_input == 0 {
        return 0;
    }
    let denominator = FEE_PRECISION.saturating_sub(total_fee_rate as u128).max(1);
    (((net_input as u128) * (total_fee_rate as u128)).saturating_add(denominator - 1) / denominator)
        .min(u64::MAX as u128) as u64
}

fn scanner_conservative_meteora_amount(raw_amount: f64) -> f64 {
    raw_amount * (1.0 - METEORA_LOCAL_QUOTE_SAFETY_BPS / 10_000.0)
}

fn scanner_quote_meteora_exact_in(
    pool: &ParsedPool,
    bin_arrays: &[ScannerMeteoraBinArray],
    amount_in: u64,
    x_to_y: bool,
) -> Option<(u64, u64, usize)> {
    let active_id = pool.meteora_active_id?;
    if amount_in == 0 {
        return None;
    }

    let total_fee_rate = scanner_meteora_total_fee_rate(pool);
    let mut bins: Vec<(i64, &ScannerMeteoraBin)> = bin_arrays
        .iter()
        .flat_map(|array| array.bins.iter().map(move |bin| (array.index, bin)))
        .collect();
    if bins.is_empty() {
        return None;
    }

    if x_to_y {
        bins.sort_by(|(_, left), (_, right)| right.bin_id.cmp(&left.bin_id));
    } else {
        bins.sort_by(|(_, left), (_, right)| left.bin_id.cmp(&right.bin_id));
    }

    let mut remaining = amount_in;
    let mut amount_out = 0.0_f64;
    let mut amount_out_without_fee = 0.0_f64;
    let mut touched_arrays = HashSet::new();

    for (array_index, bin) in bins {
        if x_to_y && bin.bin_id > active_id {
            continue;
        }
        if !x_to_y && bin.bin_id < active_id {
            continue;
        }

        let price_y_per_x = scanner_meteora_bin_price_y_per_x(pool, bin);
        if price_y_per_x <= 0.0 {
            continue;
        }

        if x_to_y {
            let available_out = bin.amount_y as f64;
            if available_out <= 0.0 {
                continue;
            }
            let max_net_input_for_bin = ceil_u64(available_out / price_y_per_x);
            let max_fee_for_bin =
                scanner_meteora_fee_for_net_input(total_fee_rate, max_net_input_for_bin);
            let max_gross_input_for_bin = max_net_input_for_bin.saturating_add(max_fee_for_bin);
            let used_gross_input = remaining.min(max_gross_input_for_bin);
            let fee_amount = scanner_meteora_fee_from_gross_input(total_fee_rate, used_gross_input);
            let used_net_input = used_gross_input.saturating_sub(fee_amount);
            amount_out += (used_net_input as f64) * price_y_per_x;
            amount_out_without_fee += (used_gross_input as f64) * price_y_per_x;
            touched_arrays.insert(array_index);
            remaining = remaining.saturating_sub(used_gross_input);
        } else {
            let available_out = bin.amount_x as f64;
            if available_out <= 0.0 {
                continue;
            }
            let max_net_input_for_bin = ceil_u64(available_out * price_y_per_x);
            let max_fee_for_bin =
                scanner_meteora_fee_for_net_input(total_fee_rate, max_net_input_for_bin);
            let max_gross_input_for_bin = max_net_input_for_bin.saturating_add(max_fee_for_bin);
            let used_gross_input = remaining.min(max_gross_input_for_bin);
            let fee_amount = scanner_meteora_fee_from_gross_input(total_fee_rate, used_gross_input);
            let used_net_input = used_gross_input.saturating_sub(fee_amount);
            amount_out += (used_net_input as f64) / price_y_per_x;
            amount_out_without_fee += (used_gross_input as f64) / price_y_per_x;
            touched_arrays.insert(array_index);
            remaining = remaining.saturating_sub(used_gross_input);
        }

        if remaining == 0 {
            break;
        }
    }

    if remaining > 0 || !amount_out.is_finite() || amount_out <= 0.0 {
        return None;
    }

    let conservative_amount_out = scanner_conservative_meteora_amount(amount_out);
    let conservative_amount_out_without_fee =
        scanner_conservative_meteora_amount(amount_out_without_fee);
    if conservative_amount_out <= 0.0 || !conservative_amount_out.is_finite() {
        return None;
    }

    Some((
        floor_u64(conservative_amount_out).max(1),
        floor_u64(conservative_amount_out_without_fee)
            .max(floor_u64(conservative_amount_out).max(1)),
        touched_arrays.len(),
    ))
}

fn scanner_trade_size_lamports(trade_size_sol: f64) -> Option<u64> {
    if !trade_size_sol.is_finite() || trade_size_sol <= 0.0 {
        return None;
    }
    Some(ceil_u64(trade_size_sol * 1_000_000_000.0).max(1))
}

fn scanner_token_account_amount(vault_data: &HashMap<String, Vec<u8>>, vault: &str) -> Option<u64> {
    vault_data
        .get(vault)
        .and_then(|data| parse_token_account_amount(data))
}

fn scanner_quote_constant_product_out(
    input_reserve: u64,
    output_reserve: u64,
    amount_in: u64,
    fee_bps: f64,
) -> Option<u64> {
    if input_reserve == 0 || output_reserve == 0 || amount_in == 0 {
        return None;
    }
    if !fee_bps.is_finite() || fee_bps < 0.0 {
        return None;
    }

    let fee_bps = fee_bps.min(10_000.0);
    let net_in = ((amount_in as f64) * (1.0 - fee_bps / 10_000.0)).floor();
    if net_in <= 0.0 || !net_in.is_finite() {
        return None;
    }
    let net_in = net_in.min(u64::MAX as f64) as u64;
    if net_in == 0 {
        return None;
    }

    let numerator = (net_in as u128).saturating_mul(output_reserve as u128);
    let denominator = (input_reserve as u128).saturating_add(net_in as u128);
    if denominator == 0 {
        return None;
    }
    Some((numerator / denominator).min(u64::MAX as u128) as u64)
}

fn scanner_meteora_x_to_y(pool: &ParsedPool, input_mint: &str, output_mint: &str) -> Option<bool> {
    let quote_is_x = pool.meteora_quote_is_x?;
    let token_mint = pool.token_mint.as_str();
    let quote_mint = pool.quote_mint.as_str();

    match (input_mint, output_mint) {
        (input, output) if input == quote_mint && output == token_mint => Some(quote_is_x),
        (input, output) if input == token_mint && output == quote_mint => Some(!quote_is_x),
        _ => None,
    }
}

fn scanner_quote_exact_in_for_pool(
    pool: &ParsedPool,
    input_mint: &str,
    output_mint: &str,
    amount_in: u64,
    vault_data: &HashMap<String, Vec<u8>>,
    meteora_bin_arrays: &HashMap<String, Vec<ScannerMeteoraBinArray>>,
    quote_context: &GrpcQuoteContext,
) -> Option<(u64, usize)> {
    if amount_in == 0 {
        return None;
    }

    match pool.venue {
        "pumpswap" => {
            let (input_reserve, output_reserve) =
                if input_mint == pool.quote_mint && output_mint == pool.token_mint {
                    (
                        scanner_token_account_amount(vault_data, &pool.quote_vault)?,
                        scanner_token_account_amount(vault_data, &pool.token_vault)?,
                    )
                } else if input_mint == pool.token_mint && output_mint == pool.quote_mint {
                    (
                        scanner_token_account_amount(vault_data, &pool.token_vault)?,
                        scanner_token_account_amount(vault_data, &pool.quote_vault)?,
                    )
                } else {
                    return None;
                };
            scanner_quote_constant_product_out(
                input_reserve,
                output_reserve,
                amount_in,
                PUMPSWAP_DISCOVERY_FEE_BPS,
            )
            .map(|amount_out| (amount_out, 0))
        }
        "meteora" => {
            let bin_arrays = meteora_bin_arrays.get(&pool.address)?;
            let x_to_y = scanner_meteora_x_to_y(pool, input_mint, output_mint)?;
            let (amount_out, _, touched_bin_arrays) =
                scanner_quote_meteora_exact_in(pool, bin_arrays, amount_in, x_to_y)?;
            Some((amount_out, touched_bin_arrays))
        }
        "raydium" => {
            let state = quote_context.raydium_states.get(&pool.address)?;
            if !raydium_quote_direction_matches(state, input_mint, output_mint) {
                return None;
            }
            let tick_arrays = quote_context
                .raydium_tick_arrays
                .values()
                .filter(|array| array.pool_id == state.pool_address)
                .collect::<Vec<_>>();
            if tick_arrays.is_empty() {
                return None;
            }
            let quote = strategy::clmm::quote_exact_input(
                state,
                &tick_arrays,
                input_mint,
                amount_in as f64,
            )
            .ok()?;
            let amount_out = floor_u64(quote.amount_out).max(1);
            Some((amount_out, 0))
        }
        "whirlpool" => {
            let state = quote_context.whirlpool_states.get(&pool.address)?;
            if !whirlpool_quote_direction_matches(state, input_mint, output_mint) {
                return None;
            }
            let amount_out = scanner_quote_whirlpool_exact_in(
                state,
                &quote_context.whirlpool_tick_arrays,
                input_mint,
                amount_in,
            )?;
            Some((amount_out, 0))
        }
        _ => None,
    }
}

fn raydium_quote_direction_matches(
    state: &RaydiumState,
    input_mint: &str,
    output_mint: &str,
) -> bool {
    (input_mint == state.base_mint && output_mint == state.quote_mint)
        || (input_mint == state.quote_mint && output_mint == state.base_mint)
}

fn whirlpool_quote_direction_matches(
    state: &WhirlpoolState,
    input_mint: &str,
    output_mint: &str,
) -> bool {
    (input_mint == state.token_mint_a && output_mint == state.token_mint_b)
        || (input_mint == state.token_mint_b && output_mint == state.token_mint_a)
}

fn scanner_whirlpool_tick_array_facade(
    start_tick_index: i32,
) -> orca_whirlpools_core::TickArrayFacade {
    orca_whirlpools_core::TickArrayFacade {
        start_tick_index,
        ticks: [orca_whirlpools_core::TickFacade::default(); orca_whirlpools_core::TICK_ARRAY_SIZE],
    }
}

fn scanner_whirlpool_tick_arrays_for_quote(
    whirlpool: &WhirlpoolState,
    whirlpool_tick_arrays: &HashMap<String, WhirlpoolTickArrayState>,
) -> Option<orca_whirlpools_core::TickArrays> {
    let matching = whirlpool_tick_arrays
        .values()
        .filter(|array| array.whirlpool == whirlpool.pool_address)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return None;
    }

    let current_start_tick_index = orca_whirlpools_core::get_tick_array_start_tick_index(
        whirlpool.tick_current_index,
        whirlpool.tick_spacing,
    );
    let offset = whirlpool.tick_spacing as i32 * orca_whirlpools_core::TICK_ARRAY_SIZE as i32;
    let ordered_start_indexes = [
        current_start_tick_index,
        current_start_tick_index + offset,
        current_start_tick_index + offset * 2,
        current_start_tick_index - offset,
        current_start_tick_index - offset * 2,
    ];
    let by_start_index = matching
        .into_iter()
        .map(|array| (array.start_tick_index, array.tick_array))
        .collect::<HashMap<_, _>>();
    let facades = ordered_start_indexes.map(|start_tick_index| {
        by_start_index
            .get(&start_tick_index)
            .copied()
            .unwrap_or_else(|| scanner_whirlpool_tick_array_facade(start_tick_index))
    });
    Some(facades.into())
}

fn scanner_whirlpool_to_facade(state: &WhirlpoolState) -> orca_whirlpools_core::WhirlpoolFacade {
    orca_whirlpools_core::WhirlpoolFacade {
        fee_tier_index_seed: [0, 0],
        tick_spacing: state.tick_spacing,
        fee_rate: state.fee_rate,
        protocol_fee_rate: state.protocol_fee_rate,
        liquidity: state.liquidity,
        sqrt_price: state.sqrt_price,
        tick_current_index: state.tick_current_index,
        fee_growth_global_a: 0,
        fee_growth_global_b: 0,
        reward_last_updated_timestamp: 0,
        reward_infos: [
            orca_whirlpools_core::WhirlpoolRewardInfoFacade::default(),
            orca_whirlpools_core::WhirlpoolRewardInfoFacade::default(),
            orca_whirlpools_core::WhirlpoolRewardInfoFacade::default(),
        ],
    }
}

fn scanner_quote_whirlpool_exact_in(
    whirlpool: &WhirlpoolState,
    whirlpool_tick_arrays: &HashMap<String, WhirlpoolTickArrayState>,
    input_mint: &str,
    amount_in: u64,
) -> Option<u64> {
    if amount_in == 0 {
        return None;
    }

    let specified_token_a = if input_mint == whirlpool.token_mint_a {
        true
    } else if input_mint == whirlpool.token_mint_b {
        false
    } else {
        return None;
    };
    let tick_arrays = scanner_whirlpool_tick_arrays_for_quote(whirlpool, whirlpool_tick_arrays)?;
    let quote = orca_whirlpools_core::swap_quote_by_input_token(
        amount_in,
        specified_token_a,
        0,
        scanner_whirlpool_to_facade(whirlpool),
        None,
        tick_arrays,
        unix_now(),
        None,
        None,
    )
    .ok()?;
    (quote.token_est_out > 0).then_some(quote.token_est_out)
}

fn scanner_apply_slippage(amount: u64, slippage_bps: f64) -> Option<u64> {
    if amount == 0 {
        return None;
    }
    if !slippage_bps.is_finite() || slippage_bps < 0.0 {
        return None;
    }
    let amount = ((amount as f64) * (1.0 - slippage_bps.min(10_000.0) / 10_000.0)).floor();
    if amount <= 0.0 || !amount.is_finite() {
        return None;
    }
    Some(amount.min(u64::MAX as f64) as u64)
}

fn scanner_trade_depth_bps(
    config: &ScannerConfig,
    trade_size_sol: f64,
    pair_liquidity_usdc: f64,
) -> f64 {
    if pair_liquidity_usdc <= 0.0
        || !pair_liquidity_usdc.is_finite()
        || trade_size_sol <= 0.0
        || !trade_size_sol.is_finite()
        || config.sol_usdc_price <= 0.0
        || !config.sol_usdc_price.is_finite()
    {
        return f64::INFINITY;
    }
    ((trade_size_sol * config.sol_usdc_price) / pair_liquidity_usdc) * 10_000.0
}

fn scanner_dynamic_buy_slippage_bps(
    config: &ScannerConfig,
    buy_quote_liquidity_usdc: f64,
    trade_depth_bps: f64,
) -> f64 {
    if buy_quote_liquidity_usdc < config.small_pool_liquidity_usdc
        || trade_depth_bps > config.max_trade_depth_bps * 0.75
    {
        config
            .buy_slippage_bps_small_pool
            .max(config.buy_slippage_bps)
    } else {
        config.buy_slippage_bps
    }
}

fn scanner_dynamic_sell_slippage_bps(
    config: &ScannerConfig,
    sell_quote_liquidity_usdc: f64,
    trade_depth_bps: f64,
    touched_bin_arrays: usize,
) -> f64 {
    if sell_quote_liquidity_usdc < config.small_pool_liquidity_usdc
        || trade_depth_bps > config.max_trade_depth_bps * 0.75
        || touched_bin_arrays > 2
    {
        config
            .sell_slippage_bps_small_pool
            .max(config.sell_slippage_bps)
    } else {
        config.sell_slippage_bps
    }
}

fn scanner_program_pair_edge(
    config: &ScannerConfig,
    buy_pool: &ParsedPool,
    sell_pool: &ParsedPool,
    vault_data: &HashMap<String, Vec<u8>>,
    meteora_bin_arrays: &HashMap<String, Vec<ScannerMeteoraBinArray>>,
    quote_context: &GrpcQuoteContext,
    trade_size_sol: f64,
    buy_index: usize,
    sell_index: usize,
    min_net_profit_pct: f64,
) -> Option<ExecutableEdge> {
    if buy_pool.token_mint != sell_pool.token_mint
        || buy_pool.quote_mint != config.sol_mint
        || sell_pool.quote_mint != config.sol_mint
    {
        return None;
    }

    let quote_in = scanner_trade_size_lamports(trade_size_sol)?;
    if quote_in == 0 {
        return None;
    }
    let pair_liquidity_usdc = buy_pool
        .quote_liquidity_usdc
        .min(sell_pool.quote_liquidity_usdc);
    let trade_depth_bps = scanner_trade_depth_bps(config, trade_size_sol, pair_liquidity_usdc);

    let route = format!(
        "{}->{}",
        venue_display_name(buy_pool.venue),
        venue_display_name(sell_pool.venue)
    );
    let (token_out_raw, _) = scanner_quote_exact_in_for_pool(
        buy_pool,
        &config.sol_mint,
        &buy_pool.token_mint,
        quote_in,
        vault_data,
        meteora_bin_arrays,
        quote_context,
    )?;
    let (quote_out_raw, touched_bin_arrays) = scanner_quote_exact_in_for_pool(
        sell_pool,
        &sell_pool.token_mint,
        &config.sol_mint,
        token_out_raw,
        vault_data,
        meteora_bin_arrays,
        quote_context,
    )?;

    let gross_profit_lamports = i128::from(quote_out_raw).saturating_sub(i128::from(quote_in));
    let net_profit_lamports =
        gross_profit_lamports.saturating_sub(i128::from(config.fixed_cost_lamports));
    let net_profit_pct = (net_profit_lamports as f64) / (quote_in as f64) * 100.0;
    let buy_slippage_bps =
        scanner_dynamic_buy_slippage_bps(config, buy_pool.quote_liquidity_usdc, trade_depth_bps);
    let conservative_token_out =
        scanner_apply_slippage(token_out_raw, buy_slippage_bps).unwrap_or(token_out_raw);
    let sell_slippage_bps = scanner_dynamic_sell_slippage_bps(
        config,
        sell_pool.quote_liquidity_usdc,
        trade_depth_bps,
        touched_bin_arrays,
    );
    let conservative_quote_out =
        scanner_apply_slippage(quote_out_raw, sell_slippage_bps).unwrap_or(quote_out_raw);
    let conservative_gross_profit_lamports =
        i128::from(conservative_quote_out).saturating_sub(i128::from(quote_in));
    let conservative_net_profit_lamports =
        conservative_gross_profit_lamports.saturating_sub(i128::from(config.fixed_cost_lamports));
    let conservative_net_profit_pct =
        (conservative_net_profit_lamports as f64) / (quote_in as f64) * 100.0;
    if config.debug_quotes && net_profit_pct.is_finite() {
        tracing::info!(
            "quote详情：mint={} route={} sol_in={} token_raw={} token_eff={} sol_raw={} sol_eff={} gross_raw={} net_raw={} gross_eff={} net_eff={} net_pct={:.4}% eff_net_pct={:.4}%",
            buy_pool.token_mint,
            route,
            quote_in,
            token_out_raw,
            conservative_token_out,
            quote_out_raw,
            conservative_quote_out,
            gross_profit_lamports,
            net_profit_lamports,
            conservative_gross_profit_lamports,
            conservative_net_profit_lamports,
            net_profit_pct,
            conservative_net_profit_pct
        );
    }
    if !net_profit_pct.is_finite() || net_profit_pct <= min_net_profit_pct {
        return None;
    }

    Some(ExecutableEdge {
        route,
        net_profit_pct,
        trade_size_sol,
        buy_index,
        sell_index,
        quote_in_lamports: quote_in,
        token_out_raw,
        quote_out_lamports: quote_out_raw,
        gross_profit_lamports,
        net_profit_lamports,
    })
}

fn scanner_best_executable_edge_for_token(
    config: &ScannerConfig,
    pools: &[ParsedPool],
    vault_data: &HashMap<String, Vec<u8>>,
    meteora_bin_arrays: &HashMap<String, Vec<ScannerMeteoraBinArray>>,
    quote_context: &GrpcQuoteContext,
    min_net_profit_pct: f64,
) -> Option<(ExecutableEdge, f64)> {
    let mut best: Option<(ExecutableEdge, f64)> = None;

    for (buy_index, buy_pool) in pools.iter().enumerate() {
        if !is_supported_routeable_venue(buy_pool.venue) || buy_pool.quote_mint != config.sol_mint {
            continue;
        }
        for (sell_index, sell_pool) in pools.iter().enumerate() {
            if buy_index == sell_index
                || !is_supported_routeable_venue(sell_pool.venue)
                || sell_pool.quote_mint != config.sol_mint
                || buy_pool.token_mint != sell_pool.token_mint
            {
                continue;
            }

            let pair_liquidity_usdc = buy_pool
                .quote_liquidity_usdc
                .min(sell_pool.quote_liquidity_usdc);
            if pair_liquidity_usdc < config.min_pair_liquidity_usdc {
                continue;
            }

            for trade_size_sol in &config.executable_trade_sizes_sol {
                let trade_size_sol = *trade_size_sol;
                let Some(edge) = scanner_program_pair_edge(
                    config,
                    buy_pool,
                    sell_pool,
                    vault_data,
                    meteora_bin_arrays,
                    quote_context,
                    trade_size_sol,
                    buy_index,
                    sell_index,
                    min_net_profit_pct,
                ) else {
                    continue;
                };

                let should_replace = best
                    .as_ref()
                    .map(|(current, current_liquidity)| {
                        edge.net_profit_pct > current.net_profit_pct
                            || ((edge.net_profit_pct - current.net_profit_pct).abs() < f64::EPSILON
                                && pair_liquidity_usdc > *current_liquidity)
                    })
                    .unwrap_or(true);
                if should_replace {
                    best = Some((edge, pair_liquidity_usdc));
                }
            }
        }
    }

    best
}

fn scanner_best_spot_edge_for_token(
    config: &ScannerConfig,
    pools: &[ParsedPool],
    vault_data: &HashMap<String, Vec<u8>>,
    min_edge_pct: f64,
) -> Option<SpotEdge> {
    let mut prices = Vec::new();
    for (index, pool) in pools.iter().enumerate() {
        if !is_supported_routeable_venue(pool.venue) {
            continue;
        }
        if let Some(price) = pool_price_ratio(pool, vault_data, &config.sol_mint) {
            if price.is_finite() && price > 0.0 {
                prices.push((index, price));
            }
        }
    }

    let mut best: Option<SpotEdge> = None;
    for (buy_index, buy_price) in prices.iter().copied() {
        for (sell_index, sell_price) in prices.iter().copied() {
            if buy_index == sell_index {
                continue;
            }
            if sell_price <= buy_price || buy_price <= 0.0 {
                continue;
            }
            let edge_pct = ((sell_price - buy_price) / buy_price) * 100.0;
            if !edge_pct.is_finite() || edge_pct < min_edge_pct {
                continue;
            }
            let pair_liquidity_usdc = pools[buy_index]
                .quote_liquidity_usdc
                .min(pools[sell_index].quote_liquidity_usdc);
            if pair_liquidity_usdc < config.min_pair_liquidity_usdc {
                continue;
            }
            let candidate = SpotEdge {
                edge_pct,
                buy_index,
                sell_index,
                pair_liquidity_usdc,
            };
            let replace = best
                .as_ref()
                .map(|current| {
                    candidate.edge_pct > current.edge_pct
                        || ((candidate.edge_pct - current.edge_pct).abs() < f64::EPSILON
                            && candidate.pair_liquidity_usdc > current.pair_liquidity_usdc)
                })
                .unwrap_or(true);
            if replace {
                best = Some(candidate);
            }
        }
    }

    best
}

fn ceil_u64(value: f64) -> u64 {
    if value <= 0.0 || !value.is_finite() {
        0
    } else {
        value.ceil().min(u64::MAX as f64) as u64
    }
}

fn floor_u64(value: f64) -> u64 {
    if value <= 0.0 || !value.is_finite() {
        0
    } else {
        value.floor().min(u64::MAX as f64) as u64
    }
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
            token_has_routeable_pair(&pools).then(|| {
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
        let mut raydium_count = 0usize;
        let mut whirlpool_count = 0usize;
        for pool in pools {
            let keep = match pool.venue {
                "pumpswap" if pumpswap_count < config.max_pumpswap_per_token => {
                    pumpswap_count += 1;
                    true
                }
                "meteora" if meteora_count < config.max_meteora_per_token => {
                    meteora_count += 1;
                    true
                }
                "raydium" if raydium_count < config.max_meteora_per_token => {
                    raydium_count += 1;
                    true
                }
                "whirlpool" if whirlpool_count < config.max_meteora_per_token => {
                    whirlpool_count += 1;
                    true
                }
                _ => false,
            };
            if !keep {
                continue;
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
                    recent_trades_5m: pool.hits / 3,
                    recent_trades_15m: pool.hits,
                    recent_volume_15m_usd: (pool.hits as f64) * 1_000.0,
                },
            );
        }
    }

    selected
}

#[derive(Debug)]
struct GrpcRouteableTokenGroup {
    token_mint: String,
    score: f64,
    pair_liquidity_usdc: f64,
    total_liquidity_usdc: f64,
    recent_trades_5m: u64,
    recent_trades_15m: u64,
    latest_slot: u64,
    pools: Vec<ParsedPool>,
}

#[derive(Debug, Default)]
struct GrpcEdgeDiagnostics {
    paired_tokens: usize,
    priced_tokens: usize,
    spread_tokens: usize,
    best_spot_edge: Option<GrpcBestSpotEdge>,
}

#[derive(Debug)]
struct GrpcBestSpotEdge {
    token_mint: String,
    route: String,
    edge_pct: f64,
    pair_liquidity_usdc: f64,
}

#[derive(Debug, Clone)]
struct ScannerMeteoraBinArray {
    lb_pair: String,
    index: i64,
    bins: Vec<ScannerMeteoraBin>,
}

#[derive(Debug, Clone)]
struct ScannerMeteoraBin {
    bin_id: i32,
    amount_x: u64,
    amount_y: u64,
    price: u128,
}

#[derive(Debug, Clone)]
struct ExecutableEdge {
    route: String,
    net_profit_pct: f64,
    trade_size_sol: f64,
    buy_index: usize,
    sell_index: usize,
    quote_in_lamports: u64,
    token_out_raw: u64,
    quote_out_lamports: u64,
    gross_profit_lamports: i128,
    net_profit_lamports: i128,
}

#[derive(Debug, Clone)]
struct SpotEdge {
    edge_pct: f64,
    buy_index: usize,
    sell_index: usize,
    pair_liquidity_usdc: f64,
}

#[derive(Debug, Default)]
struct GrpcQuoteContext {
    raydium_states: HashMap<String, RaydiumState>,
    raydium_tick_arrays: HashMap<String, ClmmTickArrayState>,
    whirlpool_states: HashMap<String, WhirlpoolState>,
    whirlpool_tick_arrays: HashMap<String, WhirlpoolTickArrayState>,
}

fn is_supported_routeable_venue(venue: &str) -> bool {
    matches!(venue, "pumpswap" | "meteora" | "raydium" | "whirlpool")
}

fn venue_display_name(venue: &str) -> &'static str {
    match venue {
        "pumpswap" => "PumpSwap",
        "meteora" => "Meteora",
        "raydium" => "RaydiumCLMM",
        "whirlpool" => "Whirlpool",
        _ => "Unknown",
    }
}

fn token_has_routeable_pair(pools: &[ParsedPool]) -> bool {
    pools
        .iter()
        .filter(|pool| is_supported_routeable_venue(pool.venue))
        .count()
        >= 2
}

fn select_routeable_grpc_pools(
    config: &ScannerConfig,
    pools: HashMap<String, ParsedPool>,
    vault_data: &HashMap<String, Vec<u8>>,
    now_unix: u64,
) -> HashMap<String, FreshPool> {
    let mut by_token: HashMap<String, Vec<ParsedPool>> = HashMap::new();
    for mut pool in pools.into_values() {
        prune_pool_activity(&mut pool, now_unix);
        if !grpc_pool_passes_snapshot_quality(config, &pool, now_unix) {
            continue;
        }
        by_token
            .entry(pool.token_mint.clone())
            .or_default()
            .push(pool);
    }

    let mut groups = by_token
        .into_iter()
        .filter_map(|(token_mint, pools)| {
            build_grpc_token_group(config, token_mint, pools, vault_data, now_unix)
        })
        .collect::<Vec<_>>();

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .pair_liquidity_usdc
                    .partial_cmp(&left.pair_liquidity_usdc)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                right
                    .total_liquidity_usdc
                    .partial_cmp(&left.total_liquidity_usdc)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.recent_trades_15m.cmp(&left.recent_trades_15m))
            .then_with(|| right.recent_trades_5m.cmp(&left.recent_trades_5m))
            .then_with(|| right.latest_slot.cmp(&left.latest_slot))
            .then_with(|| left.token_mint.cmp(&right.token_mint))
    });

    let mut selected = HashMap::new();
    for group in groups.into_iter().take(config.max_tokens) {
        let mut pools = group.pools;
        pools.sort_by(|left, right| {
            right
                .quote_liquidity_usdc
                .partial_cmp(&left.quote_liquidity_usdc)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    let (left_recent_5m, left_recent_15m) =
                        pool_recent_activity_counts(left, now_unix);
                    let (right_recent_5m, right_recent_15m) =
                        pool_recent_activity_counts(right, now_unix);
                    right_recent_15m
                        .cmp(&left_recent_15m)
                        .then_with(|| right_recent_5m.cmp(&left_recent_5m))
                })
                .then_with(|| right.latest_slot.cmp(&left.latest_slot))
        });

        let mut pumpswap_count = 0usize;
        let mut meteora_count = 0usize;
        let mut raydium_count = 0usize;
        let mut whirlpool_count = 0usize;
        for pool in pools {
            let keep = match pool.venue {
                "pumpswap" if pumpswap_count < config.max_pumpswap_per_token => {
                    pumpswap_count += 1;
                    true
                }
                "meteora" if meteora_count < config.max_meteora_per_token => {
                    meteora_count += 1;
                    true
                }
                "raydium" if raydium_count < config.max_meteora_per_token => {
                    raydium_count += 1;
                    true
                }
                "whirlpool" if whirlpool_count < config.max_meteora_per_token => {
                    whirlpool_count += 1;
                    true
                }
                _ => false,
            };
            if !keep {
                continue;
            }

            let (recent_trades_5m, recent_trades_15m) =
                pool_recent_activity_counts(&pool, now_unix);
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
                    recent_trades_5m,
                    recent_trades_15m,
                    recent_volume_15m_usd: (recent_trades_15m as f64) * 1_000.0,
                },
            );
        }
    }

    selected
}

fn grpc_edge_diagnostics(
    config: &ScannerConfig,
    pools: &HashMap<String, ParsedPool>,
    vault_data: &HashMap<String, Vec<u8>>,
    now_unix: u64,
) -> GrpcEdgeDiagnostics {
    let mut by_token: HashMap<String, Vec<ParsedPool>> = HashMap::new();
    for pool in pools.values() {
        if !grpc_pool_passes_snapshot_quality(config, pool, now_unix) {
            continue;
        }
        by_token
            .entry(pool.token_mint.clone())
            .or_default()
            .push(pool.clone());
    }

    let mut diagnostics = GrpcEdgeDiagnostics::default();
    for (token_mint, token_pools) in by_token {
        if !token_has_routeable_pair(&token_pools) {
            continue;
        }
        diagnostics.paired_tokens += 1;

        if let Some(spot_edge) =
            scanner_best_spot_edge_for_token(config, &token_pools, vault_data, 0.0)
        {
            diagnostics.priced_tokens += 1;
            diagnostics.spread_tokens += 1;
            let replace = diagnostics
                .best_spot_edge
                .as_ref()
                .map(|current| {
                    spot_edge.edge_pct > current.edge_pct
                        || ((spot_edge.edge_pct - current.edge_pct).abs() < f64::EPSILON
                            && spot_edge.pair_liquidity_usdc > current.pair_liquidity_usdc)
                })
                .unwrap_or(true);
            if replace {
                diagnostics.best_spot_edge = Some(GrpcBestSpotEdge {
                    token_mint: token_mint.clone(),
                    route: format!(
                        "{}->{}",
                        venue_display_name(token_pools[spot_edge.buy_index].venue),
                        venue_display_name(token_pools[spot_edge.sell_index].venue)
                    ),
                    edge_pct: spot_edge.edge_pct,
                    pair_liquidity_usdc: spot_edge.pair_liquidity_usdc,
                });
            }
        }
    }

    diagnostics
}

fn min_pool_recent_trades_15m(config: &ScannerConfig) -> u64 {
    config.min_token_recent_trades_15m.max(1)
}

fn min_pool_recent_volume_15m_usd(config: &ScannerConfig) -> f64 {
    config.api_min_m15_volume_usd.max(0.0)
}

fn grpc_pool_passes_snapshot_quality(
    config: &ScannerConfig,
    pool: &ParsedPool,
    _now_unix: u64,
) -> bool {
    if pool.quote_mint != config.sol_mint {
        return false;
    }
    if pool.quote_liquidity_usdc < config.min_quote_liquidity_usdc {
        return false;
    }
    true
}

fn build_grpc_token_group(
    config: &ScannerConfig,
    token_mint: String,
    pools: Vec<ParsedPool>,
    vault_data: &HashMap<String, Vec<u8>>,
    now_unix: u64,
) -> Option<GrpcRouteableTokenGroup> {
    if !token_has_routeable_pair(&pools) {
        return None;
    }

    let min_recent_trades = config.min_token_recent_trades_15m.max(1);
    let mut total_liquidity_usdc = 0.0;
    let mut recent_trades_5m = 0u64;
    let mut recent_trades_15m = 0u64;
    let mut latest_slot = 0u64;

    for pool in &pools {
        latest_slot = latest_slot.max(pool.latest_slot);
        total_liquidity_usdc += pool.quote_liquidity_usdc;
        let (pool_recent_5m, pool_recent_15m) = pool_recent_activity_counts(pool, now_unix);
        recent_trades_5m = recent_trades_5m.saturating_add(pool_recent_5m);
        recent_trades_15m = recent_trades_15m.saturating_add(pool_recent_15m);
    }

    if let Some(spot_edge) = scanner_best_spot_edge_for_token(config, &pools, vault_data, 0.0) {
        let pair_liquidity_usdc = spot_edge.pair_liquidity_usdc;
        let score = grpc_token_group_score(
            pair_liquidity_usdc,
            total_liquidity_usdc,
            recent_trades_5m,
            recent_trades_15m,
            min_recent_trades,
        ) / 1_000.0
            + spot_edge.edge_pct;

        let mut selected_pools = vec![pools[spot_edge.buy_index].clone()];
        if spot_edge.sell_index != spot_edge.buy_index {
            selected_pools.push(pools[spot_edge.sell_index].clone());
        }

        return Some(GrpcRouteableTokenGroup {
            token_mint,
            score,
            pair_liquidity_usdc,
            total_liquidity_usdc,
            recent_trades_5m,
            recent_trades_15m,
            latest_slot,
            pools: selected_pools,
        });
    }

    None
}

#[allow(dead_code)]
fn update_low_price_pick(
    pick: &mut Option<(f64, f64, usize)>,
    price_ratio: f64,
    liquidity_usdc: f64,
    index: usize,
) {
    let should_replace = pick
        .map(|(current_price, current_liquidity, _)| {
            price_ratio < current_price
                || ((price_ratio - current_price).abs() < f64::EPSILON
                    && liquidity_usdc > current_liquidity)
        })
        .unwrap_or(true);
    if should_replace {
        *pick = Some((price_ratio, liquidity_usdc, index));
    }
}

#[allow(dead_code)]
fn update_high_price_pick(
    pick: &mut Option<(f64, f64, usize)>,
    price_ratio: f64,
    liquidity_usdc: f64,
    index: usize,
) {
    let should_replace = pick
        .map(|(current_price, current_liquidity, _)| {
            price_ratio > current_price
                || ((price_ratio - current_price).abs() < f64::EPSILON
                    && liquidity_usdc > current_liquidity)
        })
        .unwrap_or(true);
    if should_replace {
        *pick = Some((price_ratio, liquidity_usdc, index));
    }
}

#[allow(dead_code)]
fn grpc_edge_candidate(
    buy: Option<(f64, f64, usize)>,
    sell: Option<(f64, f64, usize)>,
    pools: &[ParsedPool],
) -> Option<(f64, f64, usize, usize)> {
    let (buy_price, _, buy_index) = buy?;
    let (sell_price, _, sell_index) = sell?;
    if buy_index == sell_index || buy_price <= 0.0 || sell_price <= buy_price {
        return None;
    }

    let edge_pct = ((sell_price - buy_price) / buy_price) * 100.0;
    if !edge_pct.is_finite() || edge_pct <= 0.0 {
        return None;
    }

    let pair_liquidity_usdc = pools[buy_index]
        .quote_liquidity_usdc
        .min(pools[sell_index].quote_liquidity_usdc);
    Some((edge_pct, pair_liquidity_usdc, buy_index, sell_index))
}

fn grpc_token_group_score(
    pair_liquidity_usdc: f64,
    total_liquidity_usdc: f64,
    recent_trades_5m: u64,
    recent_trades_15m: u64,
    min_recent_trades: u64,
) -> f64 {
    let liquidity_score = match pair_liquidity_usdc {
        value if value >= 250_000.0 => 60.0,
        value if value >= 100_000.0 => 50.0,
        value if value >= 50_000.0 => 40.0,
        value if value >= 20_000.0 => 30.0,
        value if value >= 10_000.0 => 20.0,
        value if value >= 5_000.0 => 10.0,
        _ => 0.0,
    } + match total_liquidity_usdc {
        value if value >= 500_000.0 => 20.0,
        value if value >= 200_000.0 => 15.0,
        value if value >= 100_000.0 => 10.0,
        value if value >= 50_000.0 => 6.0,
        _ => 0.0,
    };
    let activity_score = match recent_trades_15m {
        value if value >= 100 => 25.0,
        value if value >= 50 => 20.0,
        value if value >= 20 => 16.0,
        value if value >= 10 => 12.0,
        value if value >= min_recent_trades.max(3) => 8.0,
        value if value >= 1 => 4.0,
        _ => 0.0,
    } + match recent_trades_5m {
        value if value >= 20 => 12.0,
        value if value >= 10 => 8.0,
        value if value >= 3 => 4.0,
        value if value >= 1 => 2.0,
        _ => 0.0,
    };
    let age_score = if recent_trades_15m >= min_recent_trades {
        5.0
    } else if recent_trades_15m > 0 {
        2.0
    } else {
        0.0
    };

    liquidity_score + activity_score + age_score
}

fn prune_pool_activity(pool: &mut ParsedPool, now_unix: u64) {
    let cutoff = now_unix.saturating_sub(RECENT_15M_SECS);
    while let Some(front) = pool.activity_events_unix.front().copied() {
        if front < cutoff {
            pool.activity_events_unix.pop_front();
        } else {
            break;
        }
    }
}

fn record_pool_activity(
    known_pools: &mut HashMap<String, ParsedPool>,
    address: &str,
    now_unix: u64,
    latest_slot: u64,
) -> bool {
    let Some(pool) = known_pools.get_mut(address) else {
        return false;
    };
    pool.latest_slot = pool.latest_slot.max(latest_slot);
    prune_pool_activity(pool, now_unix);
    pool.activity_events_unix.push_back(now_unix);
    pool.hits = pool.hits.saturating_add(1);
    true
}

fn pool_recent_activity_counts(pool: &ParsedPool, now_unix: u64) -> (u64, u64) {
    let cutoff_5m = now_unix.saturating_sub(RECENT_5M_SECS);
    let cutoff_15m = now_unix.saturating_sub(RECENT_15M_SECS);
    let mut recent_5m = 0u64;
    let mut recent_15m = 0u64;
    for timestamp in &pool.activity_events_unix {
        if *timestamp >= cutoff_15m {
            recent_15m += 1;
        }
        if *timestamp >= cutoff_5m {
            recent_5m += 1;
        }
    }
    (recent_5m, recent_15m)
}

#[derive(Debug, Default, Clone, Copy)]
struct PoolVenueCounts {
    pumpswap: usize,
    meteora: usize,
    raydium: usize,
    whirlpool: usize,
}

impl PoolVenueCounts {
    fn compact(self) -> String {
        format!(
            "PumpSwap={}, Meteora={}, RaydiumCLMM={}, Whirlpool={}",
            self.pumpswap, self.meteora, self.raydium, self.whirlpool
        )
    }
}

fn parsed_pool_venue_counts(pools: &HashMap<String, ParsedPool>) -> PoolVenueCounts {
    let mut counts = PoolVenueCounts::default();
    for pool in pools.values() {
        match pool.venue {
            "pumpswap" => counts.pumpswap += 1,
            "meteora" => counts.meteora += 1,
            "raydium" => counts.raydium += 1,
            "whirlpool" => counts.whirlpool += 1,
            _ => {}
        }
    }
    counts
}

fn update_state(
    config: &ScannerConfig,
    fresh_pools: HashMap<String, FreshPool>,
) -> Result<ScannerState> {
    let now = unix_now();
    let source = scanner_state_source(config);

    if config.source == ScannerSource::Grpc {
        let mut pools = fresh_pools
            .into_values()
            .filter(|pool| !config.excluded_token_mints.contains(&pool.token_mint))
            .filter(|pool| !config.excluded_market_addresses.contains(&pool.address))
            .map(|pool| StatePool {
                address: pool.address,
                venue: pool.venue.to_string(),
                token_mint: pool.token_mint,
                quote_mint: pool.quote_mint,
                first_seen_unix: now,
                last_seen_unix: now,
                last_seen_slot: pool.last_seen_slot,
                hits: pool.hits.max(1),
                misses: 0,
                quote_liquidity_usdc: pool.quote_liquidity_usdc,
                recent_trades_5m: pool.recent_trades_5m,
                recent_trades_15m: pool.recent_trades_15m,
                recent_volume_15m_usd: pool.recent_volume_15m_usd,
            })
            .collect::<Vec<_>>();
        pools.sort_by(|a, b| {
            a.token_mint
                .cmp(&b.token_mint)
                .then_with(|| a.venue.cmp(&b.venue))
                .then_with(|| a.address.cmp(&b.address))
        });

        return Ok(ScannerState {
            source,
            updated_unix: now,
            pools,
        });
    }

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
                existing.recent_trades_5m = pool.recent_trades_5m;
                existing.recent_trades_15m = pool.recent_trades_15m;
                existing.recent_volume_15m_usd = pool.recent_volume_15m_usd;
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
                recent_trades_5m: pool.recent_trades_5m,
                recent_trades_15m: pool.recent_trades_15m,
                recent_volume_15m_usd: pool.recent_volume_15m_usd,
            });
    }

    if fresh_pools.is_empty() {
        let mut pools = active
            .into_values()
            .filter(|pool| pool.misses < config.max_misses)
            .collect::<Vec<_>>();
        pools = filter_snapshot_state_pools(config, pools);
        pools.sort_by(|a, b| {
            a.token_mint
                .cmp(&b.token_mint)
                .then_with(|| a.venue.cmp(&b.venue))
                .then_with(|| a.address.cmp(&b.address))
        });

        return Ok(ScannerState {
            source,
            updated_unix: now,
            pools,
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
    pools = filter_snapshot_state_pools(config, pools);
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

fn filter_snapshot_state_pools(config: &ScannerConfig, pools: Vec<StatePool>) -> Vec<StatePool> {
    let mut by_token: HashMap<String, Vec<StatePool>> = HashMap::new();
    for pool in pools {
        if state_pool_passes_snapshot_quality(config, &pool) {
            by_token
                .entry(pool.token_mint.clone())
                .or_default()
                .push(pool);
        }
    }

    let mut filtered = Vec::new();
    for (_token_mint, mut token_pools) in by_token {
        let mut best_liquidity_by_venue: HashMap<&str, f64> = HashMap::new();
        let mut token_recent_trades_5m = 0_u64;
        let mut token_recent_trades_15m = 0_u64;
        for pool in &token_pools {
            token_recent_trades_5m = token_recent_trades_5m.saturating_add(pool.recent_trades_5m);
            token_recent_trades_15m =
                token_recent_trades_15m.saturating_add(pool.recent_trades_15m);
            if is_supported_routeable_venue(&pool.venue) {
                best_liquidity_by_venue
                    .entry(pool.venue.as_str())
                    .and_modify(|value| *value = value.max(pool.quote_liquidity_usdc))
                    .or_insert(pool.quote_liquidity_usdc);
            }
        }

        let mut venue_liquidities = best_liquidity_by_venue.into_values().collect::<Vec<_>>();
        venue_liquidities
            .sort_by(|left, right| right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal));
        let pair_liquidity_usdc = venue_liquidities
            .get(0)
            .zip(venue_liquidities.get(1))
            .map(|(first, second)| (*first).min(*second))
            .unwrap_or_default();
        if pair_liquidity_usdc < config.min_pair_liquidity_usdc {
            continue;
        }
        if token_recent_trades_5m < MIN_GRPC_POOL_RECENT_TRADES_5M {
            continue;
        }
        if token_recent_trades_15m < config.min_token_recent_trades_15m {
            continue;
        }

        token_pools.sort_by(|left, right| {
            right
                .recent_trades_15m
                .cmp(&left.recent_trades_15m)
                .then_with(|| right.recent_trades_5m.cmp(&left.recent_trades_5m))
                .then_with(|| {
                    right
                        .quote_liquidity_usdc
                        .partial_cmp(&left.quote_liquidity_usdc)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| right.last_seen_slot.cmp(&left.last_seen_slot))
        });

        let mut pumpswap_count = 0usize;
        let mut meteora_count = 0usize;
        let mut raydium_count = 0usize;
        let mut whirlpool_count = 0usize;
        for pool in token_pools {
            match pool.venue.as_str() {
                "pumpswap" if pumpswap_count < config.max_pumpswap_per_token => {
                    pumpswap_count += 1;
                    filtered.push(pool);
                }
                "meteora" if meteora_count < config.max_meteora_per_token => {
                    meteora_count += 1;
                    filtered.push(pool);
                }
                "raydium" if raydium_count < config.max_meteora_per_token => {
                    raydium_count += 1;
                    filtered.push(pool);
                }
                "whirlpool" if whirlpool_count < config.max_meteora_per_token => {
                    whirlpool_count += 1;
                    filtered.push(pool);
                }
                _ => {}
            }
        }
    }

    filtered
}

fn state_pool_passes_snapshot_quality(config: &ScannerConfig, pool: &StatePool) -> bool {
    if !is_supported_routeable_venue(&pool.venue) {
        return false;
    }
    if pool.quote_mint != config.sol_mint {
        return false;
    }
    if !pool.quote_liquidity_usdc.is_finite()
        || pool.quote_liquidity_usdc < config.min_quote_liquidity_usdc
    {
        return false;
    }
    if pool.recent_trades_5m < MIN_GRPC_POOL_RECENT_TRADES_5M {
        return false;
    }
    if pool.recent_trades_15m < min_pool_recent_trades_15m(config) {
        return false;
    }
    pool.recent_volume_15m_usd >= min_pool_recent_volume_15m_usd(config)
}

fn scanner_state_source(config: &ScannerConfig) -> String {
    match config.source {
        ScannerSource::Api if config.dexscreener_enabled => {
            "api_geckoterminal_dexscreener_multi_dex"
        }
        ScannerSource::Api => "api_geckoterminal_multi_dex",
        ScannerSource::Grpc => "grpc_multi_dex",
        ScannerSource::Onchain => "onchain_multi_dex",
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
    for pool in &state.pools {
        let record = serde_json::json!({
            "address": pool.address,
            "dex_id": pool.venue,
            "token_mint": pool.token_mint,
            "quote_mint": pool.quote_mint,
            "quote_liquidity_usdc": pool.quote_liquidity_usdc,
            "first_seen_unix": pool.first_seen_unix,
            "last_seen_unix": pool.last_seen_unix,
            "last_seen_slot": pool.last_seen_slot,
            "hits": pool.hits,
            "misses": pool.misses,
            "recent_trades_5m": pool.recent_trades_5m,
            "recent_trades_15m": pool.recent_trades_15m,
            "recent_volume_15m_usd": pool.recent_volume_15m_usd,
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
        .filter(|venues| {
            venues
                .iter()
                .filter(|venue| is_supported_routeable_venue(venue))
                .count()
                >= 2
        })
        .count()
}

fn log_routeable_pool_summary(state: &ScannerState) {
    let mut by_token: HashMap<&str, PoolVenueCounts> = HashMap::new();
    for pool in &state.pools {
        let entry = by_token.entry(&pool.token_mint).or_default();
        match pool.venue.as_str() {
            "pumpswap" => entry.pumpswap += 1,
            "meteora" => entry.meteora += 1,
            "raydium" => entry.raydium += 1,
            "whirlpool" => entry.whirlpool += 1,
            _ => {}
        }
    }

    for (token, counts) in by_token {
        let venue_count = [
            counts.pumpswap,
            counts.meteora,
            counts.raydium,
            counts.whirlpool,
        ]
        .into_iter()
        .filter(|count| *count > 0)
        .count();
        if venue_count >= 2 {
            tracing::info!("保留双边池：币种={}，{}", token, counts.compact());
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

fn read_u8(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u128(data: &[u8], offset: usize) -> Option<u128> {
    let bytes = data.get(offset..offset + 16)?;
    Some(u128::from_le_bytes(bytes.try_into().ok()?))
}

#[allow(dead_code)]
fn mint_decimals(mint: &str, sol_mint: &str) -> i32 {
    if mint == sol_mint {
        9
    } else {
        6
    }
}

#[allow(dead_code)]
fn normalize_pool_price(
    raw_price_in_quote: f64,
    token_decimals: u8,
    quote_decimals: u8,
) -> Option<f64> {
    let adjustment = 10_f64.powi(token_decimals as i32 - quote_decimals as i32);
    let price = raw_price_in_quote * adjustment;
    (price.is_finite() && price > 0.0).then_some(price)
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
        .find_map(|key| env::var(key).ok().and_then(|value| parse_env_bool(&value)))
        .unwrap_or(default)
}

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: &str = "So11111111111111111111111111111111111111112";
    const TOKEN: &str = "Token11111111111111111111111111111111111111";

    fn token_account_data(amount: u64) -> Vec<u8> {
        let mut data = vec![0u8; TOKEN_ACCOUNT_AMOUNT_OFFSET + TOKEN_ACCOUNT_AMOUNT_LEN];
        data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_OFFSET + TOKEN_ACCOUNT_AMOUNT_LEN]
            .copy_from_slice(&amount.to_le_bytes());
        data
    }

    fn base_pool(venue: &'static str) -> ParsedPool {
        ParsedPool {
            address: format!("{}_pool", venue),
            venue,
            token_mint: TOKEN.to_string(),
            quote_mint: SOL.to_string(),
            token_vault: "token_vault".to_string(),
            quote_vault: "quote_vault".to_string(),
            quote_liquidity_usdc: 0.0,
            token_decimals: 6,
            quote_decimals: 9,
            spot_price_sol_per_token: None,
            clmm_sqrt_price_x64: None,
            clmm_quote_is_token_0: None,
            latest_slot: 0,
            hits: 1,
            activity_events_unix: VecDeque::new(),
            meteora_active_id: None,
            meteora_bin_step: None,
            meteora_quote_is_x: None,
            meteora_base_factor: None,
            meteora_variable_fee_control: None,
            meteora_base_fee_power_factor: None,
            meteora_volatility_accumulator: None,
        }
    }

    #[test]
    fn pumpswap_price_ratio_normalizes_sol_quote_decimals() {
        let pool = base_pool("pumpswap");
        let mut vault_data = HashMap::new();
        vault_data.insert("token_vault".to_string(), token_account_data(1_000_000));
        vault_data.insert("quote_vault".to_string(), token_account_data(2_000_000));

        let price = pool_price_ratio(&pool, &vault_data, SOL).unwrap();

        assert!((price - 0.002).abs() < 1e-12);
    }

    #[test]
    fn meteora_price_ratio_uses_active_bin_price() {
        let mut pool = base_pool("meteora");
        pool.meteora_active_id = Some(10);
        pool.meteora_bin_step = Some(25);
        pool.meteora_quote_is_x = Some(false);
        let expected = (1.0_f64 + 25.0 / 10_000.0).powf(10.0) * 0.001;

        let price = pool_price_ratio(&pool, &HashMap::new(), SOL).unwrap();

        assert!((price - expected).abs() < 1e-12);
    }

    #[test]
    fn meteora_quote_direction_follows_wsol_side() {
        let mut wsol_is_x = base_pool("meteora");
        wsol_is_x.meteora_quote_is_x = Some(true);
        assert_eq!(scanner_meteora_x_to_y(&wsol_is_x, SOL, TOKEN), Some(true));
        assert_eq!(scanner_meteora_x_to_y(&wsol_is_x, TOKEN, SOL), Some(false));

        let mut wsol_is_y = base_pool("meteora");
        wsol_is_y.meteora_quote_is_x = Some(false);
        assert_eq!(scanner_meteora_x_to_y(&wsol_is_y, SOL, TOKEN), Some(false));
        assert_eq!(scanner_meteora_x_to_y(&wsol_is_y, TOKEN, SOL), Some(true));
    }

    #[test]
    fn scanner_trade_sizes_expand_tiny_sol_ladder() {
        let mut config = config::Config::default();
        config.strategy.program_pair_trade_sizes_sol = vec![0.001, 0.002, 0.005, 0.01];

        let sizes = scanner_executable_trade_sizes_sol(&config);

        assert!(sizes.iter().any(|size| *size >= 8.0));
        assert!(sizes.len() > 4);
    }

    #[test]
    fn monitored_programs_include_raydium_and_whirlpool() {
        let app_config = config::Config::default();
        let monitored_programs = vec![
            MonitoredProgram {
                label: config::ProgramKind::Pumpswap.default_label(),
                program_id: resolve_program_id(
                    &app_config,
                    config::ProgramKind::Pumpswap,
                    PUMPSWAP_PROGRAM_ID,
                ),
            },
            MonitoredProgram {
                label: config::ProgramKind::MeteoraDlmm.default_label(),
                program_id: resolve_program_id(
                    &app_config,
                    config::ProgramKind::MeteoraDlmm,
                    METEORA_DLMM_PROGRAM_ID,
                ),
            },
            MonitoredProgram {
                label: config::ProgramKind::RaydiumClmm.default_label(),
                program_id: resolve_program_id(
                    &app_config,
                    config::ProgramKind::RaydiumClmm,
                    RAYDIUM_CLMM_PROGRAM_ID,
                ),
            },
            MonitoredProgram {
                label: config::ProgramKind::Whirlpool.default_label(),
                program_id: resolve_program_id(
                    &app_config,
                    config::ProgramKind::Whirlpool,
                    WHIRLPOOL_PROGRAM_ID,
                ),
            },
        ];

        assert!(monitored_programs
            .iter()
            .any(|program| program.label == "Raydium Concentrated Liquidity"));
        assert!(monitored_programs
            .iter()
            .any(|program| program.label == "Whirlpools Program"));
        assert!(monitored_programs
            .iter()
            .any(|program| program.program_id == RAYDIUM_CLMM_PROGRAM_ID));
        assert!(monitored_programs
            .iter()
            .any(|program| program.program_id == WHIRLPOOL_PROGRAM_ID));
    }
}
