use anyhow::{Context, Result};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::str::FromStr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionScope {
    pub enabled: bool,
    pub dry_run_only: bool,
    pub skip_monitor_only_routes_when_live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTransport {
    Rpc,
    JitoBundle,
}

pub struct ExecutorConfig {
    pub keypair: Keypair,
    pub max_slippage_bps: u64,
    pub compute_unit_limit: u32,
    pub compute_unit_limit_margin_bps: u32,
    pub compute_unit_limit_min_buffer: u32,
    pub compute_unit_price_micro_lamports: u64,
    pub loaded_accounts_data_size_limit: u32,
    pub enabled: bool,
    pub dry_run_only: bool,
    pub rpc_url: String,
    pub address_lookup_tables: Vec<Pubkey>,
    pub live_send_min_profit_pct: f64,
    pub require_pre_send_simulation: bool,
    pub send_transport: SendTransport,
    pub jito_uuid: Option<String>,
    pub jito_tip_lamports: u64,
    pub two_hop_executor_program_id: Option<Pubkey>,
}

fn parse_bool_env(var: &str, default: bool) -> bool {
    std::env::var(var)
        .ok()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(default)
}

fn parse_send_transport_env(default_jito: bool) -> Result<SendTransport> {
    if let Ok(value) = std::env::var("SEND_TRANSPORT") {
        return match value.trim().to_ascii_lowercase().as_str() {
            "rpc" | "sendtransaction" => Ok(SendTransport::Rpc),
            "jito" | "jito_bundle" | "bundle" => Ok(SendTransport::JitoBundle),
            other => anyhow::bail!("Invalid SEND_TRANSPORT: {}", other),
        };
    }

    Ok(if default_jito {
        SendTransport::JitoBundle
    } else {
        SendTransport::Rpc
    })
}

impl ExecutionScope {
    pub fn from_env() -> Self {
        let skip_monitor_only_routes_when_live =
            parse_bool_env("SKIP_MONITOR_ONLY_ROUTES_WHEN_LIVE", true);
        Self {
            enabled: parse_bool_env("ENABLE_EXECUTION", false),
            dry_run_only: parse_bool_env("DRY_RUN_ONLY", true),
            skip_monitor_only_routes_when_live,
        }
    }

    pub fn live_send_enabled(&self) -> bool {
        self.enabled && !self.dry_run_only
    }
}

impl ExecutorConfig {
    pub fn from_env_with_rpc_default(default_rpc_url: &str) -> Result<Self> {
        let private_key_str = std::env::var("WALLET_PRIVATE_KEY")
            .context("WALLET_PRIVATE_KEY not found in environment")?;

        let keypair = if private_key_str.trim().starts_with('[') {
            let bytes: Vec<u8> = serde_json::from_str(&private_key_str)
                .context("Failed to parse private key as JSON array")?;

            if bytes.len() == 64 {
                Keypair::try_from(&bytes[..]).context("Failed to create keypair from 64 bytes")?
            } else if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Keypair::new_from_array(arr)
            } else {
                anyhow::bail!(
                    "Invalid keypair length: expected 32 or 64 bytes, got {}",
                    bytes.len()
                );
            }
        } else {
            let bytes = bs58::decode(&private_key_str)
                .into_vec()
                .context("Failed to decode base58 private key")?;

            if bytes.len() == 64 {
                Keypair::try_from(&bytes[..]).context("Failed to create keypair from base58")?
            } else if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Keypair::new_from_array(arr)
            } else {
                anyhow::bail!("Invalid base58 key length: {}", bytes.len());
            }
        };

        let max_slippage_bps = std::env::var("MAX_SLIPPAGE_BPS")
            .unwrap_or_else(|_| "100".to_string())
            .parse::<u64>()
            .context("Invalid MAX_SLIPPAGE_BPS")?;
        let compute_unit_limit = std::env::var("EXECUTION_COMPUTE_UNIT_LIMIT")
            .unwrap_or_else(|_| "1400000".to_string())
            .parse::<u32>()
            .context("Invalid EXECUTION_COMPUTE_UNIT_LIMIT")?;
        let compute_unit_limit_margin_bps =
            std::env::var("EXECUTION_COMPUTE_UNIT_LIMIT_MARGIN_BPS")
                .unwrap_or_else(|_| "1000".to_string())
                .parse::<u32>()
                .context("Invalid EXECUTION_COMPUTE_UNIT_LIMIT_MARGIN_BPS")?;
        let compute_unit_limit_min_buffer =
            std::env::var("EXECUTION_COMPUTE_UNIT_LIMIT_MIN_BUFFER")
                .unwrap_or_else(|_| "25000".to_string())
                .parse::<u32>()
                .context("Invalid EXECUTION_COMPUTE_UNIT_LIMIT_MIN_BUFFER")?;
        let compute_unit_price_micro_lamports =
            std::env::var("EXECUTION_COMPUTE_UNIT_PRICE_MICROLAMPORTS")
                .unwrap_or_else(|_| "1000".to_string())
                .parse::<u64>()
                .context("Invalid EXECUTION_COMPUTE_UNIT_PRICE_MICROLAMPORTS")?;
        let loaded_accounts_data_size_limit =
            std::env::var("EXECUTION_LOADED_ACCOUNTS_DATA_SIZE_LIMIT")
                .unwrap_or_else(|_| (64 * 1024 * 1024).to_string())
                .parse::<u32>()
                .context("Invalid EXECUTION_LOADED_ACCOUNTS_DATA_SIZE_LIMIT")?;

        let ExecutionScope {
            enabled,
            dry_run_only,
            ..
        } = ExecutionScope::from_env();

        let live_send_min_profit_pct = std::env::var("LIVE_SEND_MIN_PROFIT_PCT")
            .unwrap_or_else(|_| "0.0".to_string())
            .parse::<f64>()
            .context("Invalid LIVE_SEND_MIN_PROFIT_PCT")?;
        let require_pre_send_simulation = parse_bool_env("REQUIRE_PRE_SEND_SIMULATION", true);
        let jito_uuid = std::env::var("JITO_UUID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let send_transport = parse_send_transport_env(jito_uuid.is_some())?;
        let jito_tip_lamports = std::env::var("JITO_TIP_LAMPORTS")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u64>()
            .context("Invalid JITO_TIP_LAMPORTS")?;

        let rpc_url = std::env::var("RPC_HTTP_URL").unwrap_or_else(|_| default_rpc_url.to_string());
        let address_lookup_tables = std::env::var("ADDRESS_LOOKUP_TABLES")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(Pubkey::from_str)
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .transpose()
            .context("Invalid ADDRESS_LOOKUP_TABLES")?
            .unwrap_or_default();
        let two_hop_executor_program_id = std::env::var("TWO_HOP_EXECUTOR_PROGRAM_ID")
            .ok()
            .map(|value| Pubkey::from_str(value.trim()))
            .transpose()
            .context("Invalid TWO_HOP_EXECUTOR_PROGRAM_ID")?;

        Ok(Self {
            keypair,
            max_slippage_bps,
            compute_unit_limit,
            compute_unit_limit_margin_bps,
            compute_unit_limit_min_buffer,
            compute_unit_price_micro_lamports,
            loaded_accounts_data_size_limit,
            enabled,
            dry_run_only,
            rpc_url,
            address_lookup_tables,
            live_send_min_profit_pct,
            require_pre_send_simulation,
            send_transport,
            jito_uuid,
            jito_tip_lamports,
            two_hop_executor_program_id,
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::from_env_with_rpc_default("http://127.0.0.1:8899")
    }

    pub fn get_pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    pub fn get_pubkey_base58(&self) -> String {
        self.keypair.pubkey().to_string()
    }

    pub fn compute_unit_limit_for_send(&self, units_consumed: Option<u64>) -> u32 {
        let configured_limit = self.compute_unit_limit.max(1);
        let Some(units_consumed) = units_consumed else {
            return configured_limit;
        };
        let consumed = units_consumed.min(u32::MAX as u64) as u32;
        let percent_buffer = ((consumed as u64)
            .saturating_mul(self.compute_unit_limit_margin_bps as u64)
            / 10_000) as u32;
        let buffer = percent_buffer.max(self.compute_unit_limit_min_buffer);
        consumed.saturating_add(buffer).min(configured_limit).max(1)
    }

    pub fn estimated_send_gas_cost_sol(&self) -> f64 {
        const BASE_SIGNATURE_FEE_LAMPORTS: u64 = 5_000;
        let priority_fee_lamports = ((self.compute_unit_limit as u128)
            * (self.compute_unit_price_micro_lamports as u128))
            .saturating_add(999_999)
            / 1_000_000;
        (BASE_SIGNATURE_FEE_LAMPORTS as f64 + priority_fee_lamports as f64) / 1_000_000_000.0
    }

    pub fn require_pre_send_simulation(&self) -> bool {
        self.require_pre_send_simulation
    }

    pub fn calculate_slippage(&self, amount: u64) -> u64 {
        amount * (10000 - self.max_slippage_bps) / 10000
    }
}
