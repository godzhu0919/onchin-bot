use crate::{
    config::{Config, ProgramKind},
    executor::config::ExecutionScope,
    parser::{meteora, meteora_damm_v2, pump, pumpswap, raydium, whirlpool},
    rpc, strategy,
};
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
};

const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const PUMP_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

#[derive(Debug, Default)]
pub struct SubscriptionAccounts {
    pub pump_accounts: Vec<String>,
    pub pumpswap_pools: Vec<String>,
    pub raydium_accounts: Vec<String>,
    pub meteora_accounts: Vec<String>,
    pub whirlpool_accounts: Vec<String>,
    pub metadata: HashMap<String, AccountMetadata>,
}

#[derive(Debug, Clone)]
pub struct AccountMetadata {
    pub address: String,
    pub source: String,
    pub dex_id: Option<String>,
    pub program_kind_hint: Option<ProgramKind>,
    pub token_mint: Option<String>,
    pub base_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub label: String,
    pub price_usd: Option<f64>,
    pub liquidity_usd: Option<f64>,
    pub volume_h24: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManualMarketKind {
    Pump,
    PumpSwap,
    Raydium,
    Meteora,
    Whirlpool,
}

impl ManualMarketKind {
    fn label(self) -> &'static str {
        match self {
            ManualMarketKind::Pump => "pump",
            ManualMarketKind::PumpSwap => "pumpswap",
            ManualMarketKind::Raydium => "raydium",
            ManualMarketKind::Meteora => "meteora",
            ManualMarketKind::Whirlpool => "whirlpool",
        }
    }
}

#[derive(Debug, Clone)]
struct ManualMarketRecord {
    address: String,
    kind: ManualMarketKind,
    dex_id: String,
    program_kind_hint: Option<ProgramKind>,
    token_mint: Option<String>,
    base_mint: Option<String>,
    quote_mint: Option<String>,
    label: String,
}

pub async fn build_subscription_accounts(
    config: &Config,
    _execution_scope: &ExecutionScope,
) -> Result<SubscriptionAccounts> {
    let addresses = collect_static_market_addresses(config);
    if addresses.is_empty() {
        tracing::warn!("静态市场为空，请在 config.toml 里配置 manual_market_addresses");
        return Ok(SubscriptionAccounts::default());
    }

    tracing::info!("加载静态市场：账户数={}", addresses.len());
    build_manual_market_subscription_accounts_for_addresses(config, &addresses).await
}

fn collect_static_market_addresses(config: &Config) -> Vec<String> {
    let mut addresses = Vec::new();
    addresses.extend(config.discovery.manual_market_addresses.clone());
    addresses.extend(read_dynamic_market_addresses(
        &config.discovery.dynamic_market_addresses_path,
    ));
    addresses.extend(config.get_enabled_pump_accounts());
    addresses.extend(config.get_enabled_raydium_pools());
    addresses.extend(config.get_enabled_meteora_pools());
    addresses.extend(config.get_enabled_whirlpool_pools());

    if !config.discovery.excluded_market_addresses.is_empty() {
        let excluded = config
            .discovery
            .excluded_market_addresses
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        addresses.retain(|address| !excluded.contains(address));
    }

    dedup_strings(&mut addresses);
    addresses
}

fn read_dynamic_market_addresses(path: &str) -> Vec<String> {
    let path = path.trim();
    if path.is_empty() {
        return Vec::new();
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!("读取动态池子失败：文件={}，原因={}", path, error);
            return Vec::new();
        }
    };

    let addresses = content
        .lines()
        .filter_map(|line| {
            let value = line
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches(',')
                .trim_matches('"')
                .trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .collect::<Vec<_>>();

    if !addresses.is_empty() {
        tracing::info!("加载动态市场：文件={}，账户数={}", path, addresses.len());
    }

    addresses
}

async fn build_manual_market_subscription_accounts_for_addresses(
    config: &Config,
    addresses: &[String],
) -> Result<SubscriptionAccounts> {
    let owners = rpc::get_multiple_accounts_owners(&config.rpc.http_url, addresses)
        .await
        .context("fetch owners for static markets")?;
    let account_data = rpc::get_multiple_accounts_data(&config.rpc.http_url, addresses)
        .await
        .context("fetch account data for static markets")?;

    let mut records = Vec::new();
    for address in addresses {
        let Some(owner) = owners.get(address) else {
            tracing::warn!("跳过市场：{}，原因=缺少 owner", address);
            continue;
        };
        let Some(data) = account_data.get(address) else {
            tracing::warn!("跳过市场：{}，原因=缺少账户数据", address);
            continue;
        };
        match classify_manual_market_account(config, address, owner, data) {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {
                tracing::warn!("跳过市场：{}，原因=暂不支持的程序 {}", address, owner);
            }
            Err(error) => tracing::warn!("解析市场失败：{}，原因={}", address, error),
        }
    }

    if records.is_empty() {
        return Ok(SubscriptionAccounts::default());
    }

    let classified_count = records.len();
    let classified_counts = manual_market_kind_counts_summary(&records);
    let filtered_records = filter_manual_market_records(config, records);
    let selected_count = filtered_records.len();
    tracing::info!(
        "市场加载完成：配置={}，识别={}，可用={}，过滤={}，识别明细={}，可用明细={}",
        addresses.len(),
        classified_count,
        selected_count,
        classified_count.saturating_sub(selected_count),
        classified_counts,
        manual_market_kind_counts_summary(&filtered_records)
    );

    let mut out = SubscriptionAccounts::default();
    for record in filtered_records {
        out.metadata.insert(
            record.address.clone(),
            AccountMetadata {
                address: record.address.clone(),
                source: "static_market".to_string(),
                dex_id: Some(record.dex_id.clone()),
                program_kind_hint: record.program_kind_hint,
                token_mint: record.token_mint.clone(),
                base_mint: record.base_mint.clone(),
                quote_mint: record.quote_mint.clone(),
                label: record.label.clone(),
                price_usd: None,
                liquidity_usd: None,
                volume_h24: None,
            },
        );

        match record.kind {
            ManualMarketKind::Pump => out.pump_accounts.push(record.address),
            ManualMarketKind::PumpSwap => out.pumpswap_pools.push(record.address),
            ManualMarketKind::Raydium => out.raydium_accounts.push(record.address),
            ManualMarketKind::Meteora => out.meteora_accounts.push(record.address),
            ManualMarketKind::Whirlpool => out.whirlpool_accounts.push(record.address),
        }
    }

    dedup_strings(&mut out.pump_accounts);
    dedup_strings(&mut out.pumpswap_pools);
    dedup_strings(&mut out.raydium_accounts);
    dedup_strings(&mut out.meteora_accounts);
    dedup_strings(&mut out.whirlpool_accounts);
    Ok(out)
}

fn manual_market_kind_counts_summary(records: &[ManualMarketRecord]) -> String {
    [
        ManualMarketKind::Pump,
        ManualMarketKind::PumpSwap,
        ManualMarketKind::Raydium,
        ManualMarketKind::Meteora,
        ManualMarketKind::Whirlpool,
    ]
    .into_iter()
    .map(|kind| {
        let count = records.iter().filter(|record| record.kind == kind).count();
        format!("{}:{}", kind.label(), count)
    })
    .collect::<Vec<_>>()
    .join(",")
}

fn classify_manual_market_account(
    config: &Config,
    address: &str,
    owner: &str,
    data: &[u8],
) -> Result<Option<ManualMarketRecord>> {
    let short = &address[..address.len().min(8)];
    if let Some(kind) = config.resolve_program_kind_by_owner(owner) {
        return match kind {
            ProgramKind::Pumpfun => classify_pump(config, address, data, kind, short),
            ProgramKind::Pumpswap => classify_pumpswap(config, address, data, kind, short),
            ProgramKind::MeteoraDlmm => classify_meteora(config, address, data, Some(kind), short),
            ProgramKind::RaydiumClmm | ProgramKind::RaydiumAmmV4 | ProgramKind::RaydiumCpmm => {
                classify_raydium(config, address, data, Some(kind), short)
            }
            ProgramKind::Whirlpool => classify_whirlpool(config, address, data, Some(kind), short),
            ProgramKind::Humendfi | ProgramKind::Pancakeswap => Ok(None),
        };
    }

    match owner {
        PUMP_PROGRAM_ID => classify_pump(config, address, data, ProgramKind::Pumpfun, short),
        PUMP_AMM_PROGRAM_ID => {
            classify_pumpswap(config, address, data, ProgramKind::Pumpswap, short)
        }
        RAYDIUM_AMM_V4_PROGRAM_ID | RAYDIUM_CPMM_PROGRAM_ID | RAYDIUM_CLMM_PROGRAM_ID => {
            classify_raydium(config, address, data, None, short)
        }
        WHIRLPOOL_PROGRAM_ID => {
            classify_whirlpool(config, address, data, Some(ProgramKind::Whirlpool), short)
        }
        METEORA_DLMM_PROGRAM_ID => {
            classify_meteora(config, address, data, Some(ProgramKind::MeteoraDlmm), short)
        }
        meteora_damm_v2::METEORA_DAMM_V2_PROGRAM_ID => {
            classify_meteora(config, address, data, None, short)
        }
        _ => Ok(None),
    }
}

fn classify_pump(
    config: &Config,
    address: &str,
    data: &[u8],
    kind: ProgramKind,
    short: &str,
) -> Result<Option<ManualMarketRecord>> {
    let state = pump::parse_pump_state(data)?;
    Ok(Some(ManualMarketRecord {
        address: address.to_string(),
        kind: ManualMarketKind::Pump,
        dex_id: "pump".to_string(),
        program_kind_hint: Some(kind),
        token_mint: Some(state.token_mint.clone()),
        base_mint: Some(state.token_mint),
        quote_mint: Some(config.tokens.sol_mint.clone()),
        label: format!("static {} {}", config.program_label(kind), short),
    }))
}

fn classify_pumpswap(
    config: &Config,
    address: &str,
    data: &[u8],
    kind: ProgramKind,
    short: &str,
) -> Result<Option<ManualMarketRecord>> {
    let state = pumpswap::parse_pumpswap_pool(data, address)?;
    Ok(Some(ManualMarketRecord {
        address: address.to_string(),
        kind: ManualMarketKind::PumpSwap,
        dex_id: "pumpswap".to_string(),
        program_kind_hint: Some(kind),
        token_mint: state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint),
        base_mint: Some(state.base_mint.clone()),
        quote_mint: Some(state.quote_mint.clone()),
        label: format!("static {} {}", config.program_label(kind), short),
    }))
}

fn classify_raydium(
    config: &Config,
    address: &str,
    data: &[u8],
    kind_hint: Option<ProgramKind>,
    short: &str,
) -> Result<Option<ManualMarketRecord>> {
    let state = raydium::parse_raydium_state(data, address)?;
    let kind = kind_hint.or(match state.venue {
        crate::model::state::RaydiumVenue::AmmV4 => Some(ProgramKind::RaydiumAmmV4),
        crate::model::state::RaydiumVenue::Cpmm => Some(ProgramKind::RaydiumCpmm),
        crate::model::state::RaydiumVenue::Clmm => Some(ProgramKind::RaydiumClmm),
    });
    let label = kind
        .map(|kind| config.program_label(kind).to_string())
        .unwrap_or_else(|| state.venue.label().to_string());
    Ok(Some(ManualMarketRecord {
        address: address.to_string(),
        kind: ManualMarketKind::Raydium,
        dex_id: "raydium".to_string(),
        program_kind_hint: kind,
        token_mint: strategy::quote::raydium_traded_token(
            &state,
            &config.tokens.usdc_mint,
            &config.tokens.sol_mint,
        ),
        base_mint: Some(state.base_mint.clone()),
        quote_mint: Some(state.quote_mint.clone()),
        label: format!("static {} {}", label, short),
    }))
}

fn classify_meteora(
    config: &Config,
    address: &str,
    data: &[u8],
    kind_hint: Option<ProgramKind>,
    short: &str,
) -> Result<Option<ManualMarketRecord>> {
    let state = if kind_hint == Some(ProgramKind::MeteoraDlmm) {
        meteora::parse_meteora_state(data, address)?
    } else {
        meteora_damm_v2::parse_meteora_damm_v2_state(data, address)
            .or_else(|_| meteora::parse_meteora_state(data, address))?
    };
    Ok(Some(ManualMarketRecord {
        address: address.to_string(),
        kind: ManualMarketKind::Meteora,
        dex_id: "meteora".to_string(),
        program_kind_hint: kind_hint,
        token_mint: state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint),
        base_mint: Some(state.token_x_mint.clone()),
        quote_mint: Some(state.token_y_mint.clone()),
        label: format!("static Meteora {}", short),
    }))
}

fn classify_whirlpool(
    config: &Config,
    address: &str,
    data: &[u8],
    kind_hint: Option<ProgramKind>,
    short: &str,
) -> Result<Option<ManualMarketRecord>> {
    let state = whirlpool::parse_whirlpool_state(data, address)?;
    Ok(Some(ManualMarketRecord {
        address: address.to_string(),
        kind: ManualMarketKind::Whirlpool,
        dex_id: "whirlpool".to_string(),
        program_kind_hint: kind_hint,
        token_mint: state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint),
        base_mint: Some(state.token_mint_a.clone()),
        quote_mint: Some(state.token_mint_b.clone()),
        label: format!("static Whirlpool {}", short),
    }))
}

fn filter_manual_market_records(
    config: &Config,
    records: Vec<ManualMarketRecord>,
) -> Vec<ManualMarketRecord> {
    let excluded_tokens = config
        .discovery
        .excluded_target_token_mints
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let records = records
        .into_iter()
        .filter(|record| {
            record
                .token_mint
                .as_ref()
                .map(|mint| !excluded_tokens.contains(mint))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    if !config.discovery.require_routeable_pairs {
        return records;
    }

    let routeable_dexes = config
        .effective_routeable_dexes()
        .iter()
        .map(|dex| normalize_routeable_dex_id(&dex.to_ascii_lowercase()).to_string())
        .collect::<HashSet<_>>();
    if routeable_dexes.is_empty() {
        return records;
    }

    let min_routeable_dex_count = config.discovery.min_routeable_dex_count.max(1);
    let mut dexes_by_token: HashMap<String, HashSet<String>> = HashMap::new();
    for record in &records {
        let Some(token_mint) = &record.token_mint else {
            continue;
        };
        let dex = normalize_routeable_dex_id(&record.dex_id).to_string();
        if routeable_dexes.contains(&dex) {
            dexes_by_token
                .entry(token_mint.clone())
                .or_default()
                .insert(dex);
        }
    }

    records
        .into_iter()
        .filter(|record| {
            let Some(token_mint) = &record.token_mint else {
                return false;
            };
            dexes_by_token
                .get(token_mint)
                .map(|dexes| dexes.len() >= min_routeable_dex_count)
                .unwrap_or(false)
        })
        .collect()
}

fn normalize_routeable_dex_id(dex_id: &str) -> &str {
    match dex_id {
        "orca" | "whirlpool" => "whirlpool",
        "pumpfun" | "pump" => "pump",
        _ => dex_id,
    }
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}
