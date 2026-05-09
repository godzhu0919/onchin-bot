use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    #[serde(default)]
    pub rpc: RpcConfig,
    #[serde(default)]
    pub subscription: SubscriptionConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    pub strategy: StrategyConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default = "default_programs")]
    pub programs: Vec<ProgramConfig>,
    pub tokens: TokenConfig,
    pub pump_accounts: Vec<AccountConfig>,
    pub meteora_pools: Vec<AccountConfig>,
    pub raydium_pools: Vec<AccountConfig>,
    #[serde(default)]
    pub whirlpool_pools: Vec<AccountConfig>,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub endpoint: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcConfig {
    #[serde(default = "default_rpc_http_url")]
    pub http_url: String,
}

fn default_rpc_http_url() -> String {
    "http://127.0.0.1:8899".to_string()
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            http_url: default_rpc_http_url(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionConfig {
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
    #[serde(default = "default_reconnect_initial_ms")]
    pub reconnect_initial_ms: u64,
    #[serde(default = "default_reconnect_max_ms")]
    pub reconnect_max_ms: u64,
    #[serde(default = "default_stream_idle_timeout_secs")]
    pub stream_idle_timeout_secs: u64,
}

pub const MAX_EFFECTIVE_SUBSCRIPTION_CHANNEL_CAPACITY: usize = 4_096;

fn default_channel_capacity() -> usize {
    MAX_EFFECTIVE_SUBSCRIPTION_CHANNEL_CAPACITY
}

fn default_reconnect_initial_ms() -> u64 {
    500
}

fn default_reconnect_max_ms() -> u64 {
    30_000
}

fn default_stream_idle_timeout_secs() -> u64 {
    60
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            channel_capacity: default_channel_capacity(),
            reconnect_initial_ms: default_reconnect_initial_ms(),
            reconnect_max_ms: default_reconnect_max_ms(),
            stream_idle_timeout_secs: default_stream_idle_timeout_secs(),
        }
    }
}

impl SubscriptionConfig {
    pub fn effective_channel_capacity(&self) -> usize {
        self.channel_capacity
            .max(1)
            .min(MAX_EFFECTIVE_SUBSCRIPTION_CHANNEL_CAPACITY)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExecutionConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub dry_run_only: Option<bool>,
    #[serde(default)]
    pub max_slippage_bps: Option<u64>,
    #[serde(default)]
    pub live_send_min_profit_pct: Option<f64>,
    #[serde(default)]
    pub require_pre_send_simulation: Option<bool>,
    #[serde(default)]
    pub send_transport: Option<String>,
    #[serde(default)]
    pub jito_tip_lamports: Option<u64>,
    #[serde(default)]
    pub direct_send_extra_edge_pct: Option<f64>,
    #[serde(default)]
    pub direct_send_extra_edge_usdc: Option<f64>,
    #[serde(default)]
    pub direct_send_min_gate_pct: Option<f64>,
    #[serde(default)]
    pub direct_send_max_state_age_ms: Option<u64>,
    #[serde(default)]
    pub send_on_positive_spot: Option<bool>,
    #[serde(default)]
    pub skip_monitor_only_routes_when_live: Option<bool>,
    #[serde(default)]
    pub execution_min_interval_ms: Option<u64>,
    #[serde(default)]
    pub address_lookup_tables: Option<Vec<String>>,
    #[serde(default)]
    pub compute_unit_limit: Option<u32>,
    #[serde(default)]
    pub compute_unit_limit_margin_bps: Option<u32>,
    #[serde(default)]
    pub compute_unit_limit_min_buffer: Option<u32>,
    #[serde(default)]
    pub compute_unit_price_micro_lamports: Option<u64>,
    #[serde(default)]
    pub loaded_accounts_data_size_limit: Option<u32>,
    #[serde(default)]
    pub two_hop_executor_program_id: Option<String>,
    #[serde(default)]
    pub fast_path_direct_send: Option<bool>,
    #[serde(default)]
    pub disable_direct_send_skip: Option<bool>,
    #[serde(default)]
    pub disable_local_profit_checks: Option<bool>,
    #[serde(default)]
    pub disable_fast_path_spot_profit_filter: Option<bool>,
    #[serde(default)]
    pub enable_meteora_clmm_live_send: Option<bool>,
    #[serde(default)]
    pub enable_meteora_meteora_live_send: Option<bool>,
    #[serde(default)]
    pub program_pair_min_profit_lamports: Option<u64>,
    #[serde(default)]
    pub two_hop_profit_guard_min_usdc: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    pub min_profit_threshold: f64,
    pub sol_usdc_price: f64,
    #[serde(default = "default_trade_sizes")]
    pub trade_sizes: Vec<f64>,
    #[serde(default = "default_program_pair_trade_sizes_sol")]
    pub program_pair_trade_sizes_sol: Vec<f64>,
    #[serde(default = "default_program_pair_best_size_search_samples")]
    pub program_pair_best_size_search_samples: usize,
    #[serde(default = "default_program_pair_max_send_candidates_per_scan")]
    pub program_pair_max_send_candidates_per_scan: usize,
    #[serde(default = "default_program_pair_send_min_gross_profit_pct")]
    pub program_pair_send_min_gross_profit_pct: f64,
    #[serde(default = "default_program_pair_near_profit_margin_pct")]
    pub program_pair_near_profit_margin_pct: f64,
    #[serde(default = "default_program_pair_dynamic_gate_max_adjustment_pct")]
    pub program_pair_dynamic_gate_max_adjustment_pct: f64,
    #[serde(default = "default_preflight_gas_cost_sol")]
    pub preflight_gas_cost_sol: f64,
    #[serde(default = "default_pumpswap_buy_output_buffer_bps")]
    pub pumpswap_buy_output_buffer_bps: f64,
    #[serde(default = "default_raydium_clmm_output_buffer_bps")]
    pub raydium_clmm_output_buffer_bps: f64,
    #[serde(default = "default_pumpswap_meteora_small_pool_liquidity_usdc")]
    pub pumpswap_meteora_small_pool_liquidity_usdc: f64,
    #[serde(default = "default_pumpswap_meteora_max_trade_depth_bps")]
    pub pumpswap_meteora_max_trade_depth_bps: f64,
    #[serde(default = "default_pumpswap_meteora_buy_slippage_bps")]
    pub pumpswap_meteora_buy_slippage_bps: f64,
    #[serde(default = "default_pumpswap_meteora_buy_slippage_bps_small_pool")]
    pub pumpswap_meteora_buy_slippage_bps_small_pool: f64,
    #[serde(default = "default_pumpswap_meteora_sell_slippage_bps")]
    pub pumpswap_meteora_sell_slippage_bps: f64,
    #[serde(default = "default_pumpswap_meteora_sell_slippage_bps_small_pool")]
    pub pumpswap_meteora_sell_slippage_bps_small_pool: f64,
    #[serde(default = "default_pumpswap_meteora_max_touched_bin_arrays")]
    pub pumpswap_meteora_max_touched_bin_arrays: usize,
    #[serde(default = "default_pumpswap_meteora_max_meteora_fee_bps")]
    pub pumpswap_meteora_max_meteora_fee_bps: f64,
    #[serde(default = "default_pumpswap_meteora_failure_buffer_usdc")]
    pub pumpswap_meteora_failure_buffer_usdc: f64,
    #[serde(default = "default_pumpswap_meteora_wrap_unwrap_cost_sol")]
    pub pumpswap_meteora_wrap_unwrap_cost_sol: f64,
    #[serde(default = "default_pumpswap_meteora_ata_cost_sol")]
    pub pumpswap_meteora_ata_cost_sol: f64,
    #[serde(default = "default_pumpswap_fee_lp_share_bps")]
    pub pumpswap_fee_lp_share_bps: u64,
    #[serde(default = "default_pumpswap_fee_protocol_share_bps")]
    pub pumpswap_fee_protocol_share_bps: u64,
    #[serde(default = "default_pumpswap_fee_coin_creator_share_bps")]
    pub pumpswap_fee_coin_creator_share_bps: u64,
    #[serde(default = "default_pumpswap_meteora_class_a_min_profit_usdc")]
    pub pumpswap_meteora_class_a_min_profit_usdc: f64,
    #[serde(default = "default_pumpswap_meteora_class_b_min_profit_usdc")]
    pub pumpswap_meteora_class_b_min_profit_usdc: f64,
    #[serde(default = "default_pumpswap_meteora_class_a_min_confidence")]
    pub pumpswap_meteora_class_a_min_confidence: f64,
    #[serde(default = "default_pumpswap_meteora_class_b_min_confidence")]
    pub pumpswap_meteora_class_b_min_confidence: f64,
    #[serde(default = "default_pumpswap_meteora_min_hit_rate")]
    pub pumpswap_meteora_min_hit_rate: f64,
    #[serde(default = "default_pumpswap_meteora_min_hit_rate_samples")]
    pub pumpswap_meteora_min_hit_rate_samples: u64,
    #[serde(default = "default_max_direct_tokens_per_scan")]
    pub max_direct_tokens_per_scan: usize,
    #[serde(default = "default_max_meteora_pools_per_token_for_direct_scan")]
    pub max_meteora_pools_per_token_for_direct_scan: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_true")]
    pub token_selection_enabled: bool,
    #[serde(default = "default_token_selection_min_score")]
    pub token_selection_min_score: f64,
    #[serde(default = "default_token_selection_min_total_liquidity_usdc")]
    pub token_selection_min_total_liquidity_usdc: f64,
    #[serde(default = "default_token_selection_log_top_n")]
    pub token_selection_log_top_n: usize,
    #[serde(default = "default_max_pairs_per_token")]
    pub max_pairs_per_token: usize,
    #[serde(default = "default_min_liquidity_usdc")]
    pub min_liquidity_usdc: f64,
    #[serde(default = "default_min_volume_h24")]
    pub min_volume_h24: f64,
    #[serde(default)]
    pub manual_market_addresses: Vec<String>,
    #[serde(default = "default_dynamic_market_addresses_path")]
    pub dynamic_market_addresses_path: String,
    #[serde(default = "default_excluded_target_token_mints")]
    pub excluded_target_token_mints: Vec<String>,
    #[serde(default)]
    pub excluded_market_addresses: Vec<String>,
    #[serde(default)]
    pub require_routeable_pairs: bool,
    #[serde(default = "default_routeable_dexes")]
    pub routeable_dexes: Vec<String>,
    #[serde(default = "default_min_routeable_dex_count")]
    pub min_routeable_dex_count: usize,
}

fn default_true() -> bool {
    true
}

fn default_token_selection_min_score() -> f64 {
    45.0
}

fn default_token_selection_min_total_liquidity_usdc() -> f64 {
    3_000.0
}

fn default_token_selection_log_top_n() -> usize {
    12
}

fn default_max_pairs_per_token() -> usize {
    3
}

fn default_min_liquidity_usdc() -> f64 {
    1_000.0
}

fn default_min_volume_h24() -> f64 {
    100.0
}

fn default_dynamic_market_addresses_path() -> String {
    "dynamic_market_addresses.txt".to_string()
}

fn default_excluded_target_token_mints() -> Vec<String> {
    vec![
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
        "USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB".to_string(),
    ]
}

fn default_routeable_dexes() -> Vec<String> {
    vec![
        "pumpswap".to_string(),
        "raydium".to_string(),
        "meteora".to_string(),
        "whirlpool".to_string(),
    ]
}

fn default_min_routeable_dex_count() -> usize {
    2
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            token_selection_enabled: default_true(),
            token_selection_min_score: default_token_selection_min_score(),
            token_selection_min_total_liquidity_usdc:
                default_token_selection_min_total_liquidity_usdc(),
            token_selection_log_top_n: default_token_selection_log_top_n(),
            max_pairs_per_token: default_max_pairs_per_token(),
            min_liquidity_usdc: default_min_liquidity_usdc(),
            min_volume_h24: default_min_volume_h24(),
            manual_market_addresses: Vec::new(),
            dynamic_market_addresses_path: default_dynamic_market_addresses_path(),
            excluded_target_token_mints: default_excluded_target_token_mints(),
            excluded_market_addresses: Vec::new(),
            require_routeable_pairs: false,
            routeable_dexes: default_routeable_dexes(),
            min_routeable_dex_count: default_min_routeable_dex_count(),
        }
    }
}

fn default_trade_sizes() -> Vec<f64> {
    vec![10.0, 50.0, 100.0, 500.0]
}

fn default_program_pair_trade_sizes_sol() -> Vec<f64> {
    vec![0.05, 0.1, 0.25, 0.5, 0.75, 0.9]
}

fn default_program_pair_best_size_search_samples() -> usize {
    4
}

fn default_program_pair_max_send_candidates_per_scan() -> usize {
    1
}

fn default_program_pair_send_min_gross_profit_pct() -> f64 {
    0.0
}

fn default_program_pair_near_profit_margin_pct() -> f64 {
    0.5
}

fn default_program_pair_dynamic_gate_max_adjustment_pct() -> f64 {
    0.35
}

fn default_preflight_gas_cost_sol() -> f64 {
    0.00005
}

fn default_pumpswap_buy_output_buffer_bps() -> f64 {
    200.0
}

fn default_raydium_clmm_output_buffer_bps() -> f64 {
    250.0
}

fn default_pumpswap_meteora_small_pool_liquidity_usdc() -> f64 {
    2_500.0
}

fn default_pumpswap_meteora_max_trade_depth_bps() -> f64 {
    600.0
}

fn default_pumpswap_meteora_buy_slippage_bps() -> f64 {
    125.0
}

fn default_pumpswap_meteora_buy_slippage_bps_small_pool() -> f64 {
    275.0
}

fn default_pumpswap_meteora_sell_slippage_bps() -> f64 {
    90.0
}

fn default_pumpswap_meteora_sell_slippage_bps_small_pool() -> f64 {
    225.0
}

fn default_pumpswap_meteora_max_touched_bin_arrays() -> usize {
    5
}

fn default_pumpswap_meteora_max_meteora_fee_bps() -> f64 {
    180.0
}

fn default_pumpswap_meteora_failure_buffer_usdc() -> f64 {
    0.03
}

fn default_pumpswap_meteora_wrap_unwrap_cost_sol() -> f64 {
    0.0
}

fn default_pumpswap_meteora_ata_cost_sol() -> f64 {
    0.0
}

fn default_pumpswap_fee_lp_share_bps() -> u64 {
    8_000
}

fn default_pumpswap_fee_protocol_share_bps() -> u64 {
    1_250
}

fn default_pumpswap_fee_coin_creator_share_bps() -> u64 {
    750
}

fn default_pumpswap_meteora_class_a_min_profit_usdc() -> f64 {
    0.08
}

fn default_pumpswap_meteora_class_b_min_profit_usdc() -> f64 {
    0.03
}

fn default_pumpswap_meteora_class_a_min_confidence() -> f64 {
    0.72
}

fn default_pumpswap_meteora_class_b_min_confidence() -> f64 {
    0.45
}

fn default_pumpswap_meteora_min_hit_rate() -> f64 {
    0.25
}

fn default_pumpswap_meteora_min_hit_rate_samples() -> u64 {
    4
}

fn default_max_direct_tokens_per_scan() -> usize {
    4
}

fn default_max_meteora_pools_per_token_for_direct_scan() -> usize {
    2
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProgramKind {
    Pumpfun,
    Pumpswap,
    MeteoraDlmm,
    RaydiumClmm,
    RaydiumAmmV4,
    RaydiumCpmm,
    Whirlpool,
    Humendfi,
    Pancakeswap,
}

impl ProgramKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pumpfun => "pumpfun",
            Self::Pumpswap => "pumpswap",
            Self::MeteoraDlmm => "meteora_dlmm",
            Self::RaydiumClmm => "raydium_clmm",
            Self::RaydiumAmmV4 => "raydium_amm_v4",
            Self::RaydiumCpmm => "raydium_cpmm",
            Self::Whirlpool => "whirlpool",
            Self::Humendfi => "humendfi",
            Self::Pancakeswap => "pancakeswap",
        }
    }

    pub fn default_label(self) -> &'static str {
        match self {
            Self::Pumpfun => "Pump.fun",
            Self::Pumpswap => "PumpSwap",
            Self::MeteoraDlmm => "Meteora DLMM",
            Self::RaydiumClmm => "Raydium Concentrated Liquidity",
            Self::RaydiumAmmV4 => "Raydium Liquidity Pool V4",
            Self::RaydiumCpmm => "Raydium CPMM",
            Self::Whirlpool => "Whirlpools Program",
            Self::Humendfi => "HumendiFi",
            Self::Pancakeswap => "PancakeSwap",
        }
    }

    pub fn default_discovery_dex_ids(self) -> &'static [&'static str] {
        match self {
            Self::Pumpfun => &["pumpfun"],
            Self::Pumpswap => &["pumpswap"],
            Self::MeteoraDlmm => &["meteora"],
            Self::RaydiumClmm | Self::RaydiumAmmV4 | Self::RaydiumCpmm => &["raydium"],
            Self::Whirlpool => &["orca", "whirlpool"],
            Self::Humendfi => &["humendfi"],
            Self::Pancakeswap => &["pancakeswap"],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProgramConfig {
    pub kind: ProgramKind,
    pub program_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub discovery_dex_ids: Vec<String>,
}

impl ProgramConfig {
    pub fn label(&self) -> &str {
        self.label
            .as_deref()
            .unwrap_or_else(|| self.kind.default_label())
    }

    pub fn discovery_dex_ids(&self) -> Vec<String> {
        if self.discovery_dex_ids.is_empty() {
            self.kind
                .default_discovery_dex_ids()
                .iter()
                .map(|value| value.to_string())
                .collect()
        } else {
            self.discovery_dex_ids.clone()
        }
    }
}

fn default_programs() -> Vec<ProgramConfig> {
    vec![
        ProgramConfig {
            kind: ProgramKind::Pumpfun,
            program_id: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::Pumpswap,
            program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::MeteoraDlmm,
            program_id: "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::RaydiumClmm,
            program_id: "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::RaydiumAmmV4,
            program_id: "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::RaydiumCpmm,
            program_id: "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::Whirlpool,
            program_id: "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
            enabled: true,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::Humendfi,
            program_id: "9H6tua7jkLhdm3w8BvgpTn5LZNU7g4ZynDmCiNN3q6Rp".to_string(),
            enabled: false,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
        ProgramConfig {
            kind: ProgramKind::Pancakeswap,
            program_id: String::new(),
            enabled: false,
            label: None,
            discovery_dex_ids: Vec::new(),
        },
    ]
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenConfig {
    pub usdc_mint: String,
    pub sol_mint: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    pub address: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub heartbeat_interval: u64,
    #[serde(default = "default_idle_status_interval_secs")]
    pub idle_status_interval_secs: u64,
}

fn default_idle_status_interval_secs() -> u64 {
    15
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).context("Failed to read config file")?;

        let config: Config = toml::from_str(&content).context("Failed to parse config file")?;

        Ok(config)
    }

    pub fn from_file_or_default() -> Result<Self> {
        if Path::new("config.toml").exists() {
            Self::from_file("config.toml")
        } else {
            tracing::warn!("config.toml not found, using default configuration");
            Ok(Self::default())
        }
    }

    pub fn get_enabled_pump_accounts(&self) -> Vec<String> {
        self.pump_accounts
            .iter()
            .filter(|acc| acc.enabled)
            .map(|acc| acc.address.clone())
            .collect()
    }

    pub fn get_enabled_meteora_pools(&self) -> Vec<String> {
        self.meteora_pools
            .iter()
            .filter(|pool| pool.enabled)
            .map(|pool| pool.address.clone())
            .collect()
    }

    pub fn get_enabled_raydium_pools(&self) -> Vec<String> {
        self.raydium_pools
            .iter()
            .filter(|pool| pool.enabled)
            .map(|pool| pool.address.clone())
            .collect()
    }

    pub fn get_enabled_whirlpool_pools(&self) -> Vec<String> {
        self.whirlpool_pools
            .iter()
            .filter(|pool| pool.enabled)
            .map(|pool| pool.address.clone())
            .collect()
    }

    pub fn enabled_programs(&self) -> Vec<&ProgramConfig> {
        self.programs
            .iter()
            .filter(|program| program.enabled && !program.program_id.trim().is_empty())
            .collect()
    }

    pub fn enabled_program_kinds(&self) -> Vec<ProgramKind> {
        self.enabled_programs()
            .into_iter()
            .map(|program| program.kind)
            .collect()
    }

    pub fn program_by_kind(&self, kind: ProgramKind) -> Option<&ProgramConfig> {
        self.programs
            .iter()
            .find(|program| program.kind == kind && program.enabled)
    }

    pub fn program_label(&self, kind: ProgramKind) -> &str {
        self.program_by_kind(kind)
            .map(|program| program.label())
            .unwrap_or_else(|| kind.default_label())
    }

    pub fn resolve_program_kind_by_owner(&self, owner: &str) -> Option<ProgramKind> {
        self.programs.iter().find_map(|program| {
            (program.enabled
                && !program.program_id.trim().is_empty()
                && program.program_id == owner)
                .then_some(program.kind)
        })
    }

    pub fn effective_routeable_dexes(&self) -> Vec<String> {
        let mut dexes = self.discovery.routeable_dexes.clone();
        for program in self.enabled_programs() {
            dexes.extend(program.discovery_dex_ids());
        }
        normalize_string_list(&mut dexes);
        dexes
    }

    pub fn apply_runtime_overrides_to_env(&self) {
        std::env::set_var("RPC_HTTP_URL", &self.rpc.http_url);
        apply_env_override("ENABLE_EXECUTION", self.execution.enabled);
        apply_env_override("DRY_RUN_ONLY", self.execution.dry_run_only);
        apply_env_override("MAX_SLIPPAGE_BPS", self.execution.max_slippage_bps);
        apply_env_override(
            "LIVE_SEND_MIN_PROFIT_PCT",
            self.execution.live_send_min_profit_pct,
        );
        apply_env_override(
            "REQUIRE_PRE_SEND_SIMULATION",
            self.execution.require_pre_send_simulation,
        );
        apply_env_override("SEND_TRANSPORT", self.execution.send_transport.as_deref());
        apply_env_override("JITO_TIP_LAMPORTS", self.execution.jito_tip_lamports);
        apply_env_override(
            "DIRECT_SEND_EXTRA_EDGE_PCT",
            self.execution.direct_send_extra_edge_pct,
        );
        apply_env_override(
            "DIRECT_SEND_EXTRA_EDGE_USDC",
            self.execution.direct_send_extra_edge_usdc,
        );
        apply_env_override(
            "DIRECT_SEND_MIN_GATE_PCT",
            self.execution.direct_send_min_gate_pct,
        );
        apply_env_override(
            "DIRECT_SEND_MAX_STATE_AGE_MS",
            self.execution.direct_send_max_state_age_ms,
        );
        apply_env_override(
            "SEND_ON_POSITIVE_SPOT",
            self.execution.send_on_positive_spot,
        );
        apply_env_override(
            "SKIP_MONITOR_ONLY_ROUTES_WHEN_LIVE",
            self.execution.skip_monitor_only_routes_when_live,
        );
        apply_env_override(
            "EXECUTION_MIN_INTERVAL_MS",
            self.execution.execution_min_interval_ms,
        );
        apply_env_override(
            "ADDRESS_LOOKUP_TABLES",
            self.execution
                .address_lookup_tables
                .as_ref()
                .map(|values| values.join(",")),
        );
        apply_env_override(
            "EXECUTION_COMPUTE_UNIT_LIMIT",
            self.execution.compute_unit_limit,
        );
        apply_env_override(
            "EXECUTION_COMPUTE_UNIT_LIMIT_MARGIN_BPS",
            self.execution.compute_unit_limit_margin_bps,
        );
        apply_env_override(
            "EXECUTION_COMPUTE_UNIT_LIMIT_MIN_BUFFER",
            self.execution.compute_unit_limit_min_buffer,
        );
        apply_env_override(
            "EXECUTION_COMPUTE_UNIT_PRICE_MICROLAMPORTS",
            self.execution.compute_unit_price_micro_lamports,
        );
        apply_env_override(
            "EXECUTION_LOADED_ACCOUNTS_DATA_SIZE_LIMIT",
            self.execution.loaded_accounts_data_size_limit,
        );
        apply_env_override(
            "TWO_HOP_EXECUTOR_PROGRAM_ID",
            self.execution.two_hop_executor_program_id.as_deref(),
        );
        apply_env_override(
            "FAST_PATH_DIRECT_SEND",
            self.execution.fast_path_direct_send,
        );
        apply_env_override(
            "DISABLE_DIRECT_SEND_SKIP",
            self.execution.disable_direct_send_skip,
        );
        apply_env_override(
            "DISABLE_LOCAL_PROFIT_CHECKS",
            self.execution.disable_local_profit_checks,
        );
        apply_env_override(
            "DISABLE_FAST_PATH_SPOT_PROFIT_FILTER",
            self.execution.disable_fast_path_spot_profit_filter,
        );
        apply_env_override(
            "ENABLE_METEORA_CLMM_LIVE_SEND",
            self.execution.enable_meteora_clmm_live_send,
        );
        apply_env_override(
            "ENABLE_METEORA_METEORA_LIVE_SEND",
            self.execution.enable_meteora_meteora_live_send,
        );
        apply_env_override(
            "PROGRAM_PAIR_MIN_PROFIT_LAMPORTS",
            self.execution.program_pair_min_profit_lamports,
        );
        apply_env_override(
            "TWO_HOP_PROFIT_GUARD_MIN_USDC",
            self.execution.two_hop_profit_guard_min_usdc,
        );
    }
}

fn normalize_string_list(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        !normalized.is_empty() && seen.insert(normalized)
    });
}

impl Default for Config {
    fn default() -> Self {
        Config {
            grpc: GrpcConfig {
                endpoint: "127.0.0.1:10000".to_string(),
                token: None,
            },
            rpc: RpcConfig::default(),
            subscription: SubscriptionConfig::default(),
            execution: ExecutionConfig::default(),
            strategy: StrategyConfig {
                min_profit_threshold: 2.0,
                sol_usdc_price: 150.0,
                trade_sizes: default_trade_sizes(),
                program_pair_trade_sizes_sol: default_program_pair_trade_sizes_sol(),
                program_pair_best_size_search_samples:
                    default_program_pair_best_size_search_samples(),
                program_pair_max_send_candidates_per_scan:
                    default_program_pair_max_send_candidates_per_scan(),
                program_pair_send_min_gross_profit_pct:
                    default_program_pair_send_min_gross_profit_pct(),
                program_pair_near_profit_margin_pct: default_program_pair_near_profit_margin_pct(),
                program_pair_dynamic_gate_max_adjustment_pct:
                    default_program_pair_dynamic_gate_max_adjustment_pct(),
                preflight_gas_cost_sol: default_preflight_gas_cost_sol(),
                pumpswap_buy_output_buffer_bps: default_pumpswap_buy_output_buffer_bps(),
                raydium_clmm_output_buffer_bps: default_raydium_clmm_output_buffer_bps(),
                pumpswap_meteora_small_pool_liquidity_usdc:
                    default_pumpswap_meteora_small_pool_liquidity_usdc(),
                pumpswap_meteora_max_trade_depth_bps: default_pumpswap_meteora_max_trade_depth_bps(
                ),
                pumpswap_meteora_buy_slippage_bps: default_pumpswap_meteora_buy_slippage_bps(),
                pumpswap_meteora_buy_slippage_bps_small_pool:
                    default_pumpswap_meteora_buy_slippage_bps_small_pool(),
                pumpswap_meteora_sell_slippage_bps: default_pumpswap_meteora_sell_slippage_bps(),
                pumpswap_meteora_sell_slippage_bps_small_pool:
                    default_pumpswap_meteora_sell_slippage_bps_small_pool(),
                pumpswap_meteora_max_touched_bin_arrays:
                    default_pumpswap_meteora_max_touched_bin_arrays(),
                pumpswap_meteora_max_meteora_fee_bps: default_pumpswap_meteora_max_meteora_fee_bps(
                ),
                pumpswap_meteora_failure_buffer_usdc: default_pumpswap_meteora_failure_buffer_usdc(
                ),
                pumpswap_meteora_wrap_unwrap_cost_sol:
                    default_pumpswap_meteora_wrap_unwrap_cost_sol(),
                pumpswap_meteora_ata_cost_sol: default_pumpswap_meteora_ata_cost_sol(),
                pumpswap_fee_lp_share_bps: default_pumpswap_fee_lp_share_bps(),
                pumpswap_fee_protocol_share_bps: default_pumpswap_fee_protocol_share_bps(),
                pumpswap_fee_coin_creator_share_bps: default_pumpswap_fee_coin_creator_share_bps(),
                pumpswap_meteora_class_a_min_profit_usdc:
                    default_pumpswap_meteora_class_a_min_profit_usdc(),
                pumpswap_meteora_class_b_min_profit_usdc:
                    default_pumpswap_meteora_class_b_min_profit_usdc(),
                pumpswap_meteora_class_a_min_confidence:
                    default_pumpswap_meteora_class_a_min_confidence(),
                pumpswap_meteora_class_b_min_confidence:
                    default_pumpswap_meteora_class_b_min_confidence(),
                pumpswap_meteora_min_hit_rate: default_pumpswap_meteora_min_hit_rate(),
                pumpswap_meteora_min_hit_rate_samples:
                    default_pumpswap_meteora_min_hit_rate_samples(),
                max_direct_tokens_per_scan: default_max_direct_tokens_per_scan(),
                max_meteora_pools_per_token_for_direct_scan:
                    default_max_meteora_pools_per_token_for_direct_scan(),
            },
            discovery: DiscoveryConfig::default(),
            programs: default_programs(),
            tokens: TokenConfig {
                usdc_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                sol_mint: "So11111111111111111111111111111111111111112".to_string(),
            },
            pump_accounts: vec![AccountConfig {
                address: "6v7aDnzcXgjGmRAvRgZpEay1g4qR28pForXZuD8Xqjbf".to_string(),
                name: "Example Pump Token".to_string(),
                enabled: true,
            }],
            meteora_pools: vec![AccountConfig {
                address: "4ArWkekbQ2HHmZUiP613eqJhJ8Xqc6tnrSx9UjT5nxVN".to_string(),
                name: "ALLAH/WSOL DLMM".to_string(),
                enabled: true,
            }],
            raydium_pools: vec![AccountConfig {
                address: "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2".to_string(),
                name: "SOL/USDC (Raydium V4)".to_string(),
                enabled: true,
            }],
            whirlpool_pools: Vec::new(),
            logging: LoggingConfig {
                level: "info".to_string(),
                heartbeat_interval: 50,
                idle_status_interval_secs: default_idle_status_interval_secs(),
            },
        }
    }
}

fn apply_env_override<T>(key: &str, value: Option<T>)
where
    T: ToString,
{
    if let Some(value) = value {
        std::env::set_var(key, value.to_string());
    }
}
