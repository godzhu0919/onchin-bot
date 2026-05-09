use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::env;
use std::str::FromStr;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const DEXSCREENER_TOKEN_PAIRS_URL: &str = "https://api.dexscreener.com/token-pairs/v1/solana";
const DEXSCREENER_LATEST_PROFILES_URL: &str =
    "https://api.dexscreener.com/token-profiles/latest/v1";
const DEXSCREENER_TOKEN_BOOSTS_LATEST_URL: &str =
    "https://api.dexscreener.com/token-boosts/latest/v1";
const DEXSCREENER_TOKEN_BOOSTS_TOP_URL: &str = "https://api.dexscreener.com/token-boosts/top/v1";

#[derive(Debug, Deserialize)]
struct DexScreenerProfile {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "tokenAddress")]
    token_address: String,
}

#[derive(Debug, Deserialize)]
struct DexScreenerPair {
    #[serde(rename = "dexId")]
    dex_id: String,
    #[serde(rename = "pairAddress")]
    pair_address: String,
    labels: Option<Vec<String>>,
    #[serde(rename = "priceUsd")]
    price_usd: Option<String>,
    #[serde(rename = "baseToken")]
    base_token: DexScreenerToken,
    #[serde(rename = "quoteToken")]
    quote_token: DexScreenerToken,
    liquidity: Option<DexScreenerLiquidity>,
    volume: Option<DexScreenerVolume>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerToken {
    address: String,
    symbol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerVolume {
    h24: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: RpcResult,
}

#[derive(Debug, Deserialize)]
struct RpcResult {
    value: Vec<Option<RpcAccount>>,
}

#[derive(Debug, Deserialize)]
struct RpcAccount {
    owner: String,
}

#[derive(Debug, Clone)]
struct PairSummary {
    address: String,
    symbol: String,
    price_usd: f64,
    liquidity_usd: f64,
    volume_h24: f64,
}

#[derive(Debug)]
struct Candidate {
    mint: String,
    symbol: String,
    direction: &'static str,
    gross_pct: f64,
    net_pct: f64,
    pumpswap: PairSummary,
    meteora: PairSummary,
    bitmap_extension: String,
}

fn stable_dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> anyhow::Result<T> {
    Ok(reqwest::Client::new()
        .get(url)
        .header("User-Agent", "codex-pumpswap-meteora-screener/1.0")
        .send()
        .await?
        .error_for_status()?
        .json::<T>()
        .await?)
}

async fn fetch_token_pairs(mint: &str) -> anyhow::Result<Vec<DexScreenerPair>> {
    fetch_json(&format!("{DEXSCREENER_TOKEN_PAIRS_URL}/{mint}")).await
}

async fn fetch_profile_tokens(url: &str) -> anyhow::Result<Vec<String>> {
    let profiles: Vec<DexScreenerProfile> = fetch_json(url).await?;
    Ok(profiles
        .into_iter()
        .filter(|item| item.chain_id == "solana")
        .map(|item| item.token_address)
        .collect())
}

async fn get_multiple_account_owners(
    rpc_url: &str,
    accounts: &[String],
) -> anyhow::Result<HashMap<String, String>> {
    let client = reqwest::Client::new();
    let mut owners = HashMap::new();
    for chunk in accounts.chunks(100) {
        let response = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getMultipleAccounts",
                "params": [chunk, {"encoding": "base64", "commitment": "processed"}]
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<RpcResponse>()
            .await?;
        for (account, value) in chunk.iter().zip(response.result.value.into_iter()) {
            if let Some(value) = value {
                owners.insert(account.clone(), value.owner);
            }
        }
    }
    Ok(owners)
}

fn quote_mint_supported(pair: &DexScreenerPair) -> bool {
    pair.base_token.address == SOL_MINT
        || pair.quote_token.address == SOL_MINT
        || pair.base_token.address == USDC_MINT
        || pair.quote_token.address == USDC_MINT
}

fn top_pair(pairs: &[DexScreenerPair], dex: &str, require_dlmm: bool) -> Option<PairSummary> {
    pairs
        .iter()
        .filter(|pair| pair.dex_id.eq_ignore_ascii_case(dex))
        .filter(|pair| quote_mint_supported(pair))
        .filter(|pair| {
            if !require_dlmm {
                return true;
            }
            pair.labels
                .as_ref()
                .map(|labels| {
                    labels
                        .iter()
                        .any(|label| label.eq_ignore_ascii_case("dlmm"))
                })
                .unwrap_or(false)
        })
        .filter_map(|pair| {
            let price_usd = pair.price_usd.as_deref()?.parse::<f64>().ok()?;
            let liquidity_usd = pair
                .liquidity
                .as_ref()
                .and_then(|item| item.usd)
                .unwrap_or(0.0);
            let volume_h24 = pair
                .volume
                .as_ref()
                .and_then(|item| item.h24)
                .unwrap_or(0.0);
            let symbol =
                if pair.quote_token.address == SOL_MINT || pair.quote_token.address == USDC_MINT {
                    pair.base_token
                        .symbol
                        .clone()
                        .unwrap_or_else(|| "?".to_string())
                } else {
                    pair.quote_token
                        .symbol
                        .clone()
                        .unwrap_or_else(|| "?".to_string())
                };
            Some(PairSummary {
                address: pair.pair_address.clone(),
                symbol,
                price_usd,
                liquidity_usd,
                volume_h24,
            })
        })
        .max_by(|left, right| {
            left.liquidity_usd
                .partial_cmp(&right.liquidity_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.volume_h24
                        .partial_cmp(&right.volume_h24)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
}

fn bitmap_extension_address(pool: &str) -> anyhow::Result<Pubkey> {
    let pool = Pubkey::from_str(pool)?;
    let program = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID)?;
    Ok(Pubkey::find_program_address(&[b"bitmap", pool.as_ref()], &program).0)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_string());
    let scan_limit = env_usize("SCAN_LIMIT", 300);
    let min_liquidity_usd = env_f64("MIN_LIQUIDITY_USD", 5_000.0);
    let min_volume_h24 = env_f64("MIN_VOLUME_H24", 10_000.0);
    let print_limit = env_usize("PRINT_LIMIT", 20);

    let mut mints = Vec::new();
    mints.extend(fetch_profile_tokens(DEXSCREENER_LATEST_PROFILES_URL).await?);
    mints.extend(fetch_profile_tokens(DEXSCREENER_TOKEN_BOOSTS_LATEST_URL).await?);
    mints.extend(fetch_profile_tokens(DEXSCREENER_TOKEN_BOOSTS_TOP_URL).await?);
    let mints = stable_dedup(mints);

    let mut raw_candidates = Vec::new();
    for mint in mints.into_iter().take(scan_limit) {
        let Ok(pairs) = fetch_token_pairs(&mint).await else {
            continue;
        };
        let Some(pumpswap) = top_pair(&pairs, "pumpswap", false) else {
            continue;
        };
        let Some(meteora) = top_pair(&pairs, "meteora", true) else {
            continue;
        };
        if pumpswap.liquidity_usd < min_liquidity_usd || meteora.liquidity_usd < min_liquidity_usd {
            continue;
        }
        if pumpswap.volume_h24 < min_volume_h24 || meteora.volume_h24 < min_volume_h24 {
            continue;
        }
        let low = pumpswap.price_usd.min(meteora.price_usd);
        let high = pumpswap.price_usd.max(meteora.price_usd);
        if low <= 0.0 || !low.is_finite() || !high.is_finite() {
            continue;
        }
        let gross_pct = ((high - low) / low) * 100.0;
        let net_pct = gross_pct - (1.25 + 0.50 + 0.50);
        let direction = if pumpswap.price_usd < meteora.price_usd {
            "PumpSwap->Meteora"
        } else {
            "Meteora->PumpSwap"
        };
        let bitmap_extension = bitmap_extension_address(&meteora.address)?.to_string();
        raw_candidates.push((
            mint,
            pumpswap,
            meteora,
            gross_pct,
            net_pct,
            direction,
            bitmap_extension,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let owners = get_multiple_account_owners(
        &rpc_url,
        &raw_candidates
            .iter()
            .map(|item| item.6.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    let mut candidates = Vec::new();
    for (mint, pumpswap, meteora, gross_pct, net_pct, direction, bitmap_extension) in raw_candidates
    {
        if owners
            .get(&bitmap_extension)
            .is_some_and(|owner| owner == METEORA_DLMM_PROGRAM_ID)
        {
            candidates.push(Candidate {
                mint,
                symbol: pumpswap.symbol.clone(),
                direction,
                gross_pct,
                net_pct,
                pumpswap,
                meteora,
                bitmap_extension,
            });
        }
    }

    candidates.sort_by(|left, right| {
        right
            .net_pct
            .partial_cmp(&left.net_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for candidate in candidates.iter().take(print_limit) {
        println!(
            "{} {} net~{:.2}% gross~{:.2}% dir={}",
            candidate.symbol,
            candidate.mint,
            candidate.net_pct,
            candidate.gross_pct,
            candidate.direction
        );
        println!(
            "  pumpswap {} liq~{:.0} vol~{:.0}",
            candidate.pumpswap.address,
            candidate.pumpswap.liquidity_usd,
            candidate.pumpswap.volume_h24
        );
        println!(
            "  meteora  {} liq~{:.0} vol~{:.0} bitmap={}",
            candidate.meteora.address,
            candidate.meteora.liquidity_usd,
            candidate.meteora.volume_h24,
            candidate.bitmap_extension
        );
    }

    Ok(())
}
