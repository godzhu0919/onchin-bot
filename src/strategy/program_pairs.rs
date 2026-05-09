use crate::{
    config::{Config, ProgramKind},
    market_universe::AccountMetadata,
    model::state::{
        MeteoraState, PumpState, PumpSwapState, RaydiumState, RaydiumVenue, WhirlpoolState,
    },
    strategy::quote,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketSource {
    LiveState,
    ExternalSnapshot,
}

#[derive(Debug, Clone)]
pub struct ProgramMarket {
    pub address: String,
    pub token_mint: String,
    pub quote_mint: String,
    pub kind: ProgramKind,
    pub label: String,
    pub price_quote: f64,
    pub price_usdc: f64,
    pub liquidity_usd: Option<f64>,
    pub source: MarketSource,
}

#[derive(Debug, Clone)]
pub struct ProgramPairPnlBreakdown {
    pub gross_profit_usdc: f64,
    pub slippage_buffer_usdc: f64,
    pub gas_cost_usdc: f64,
    pub net_profit_usdc: f64,
}

#[derive(Debug, Clone)]
pub struct ProgramPairSizeEvaluation {
    pub trade_size_quote: f64,
    pub trade_size_usdc: f64,
    pub gross_profit_quote: f64,
    pub gas_cost_quote: f64,
    pub net_profit_quote: f64,
    pub gross_profit_pct: f64,
    pub net_profit_pct: f64,
    pub pnl_usdc: ProgramPairPnlBreakdown,
}

#[derive(Debug, Clone)]
pub struct ProgramPairExactSearch {
    pub preferred_trade_size_quote: f64,
    pub requested_size_count: usize,
    pub successful_size_count: usize,
    pub evaluations: Vec<ProgramPairSizeEvaluation>,
}

#[derive(Debug, Clone)]
pub struct ProgramPairGateState {
    pub base_threshold_pct: f64,
    pub dynamic_adjustment_pct: f64,
    pub effective_threshold_pct: f64,
    pub gate_profit_pct: f64,
}

#[derive(Debug, Clone)]
pub struct ProgramPairCandidate {
    pub token_mint: String,
    pub quote_mint: String,
    pub pricing_mode: ProgramPairPricingMode,
    pub buy_market: ProgramMarket,
    pub sell_market: ProgramMarket,
    pub trade_size_quote: f64,
    pub gross_profit_quote: f64,
    pub gas_cost_quote: f64,
    pub net_profit_quote: f64,
    pub gross_profit_pct: f64,
    pub net_profit_pct: f64,
    pub selected_pnl_usdc: Option<ProgramPairPnlBreakdown>,
    pub exact_search: Option<ProgramPairExactSearch>,
    pub gate: Option<ProgramPairGateState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramPairPricingMode {
    SpotHeuristic,
    ExactQuote,
}

pub fn directional_candidates_for_tokens(
    config: &Config,
    metadata: &HashMap<String, AccountMetadata>,
    pump_state: &HashMap<String, PumpState>,
    pumpswap_state: &HashMap<String, PumpSwapState>,
    raydium_state: &HashMap<String, RaydiumState>,
    meteora_state: &HashMap<String, MeteoraState>,
    whirlpool_state: &HashMap<String, WhirlpoolState>,
    token_filter: &HashSet<String>,
) -> Vec<ProgramPairCandidate> {
    let markets = build_markets(
        config,
        metadata,
        pump_state,
        pumpswap_state,
        raydium_state,
        meteora_state,
        whirlpool_state,
    );
    let mut candidates = Vec::new();
    for (token_mint, token_markets) in markets {
        if !token_filter.is_empty() && !token_filter.contains(&token_mint) {
            continue;
        }
        candidates.extend(directional_candidates_for_market_set(config, token_markets));
    }
    candidates.sort_by(|left, right| {
        right
            .net_profit_pct
            .partial_cmp(&left.net_profit_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.token_mint.cmp(&right.token_mint))
            .then_with(|| left.buy_market.address.cmp(&right.buy_market.address))
            .then_with(|| left.sell_market.address.cmp(&right.sell_market.address))
    });
    candidates
}

pub fn routeable_tokens(
    config: &Config,
    metadata: &HashMap<String, AccountMetadata>,
    pump_state: &HashMap<String, PumpState>,
    pumpswap_state: &HashMap<String, PumpSwapState>,
    raydium_state: &HashMap<String, RaydiumState>,
    meteora_state: &HashMap<String, MeteoraState>,
    whirlpool_state: &HashMap<String, WhirlpoolState>,
) -> Vec<String> {
    let markets = build_markets(
        config,
        metadata,
        pump_state,
        pumpswap_state,
        raydium_state,
        meteora_state,
        whirlpool_state,
    );
    let mut tokens = markets
        .into_iter()
        .filter_map(|(token_mint, token_markets)| {
            has_routeable_program_pair(&token_markets).then_some(token_mint)
        })
        .collect::<Vec<_>>();
    tokens.sort();
    tokens
}

pub fn best_candidates_for_tokens(
    config: &Config,
    metadata: &HashMap<String, AccountMetadata>,
    pump_state: &HashMap<String, PumpState>,
    pumpswap_state: &HashMap<String, PumpSwapState>,
    raydium_state: &HashMap<String, RaydiumState>,
    meteora_state: &HashMap<String, MeteoraState>,
    whirlpool_state: &HashMap<String, WhirlpoolState>,
    token_filter: &HashSet<String>,
) -> Vec<ProgramPairCandidate> {
    let markets = build_markets(
        config,
        metadata,
        pump_state,
        pumpswap_state,
        raydium_state,
        meteora_state,
        whirlpool_state,
    );
    let mut candidates = Vec::new();
    for (token_mint, token_markets) in markets {
        if !token_filter.is_empty() && !token_filter.contains(&token_mint) {
            continue;
        }
        if let Some(candidate) = best_candidate_for_market_set(config, token_markets) {
            candidates.push(candidate);
        }
    }
    candidates.sort_by(|left, right| {
        right
            .net_profit_pct
            .partial_cmp(&left.net_profit_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.token_mint.cmp(&right.token_mint))
    });
    candidates
}

fn build_markets(
    config: &Config,
    metadata: &HashMap<String, AccountMetadata>,
    pump_state: &HashMap<String, PumpState>,
    pumpswap_state: &HashMap<String, PumpSwapState>,
    raydium_state: &HashMap<String, RaydiumState>,
    meteora_state: &HashMap<String, MeteoraState>,
    whirlpool_state: &HashMap<String, WhirlpoolState>,
) -> HashMap<String, Vec<ProgramMarket>> {
    let enabled = config
        .enabled_program_kinds()
        .into_iter()
        .collect::<HashSet<_>>();
    let mut markets_by_token: HashMap<String, Vec<ProgramMarket>> = HashMap::new();

    if enabled.contains(&ProgramKind::Pumpfun) {
        for (address, state) in pump_state {
            if let Some(market) = pump_market(config, address, state) {
                markets_by_token
                    .entry(market.token_mint.clone())
                    .or_default()
                    .push(market);
            }
        }
    }

    if enabled.contains(&ProgramKind::Pumpswap) {
        for (address, state) in pumpswap_state {
            if let Some(market) = pumpswap_market(config, address, state) {
                markets_by_token
                    .entry(market.token_mint.clone())
                    .or_default()
                    .push(market);
            }
        }
    }

    if enabled.iter().any(|kind| {
        matches!(
            kind,
            ProgramKind::RaydiumAmmV4 | ProgramKind::RaydiumCpmm | ProgramKind::RaydiumClmm
        )
    }) {
        for (address, state) in raydium_state {
            if let Some(market) = raydium_market(config, address, state) {
                markets_by_token
                    .entry(market.token_mint.clone())
                    .or_default()
                    .push(market);
            }
        }
    }

    if enabled.contains(&ProgramKind::MeteoraDlmm) {
        for (address, state) in meteora_state {
            if let Some(market) = meteora_market(config, address, state) {
                markets_by_token
                    .entry(market.token_mint.clone())
                    .or_default()
                    .push(market);
            }
        }
    }

    if enabled.contains(&ProgramKind::Whirlpool) {
        for (address, state) in whirlpool_state {
            if let Some(market) = whirlpool_market(config, address, state) {
                markets_by_token
                    .entry(market.token_mint.clone())
                    .or_default()
                    .push(market);
            }
        }
    }

    let live_addresses = pump_state
        .keys()
        .chain(pumpswap_state.keys())
        .chain(raydium_state.keys())
        .chain(meteora_state.keys())
        .chain(whirlpool_state.keys())
        .cloned()
        .collect::<HashSet<_>>();
    if enabled.contains(&ProgramKind::Pancakeswap) {
        for market in external_snapshot_markets(config, metadata, &live_addresses) {
            markets_by_token
                .entry(market.token_mint.clone())
                .or_default()
                .push(market);
        }
    }

    markets_by_token
}

fn has_routeable_program_pair(markets: &[ProgramMarket]) -> bool {
    for (index, left) in markets.iter().enumerate() {
        for right in markets.iter().skip(index + 1) {
            if left.kind != right.kind && left.quote_mint == right.quote_mint {
                return true;
            }
        }
    }
    false
}

fn best_candidate_for_market_set(
    config: &Config,
    markets: Vec<ProgramMarket>,
) -> Option<ProgramPairCandidate> {
    let mut best = None;
    for candidate in directional_candidates_for_market_set(config, markets) {
        let replace = best
            .as_ref()
            .map(|current: &ProgramPairCandidate| candidate.net_profit_pct > current.net_profit_pct)
            .unwrap_or(true);
        if replace {
            best = Some(candidate);
        }
    }
    best
}

fn directional_candidates_for_market_set(
    config: &Config,
    markets: Vec<ProgramMarket>,
) -> Vec<ProgramPairCandidate> {
    let mut candidates = Vec::new();
    for buy_market in &markets {
        for sell_market in &markets {
            if buy_market.address == sell_market.address {
                continue;
            }
            if let Some(candidate) =
                evaluate_directional_program_pair(config, buy_market.clone(), sell_market.clone())
            {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn evaluate_directional_program_pair(
    config: &Config,
    buy_market: ProgramMarket,
    sell_market: ProgramMarket,
) -> Option<ProgramPairCandidate> {
    let same_kind_meteora_pair = buy_market.kind == ProgramKind::MeteoraDlmm
        && sell_market.kind == ProgramKind::MeteoraDlmm
        && config.program_by_kind(ProgramKind::MeteoraDlmm).is_some();
    if (buy_market.kind == sell_market.kind && !same_kind_meteora_pair)
        || buy_market.token_mint != sell_market.token_mint
        || buy_market.quote_mint != sell_market.quote_mint
        || !buy_market.price_quote.is_finite()
        || !sell_market.price_quote.is_finite()
        || buy_market.price_quote <= 0.0
        || sell_market.price_quote <= 0.0
    {
        return None;
    }

    let gross_profit_pct =
        ((sell_market.price_quote - buy_market.price_quote) / buy_market.price_quote) * 100.0;
    if !gross_profit_pct.is_finite() || gross_profit_pct <= 0.0 {
        return None;
    }

    let mut best: Option<ProgramPairCandidate> = None;
    for trade_size_quote in
        configured_program_pair_trade_sizes_quote(config, &buy_market.quote_mint)
    {
        let gross_profit_quote = trade_size_quote * gross_profit_pct / 100.0;
        let slippage_buffer_quote = trade_size_quote * quote::SLIPPAGE_BUFFER_BPS / 10_000.0;
        let gas_cost_quote = quote_native_gas_cost(config, &buy_market.quote_mint)?;
        let net_profit_quote = gross_profit_quote - slippage_buffer_quote - gas_cost_quote;
        let net_profit_pct = (net_profit_quote / trade_size_quote) * 100.0;
        let gross_profit_usdc = quote_to_usdc(config, &buy_market.quote_mint, gross_profit_quote)?;
        let slippage_buffer_usdc =
            quote_to_usdc(config, &buy_market.quote_mint, slippage_buffer_quote)?;
        let gas_cost_usdc = quote_to_usdc(config, &buy_market.quote_mint, gas_cost_quote)?;
        let net_profit_usdc = quote_to_usdc(config, &buy_market.quote_mint, net_profit_quote)?;

        let candidate = ProgramPairCandidate {
            token_mint: buy_market.token_mint.clone(),
            quote_mint: buy_market.quote_mint.clone(),
            pricing_mode: ProgramPairPricingMode::SpotHeuristic,
            buy_market: buy_market.clone(),
            sell_market: sell_market.clone(),
            trade_size_quote,
            gross_profit_quote,
            gas_cost_quote,
            net_profit_quote,
            gross_profit_pct,
            net_profit_pct,
            selected_pnl_usdc: Some(ProgramPairPnlBreakdown {
                gross_profit_usdc,
                slippage_buffer_usdc,
                gas_cost_usdc,
                net_profit_usdc,
            }),
            exact_search: None,
            gate: None,
        };
        let replace = best
            .as_ref()
            .map(|current| candidate.net_profit_pct > current.net_profit_pct)
            .unwrap_or(true);
        if replace {
            best = Some(candidate);
        }
    }

    best
}

pub(crate) fn best_size_search_trade_sizes_quote(
    config: &Config,
    quote_mint: &str,
    preferred_trade_size_quote: f64,
) -> Vec<f64> {
    let sizes = configured_program_pair_trade_sizes_quote(config, quote_mint);
    let max_samples = config.strategy.program_pair_best_size_search_samples.max(1);
    if sizes.len() <= max_samples {
        return sizes;
    }

    let preferred_index = sizes
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_gap = (**left - preferred_trade_size_quote).abs();
            let right_gap = (**right - preferred_trade_size_quote).abs();
            left_gap
                .partial_cmp(&right_gap)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);

    let mut selected_indices = vec![preferred_index];
    let mut distance = 1usize;
    while selected_indices.len() < max_samples && distance < sizes.len() {
        if let Some(index) = preferred_index.checked_sub(distance) {
            selected_indices.push(index);
            if selected_indices.len() >= max_samples {
                break;
            }
        }
        let index = preferred_index + distance;
        if index < sizes.len() {
            selected_indices.push(index);
        }
        distance += 1;
    }

    if selected_indices.len() < max_samples {
        for index in 0..sizes.len() {
            if !selected_indices.contains(&index) {
                selected_indices.push(index);
            }
            if selected_indices.len() >= max_samples {
                break;
            }
        }
    }

    selected_indices.sort_unstable();
    selected_indices.dedup();
    selected_indices
        .into_iter()
        .map(|index| sizes[index])
        .collect()
}

fn pump_market(config: &Config, address: &str, state: &PumpState) -> Option<ProgramMarket> {
    if state.sol_reserve == 0 || state.token_reserve == 0 {
        return None;
    }
    let token_amount = state.token_reserve as f64 / 1_000_000.0;
    if token_amount <= 0.0 {
        return None;
    }
    let sol_amount = state.sol_reserve as f64 / 1_000_000_000.0;
    let price_usdc = (sol_amount / token_amount) * config.strategy.sol_usdc_price;
    valid_price_market(
        address,
        &state.token_mint,
        &config.tokens.sol_mint,
        ProgramKind::Pumpfun,
        config.program_label(ProgramKind::Pumpfun),
        sol_amount / token_amount,
        price_usdc,
        Some(sol_amount * config.strategy.sol_usdc_price),
        MarketSource::LiveState,
    )
}

fn raydium_market(config: &Config, address: &str, state: &RaydiumState) -> Option<ProgramMarket> {
    let kind = match state.venue {
        RaydiumVenue::AmmV4 => ProgramKind::RaydiumAmmV4,
        RaydiumVenue::Cpmm => ProgramKind::RaydiumCpmm,
        RaydiumVenue::Clmm => ProgramKind::RaydiumClmm,
    };
    config.program_by_kind(kind)?;

    let token_mint =
        quote::raydium_traded_token(state, &config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let price_quote_per_base = state.calculate_price();
    if !price_quote_per_base.is_finite() || price_quote_per_base <= 0.0 {
        return None;
    }

    let (quote_mint, price_in_quote) = if state.base_mint == token_mint {
        (state.quote_mint.clone(), price_quote_per_base)
    } else if state.quote_mint == token_mint {
        (state.base_mint.clone(), 1.0 / price_quote_per_base)
    } else {
        return None;
    };
    let price_usdc = quote_to_usdc(config, &quote_mint, price_in_quote)?;

    valid_price_market(
        address,
        &token_mint,
        &quote_mint,
        kind,
        config.program_label(kind),
        price_in_quote,
        price_usdc,
        liquidity_usd_for_raydium(config, state, &quote_mint),
        MarketSource::LiveState,
    )
}

fn pumpswap_market(config: &Config, address: &str, state: &PumpSwapState) -> Option<ProgramMarket> {
    let token_mint = state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let (quote_mint, quote_reserve, token_reserve) = if state.base_mint == token_mint {
        (
            state.quote_mint.clone(),
            state.quote_reserve,
            state.base_reserve,
        )
    } else if state.quote_mint == token_mint {
        (
            state.base_mint.clone(),
            state.base_reserve,
            state.quote_reserve,
        )
    } else {
        return None;
    };
    if token_reserve == 0 || quote_reserve == 0 {
        return None;
    }

    let token_decimals = default_decimals(&token_mint);
    let quote_decimals = default_decimals(&quote_mint);
    let token_amount = token_reserve as f64 / 10_f64.powi(token_decimals);
    let quote_amount = quote_reserve as f64 / 10_f64.powi(quote_decimals);
    if token_amount <= 0.0 || quote_amount <= 0.0 {
        return None;
    }

    let price_in_quote = quote_amount / token_amount;
    let price_usdc = quote_to_usdc(config, &quote_mint, price_in_quote)?;
    let liquidity_usd = quote_to_usdc(config, &quote_mint, quote_amount);

    valid_price_market(
        address,
        &token_mint,
        &quote_mint,
        ProgramKind::Pumpswap,
        config.program_label(ProgramKind::Pumpswap),
        price_in_quote,
        price_usdc,
        liquidity_usd,
        MarketSource::LiveState,
    )
}

fn meteora_market(config: &Config, address: &str, state: &MeteoraState) -> Option<ProgramMarket> {
    let token_mint = state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let quote_mint = state.quote_mint(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    if state.is_damm_v2() {
        let (quote_reserve, token_reserve) = if state.token_x_mint == quote_mint {
            (state.token_x_amount, state.token_y_amount)
        } else if state.token_y_mint == quote_mint {
            (state.token_y_amount, state.token_x_amount)
        } else {
            return None;
        };
        if token_reserve == 0 || quote_reserve == 0 {
            return None;
        }

        let token_decimals = default_decimals(&token_mint);
        let quote_decimals = default_decimals(&quote_mint);
        let token_amount = token_reserve as f64 / 10_f64.powi(token_decimals);
        let quote_amount = quote_reserve as f64 / 10_f64.powi(quote_decimals);
        if token_amount <= 0.0 || quote_amount <= 0.0 {
            return None;
        }

        let price_in_quote = quote_amount / token_amount;
        let price_usdc = quote_to_usdc(config, &quote_mint, price_in_quote)?;
        let liquidity_usd = quote_to_usdc(config, &quote_mint, quote_amount);

        return valid_price_market(
            address,
            &token_mint,
            &quote_mint,
            ProgramKind::MeteoraDlmm,
            config.program_label(ProgramKind::MeteoraDlmm),
            price_in_quote,
            price_usdc,
            liquidity_usd,
            MarketSource::LiveState,
        );
    }

    let raw_price_in_quote =
        state.token_price_in_quote(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let decimal_adjustment =
        10_f64.powi(default_decimals(&token_mint) - default_decimals(&quote_mint));
    let price_usdc = quote_to_usdc(config, &quote_mint, raw_price_in_quote * decimal_adjustment)?;
    let liquidity_usd = if quote_mint == config.tokens.usdc_mint {
        Some(state.token_y_amount as f64 / 1_000_000.0)
    } else if quote_mint == config.tokens.sol_mint {
        Some((state.token_y_amount as f64 / 1_000_000_000.0) * config.strategy.sol_usdc_price)
    } else {
        None
    };

    valid_price_market(
        address,
        &token_mint,
        &quote_mint,
        ProgramKind::MeteoraDlmm,
        config.program_label(ProgramKind::MeteoraDlmm),
        raw_price_in_quote * decimal_adjustment,
        price_usdc,
        liquidity_usd,
        MarketSource::LiveState,
    )
}

fn whirlpool_market(
    config: &Config,
    address: &str,
    state: &WhirlpoolState,
) -> Option<ProgramMarket> {
    let token_mint = state.traded_token(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let quote_mint = state.quote_mint(&config.tokens.usdc_mint, &config.tokens.sol_mint)?;
    let price_quote_per_a = state.calculate_price(
        default_decimals(&state.token_mint_a) as u8,
        default_decimals(&state.token_mint_b) as u8,
    );
    if !price_quote_per_a.is_finite() || price_quote_per_a <= 0.0 {
        return None;
    }

    let price_in_quote = if state.token_mint_a == token_mint {
        price_quote_per_a
    } else if state.token_mint_b == token_mint {
        1.0 / price_quote_per_a
    } else {
        return None;
    };
    let price_usdc = quote_to_usdc(config, &quote_mint, price_in_quote)?;

    valid_price_market(
        address,
        &token_mint,
        &quote_mint,
        ProgramKind::Whirlpool,
        config.program_label(ProgramKind::Whirlpool),
        price_in_quote,
        price_usdc,
        None,
        MarketSource::LiveState,
    )
}

fn external_snapshot_markets(
    config: &Config,
    metadata: &HashMap<String, AccountMetadata>,
    live_addresses: &HashSet<String>,
) -> Vec<ProgramMarket> {
    metadata
        .values()
        .filter(|meta| !live_addresses.contains(&meta.address))
        .filter_map(|meta| {
            let kind = meta.program_kind_hint?;
            if kind != ProgramKind::Pancakeswap {
                return None;
            }
            config.program_by_kind(kind)?;
            let token_mint = meta.token_mint.as_ref()?;
            let quote_mint = meta.quote_mint.as_ref()?;
            let price_usdc = meta.price_usd?;
            valid_price_market(
                &meta.address,
                token_mint,
                quote_mint,
                kind,
                config.program_label(kind),
                quote_native_price_from_usd(config, quote_mint, price_usdc)?,
                price_usdc,
                meta.liquidity_usd,
                MarketSource::ExternalSnapshot,
            )
        })
        .collect()
}

fn valid_price_market(
    address: &str,
    token_mint: &str,
    quote_mint: &str,
    kind: ProgramKind,
    label: &str,
    price_quote: f64,
    price_usdc: f64,
    liquidity_usd: Option<f64>,
    source: MarketSource,
) -> Option<ProgramMarket> {
    (price_quote.is_finite() && price_quote > 0.0 && price_usdc.is_finite() && price_usdc > 0.0)
        .then(|| ProgramMarket {
            address: address.to_string(),
            token_mint: token_mint.to_string(),
            quote_mint: quote_mint.to_string(),
            kind,
            label: label.to_string(),
            price_quote,
            price_usdc,
            liquidity_usd,
            source,
        })
}

fn quote_to_usdc(config: &Config, quote_mint: &str, amount_in_quote: f64) -> Option<f64> {
    if quote_mint == config.tokens.usdc_mint {
        Some(amount_in_quote)
    } else if quote_mint == config.tokens.sol_mint {
        Some(amount_in_quote * config.strategy.sol_usdc_price)
    } else {
        None
    }
}

fn quote_native_price_from_usd(config: &Config, quote_mint: &str, price_usd: f64) -> Option<f64> {
    if quote_mint == config.tokens.usdc_mint {
        Some(price_usd)
    } else if quote_mint == config.tokens.sol_mint {
        Some(price_usd / config.strategy.sol_usdc_price)
    } else {
        None
    }
}

pub(crate) fn configured_program_pair_trade_sizes_quote(
    config: &Config,
    quote_mint: &str,
) -> Vec<f64> {
    let mut sizes = config
        .strategy
        .program_pair_trade_sizes_sol
        .iter()
        .copied()
        .filter_map(|size| {
            if !size.is_finite() || size <= 0.0 {
                return None;
            }
            let trade_size_usd = size * config.strategy.sol_usdc_price;
            quote_native_price_from_usd(config, quote_mint, trade_size_usd)
        })
        .collect::<Vec<_>>();
    if quote_mint == config.tokens.usdc_mint {
        sizes.extend(
            config
                .strategy
                .trade_sizes
                .iter()
                .copied()
                .filter(|size| size.is_finite() && *size > 0.0),
        );
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes.dedup_by(|left, right| (*left - *right).abs() < 1e-9);
    sizes
}

fn quote_native_gas_cost(config: &Config, quote_mint: &str) -> Option<f64> {
    if quote_mint == config.tokens.sol_mint {
        Some(config.strategy.preflight_gas_cost_sol * 2.0)
    } else if quote_mint == config.tokens.usdc_mint {
        Some(config.strategy.preflight_gas_cost_sol * config.strategy.sol_usdc_price * 2.0)
    } else {
        None
    }
}

fn liquidity_usd_for_raydium(
    config: &Config,
    state: &RaydiumState,
    quote_mint: &str,
) -> Option<f64> {
    match state.venue {
        RaydiumVenue::Clmm => None,
        RaydiumVenue::AmmV4 => {
            if state.base_reserve == 0 || state.quote_reserve == 0 {
                None
            } else if quote_mint == config.tokens.usdc_mint {
                Some(state.quote_reserve as f64 / 1_000_000.0)
            } else if quote_mint == config.tokens.sol_mint {
                Some(
                    (state.quote_reserve as f64 / 1_000_000_000.0) * config.strategy.sol_usdc_price,
                )
            } else {
                None
            }
        }
        RaydiumVenue::Cpmm => Some(state.get_liquidity_usdc(config.strategy.sol_usdc_price)),
    }
}

fn default_decimals(mint: &str) -> i32 {
    if mint == "So11111111111111111111111111111111111111112" {
        9
    } else {
        6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const SOL: &str = "So11111111111111111111111111111111111111112";

    #[test]
    fn routeable_tokens_require_two_programs_with_same_quote() {
        let config = Config::default();
        let mut pump_state = HashMap::new();
        let mut raydium_state = HashMap::new();
        pump_state.insert(
            "pump".to_string(),
            PumpState {
                sol_reserve: 1_000_000_000,
                token_reserve: 1_000_000,
                token_mint: "MintA".to_string(),
                price_history: Vec::new(),
            },
        );
        raydium_state.insert(
            "ray".to_string(),
            RaydiumState {
                pool_address: "ray".to_string(),
                venue: RaydiumVenue::AmmV4,
                amm_config: None,
                base_mint: "MintA".to_string(),
                quote_mint: SOL.to_string(),
                base_vault: None,
                quote_vault: None,
                observation_key: None,
                base_reserve: 2_000_000,
                quote_reserve: 3_000_000_000,
                base_decimals: 6,
                quote_decimals: 9,
                sqrt_price_x64: None,
                liquidity: 0,
                tick_current: None,
                tick_spacing: None,
                base_fee_owed: 0,
                quote_fee_owed: 0,
                fee_bps: 25.0,
                price_history: Vec::new(),
            },
        );

        let tokens = routeable_tokens(
            &config,
            &HashMap::new(),
            &pump_state,
            &HashMap::new(),
            &raydium_state,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(tokens, vec!["MintA".to_string()]);
    }

    #[test]
    fn best_candidate_prefers_lower_buy_price() {
        let config = Config::default();
        let markets = vec![
            ProgramMarket {
                address: "a".to_string(),
                token_mint: "MintA".to_string(),
                quote_mint: SOL.to_string(),
                kind: ProgramKind::MeteoraDlmm,
                label: "Meteora".to_string(),
                price_quote: 1.0,
                price_usdc: 150.0,
                liquidity_usd: Some(1_000.0),
                source: MarketSource::LiveState,
            },
            ProgramMarket {
                address: "b".to_string(),
                token_mint: "MintA".to_string(),
                quote_mint: SOL.to_string(),
                kind: ProgramKind::Whirlpool,
                label: "Whirlpool".to_string(),
                price_quote: 1.2,
                price_usdc: 180.0,
                liquidity_usd: Some(1_000.0),
                source: MarketSource::LiveState,
            },
        ];

        let candidate = best_candidate_for_market_set(&config, markets).unwrap();
        assert_eq!(candidate.buy_market.kind, ProgramKind::MeteoraDlmm);
        assert_eq!(candidate.sell_market.kind, ProgramKind::Whirlpool);
        assert!(candidate.gross_profit_pct > 0.0);
    }

    #[test]
    fn configured_program_pair_trade_sizes_support_usdc_quotes() {
        let config = Config::default();

        let sizes = configured_program_pair_trade_sizes_quote(&config, USDC);
        let mut expected = config
            .strategy
            .program_pair_trade_sizes_sol
            .iter()
            .map(|size| size * config.strategy.sol_usdc_price)
            .collect::<Vec<_>>();
        expected.extend(
            config
                .strategy
                .trade_sizes
                .iter()
                .copied()
                .filter(|size| size.is_finite() && *size > 0.0),
        );
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        expected.dedup_by(|left, right| (*left - *right).abs() < 1e-9);

        assert_eq!(sizes, expected);
    }

    #[test]
    fn best_size_search_prefers_sizes_around_spot_winner() {
        let mut config = Config::default();
        config.strategy.program_pair_trade_sizes_sol = vec![0.05, 0.1, 0.25, 0.5, 0.75, 0.9];
        config.strategy.program_pair_best_size_search_samples = 4;

        let sizes = best_size_search_trade_sizes_quote(&config, SOL, 0.5);

        assert_eq!(sizes, vec![0.1, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn meteora_damm_v2_market_uses_reserve_price() {
        let config = Config::default();
        let state = MeteoraState {
            pool_address: "damm".to_string(),
            active_id: 0,
            bin_step: 0,
            base_factor: 0,
            variable_fee_control: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            volatility_accumulator: 0,
            token_x_mint: USDC.to_string(),
            token_y_mint: "MintA".to_string(),
            reserve_x: "reserve_x".to_string(),
            reserve_y: "reserve_y".to_string(),
            bin_array_bitmap: [0u64; 16],
            token_x_amount: 2_500_000,
            token_y_amount: 1_000_000,
            fee_bps: 30.0,
            damm_v2_pool_data: Some(vec![0; 8]),
            price_history: Vec::new(),
        };

        let market = meteora_market(&config, "damm", &state).unwrap();

        assert_eq!(market.quote_mint, USDC);
        assert!((market.price_quote - 2.5).abs() < 1e-9);
        assert!((market.price_usdc - 2.5).abs() < 1e-9);
    }
}
