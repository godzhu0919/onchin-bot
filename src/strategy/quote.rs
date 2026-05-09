use crate::model::state::{PumpState, RaydiumState};

pub const PUMP_FEE_BPS: f64 = 100.0;
pub const RAYDIUM_FEE_BPS: f64 = 25.0;
pub const SLIPPAGE_BUFFER_BPS: f64 = 50.0;
pub const GAS_COST_SOL: f64 = 0.001;
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Venue {
    Pump,
    PumpSwap,
    Raydium,
}

impl Venue {
    pub fn as_str(self) -> &'static str {
        match self {
            Venue::Pump => "Pump",
            Venue::PumpSwap => "PumpSwap",
            Venue::Raydium => "Raydium",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConstantProductPool {
    pub venue: Venue,
    pub token_mint: String,
    pub quote_mint: String,
    pub token_reserve: u64,
    pub quote_reserve: u64,
    pub fee_bps: f64,
}

#[derive(Debug, Clone)]
pub struct SwapQuote {
    pub venue: Venue,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: f64,
    pub amount_out: f64,
    pub spot_price: f64,
    pub effective_price: f64,
    pub fee_bps: f64,
    pub price_impact_bps: f64,
}

#[derive(Debug, Clone)]
pub struct CostBreakdown {
    pub gross_profit_usdc: f64,
    pub slippage_buffer_usdc: f64,
    pub gas_cost_usdc: f64,
    pub net_profit_usdc: f64,
}

#[derive(Debug, Clone)]
pub struct ArbitrageQuote {
    pub token_mint: String,
    pub buy_venue: Venue,
    pub sell_venue: Venue,
    pub buy_price_usdc: f64,
    pub sell_price_usdc: f64,
    pub trade_size_usdc: f64,
    pub final_amount_usdc: f64,
    pub profit_pct: f64,
    pub total_price_impact_bps: f64,
    pub buy_leg: SwapQuote,
    pub sell_leg: SwapQuote,
    pub costs: CostBreakdown,
}

pub fn raydium_traded_token(
    raydium: &RaydiumState,
    usdc_mint: &str,
    sol_mint: &str,
) -> Option<String> {
    let token = if raydium.base_mint == usdc_mint || raydium.base_mint == sol_mint {
        Some(raydium.quote_mint.clone())
    } else if raydium.quote_mint == usdc_mint || raydium.quote_mint == sol_mint {
        Some(raydium.base_mint.clone())
    } else {
        None
    };

    if let Some(ref t) = token {
        tracing::debug!(
            "Raydium pool {} traded token: {} (base={} quote={})",
            &raydium.pool_address[..8],
            &t[..8],
            &raydium.base_mint[..8],
            &raydium.quote_mint[..8]
        );
    } else {
        tracing::debug!(
            "Raydium pool {} has no traded token (base={} quote={}, looking for USDC={} or SOL={})",
            &raydium.pool_address[..8],
            &raydium.base_mint[..8],
            &raydium.quote_mint[..8],
            &usdc_mint[..8],
            &sol_mint[..8]
        );
    }

    token
}

pub fn quote_best_two_leg_arbitrage(
    pump: &PumpState,
    raydium: &RaydiumState,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ArbitrageQuote> {
    if trade_size_usdc <= 0.0 || sol_usdc_price <= 0.0 {
        return None;
    }

    let raydium_token = raydium_traded_token(raydium, usdc_mint, sol_mint)?;
    if raydium_token != pump.token_mint {
        return None;
    }

    let pump_to_raydium = quote_pump_buy_raydium_sell(
        pump,
        raydium,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    );
    let raydium_to_pump = quote_raydium_buy_pump_sell(
        pump,
        raydium,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    );

    match (pump_to_raydium, raydium_to_pump) {
        (Some(left), Some(right)) => {
            if left.costs.net_profit_usdc >= right.costs.net_profit_usdc {
                Some(left)
            } else {
                Some(right)
            }
        }
        (Some(quote), None) | (None, Some(quote)) => Some(quote),
        (None, None) => None,
    }
}

pub fn quote_best_cp_arbitrage(
    left: &ConstantProductPool,
    right: &ConstantProductPool,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ArbitrageQuote> {
    if left.token_mint != right.token_mint || left.token_mint == left.quote_mint {
        return None;
    }

    let left_to_right = quote_cp_path(
        left,
        right,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    );
    let right_to_left = quote_cp_path(
        right,
        left,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    );

    match (left_to_right, right_to_left) {
        (Some(left), Some(right)) => {
            if left.costs.net_profit_usdc >= right.costs.net_profit_usdc {
                Some(left)
            } else {
                Some(right)
            }
        }
        (Some(quote), None) | (None, Some(quote)) => Some(quote),
        (None, None) => None,
    }
}

fn quote_cp_path(
    buy_pool: &ConstantProductPool,
    sell_pool: &ConstantProductPool,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ArbitrageQuote> {
    let quote_amount_in = usdc_to_raw(
        &buy_pool.quote_mint,
        trade_size_usdc,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
    )?;
    let buy_leg = quote_constant_product(
        buy_pool.venue,
        &buy_pool.quote_mint,
        &buy_pool.token_mint,
        buy_pool.quote_reserve,
        buy_pool.token_reserve,
        quote_amount_in,
        buy_pool.fee_bps,
    )?;
    let sell_leg = quote_constant_product(
        sell_pool.venue,
        &sell_pool.token_mint,
        &sell_pool.quote_mint,
        sell_pool.token_reserve,
        sell_pool.quote_reserve,
        buy_leg.amount_out,
        sell_pool.fee_bps,
    )?;
    let final_amount_usdc = amount_to_usdc(
        &sell_leg.token_out,
        sell_leg.amount_out,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
    )?;

    build_arbitrage_quote_from_parts(
        buy_pool.token_mint.clone(),
        buy_pool.venue,
        sell_pool.venue,
        buy_leg,
        sell_leg,
        trade_size_usdc,
        final_amount_usdc,
        sol_usdc_price,
    )
}

fn quote_pump_buy_raydium_sell(
    pump: &PumpState,
    raydium: &RaydiumState,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ArbitrageQuote> {
    let sol_in = trade_size_usdc / sol_usdc_price;
    let sol_lamports_in = ui_to_raw(sol_in, 9);
    let pump_buy = quote_constant_product(
        Venue::Pump,
        sol_mint,
        &pump.token_mint,
        pump.sol_reserve,
        pump.token_reserve,
        sol_lamports_in,
        PUMP_FEE_BPS,
    )?;

    let raydium_sell = quote_raydium_token_to_quote(
        raydium,
        &pump.token_mint,
        usdc_mint,
        sol_mint,
        pump_buy.amount_out,
    )?;

    let final_amount_usdc = amount_to_usdc(
        &raydium_sell.token_out,
        raydium_sell.amount_out,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
    )?;

    build_arbitrage_quote(
        pump,
        Venue::Pump,
        Venue::Raydium,
        pump_buy,
        raydium_sell,
        trade_size_usdc,
        final_amount_usdc,
        sol_usdc_price,
    )
}

fn quote_raydium_buy_pump_sell(
    pump: &PumpState,
    raydium: &RaydiumState,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ArbitrageQuote> {
    let raydium_buy = quote_raydium_quote_to_token(
        raydium,
        &pump.token_mint,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    )?;

    let pump_sell = quote_constant_product(
        Venue::Pump,
        &pump.token_mint,
        sol_mint,
        pump.token_reserve,
        pump.sol_reserve,
        raydium_buy.amount_out,
        PUMP_FEE_BPS,
    )?;

    let final_amount_usdc = raw_to_ui(pump_sell.amount_out, 9) * sol_usdc_price;

    build_arbitrage_quote(
        pump,
        Venue::Raydium,
        Venue::Pump,
        raydium_buy,
        pump_sell,
        trade_size_usdc,
        final_amount_usdc,
        sol_usdc_price,
    )
}

fn quote_raydium_quote_to_token(
    raydium: &RaydiumState,
    token_mint: &str,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    amount_usdc: f64,
) -> Option<SwapQuote> {
    let base_is_token = raydium.base_mint == token_mint;
    let quote_is_token = raydium.quote_mint == token_mint;
    let base_reserve = raydium.effective_base_reserve();
    let quote_reserve = raydium.effective_quote_reserve();

    if base_is_token && (raydium.quote_mint == usdc_mint || raydium.quote_mint == sol_mint) {
        let amount_in = usdc_to_raw(
            &raydium.quote_mint,
            amount_usdc,
            usdc_mint,
            sol_mint,
            sol_usdc_price,
        )?;
        quote_constant_product(
            Venue::Raydium,
            &raydium.quote_mint,
            &raydium.base_mint,
            quote_reserve,
            base_reserve,
            amount_in,
            raydium.fee_bps,
        )
    } else if quote_is_token && (raydium.base_mint == usdc_mint || raydium.base_mint == sol_mint) {
        let amount_in = usdc_to_raw(
            &raydium.base_mint,
            amount_usdc,
            usdc_mint,
            sol_mint,
            sol_usdc_price,
        )?;
        quote_constant_product(
            Venue::Raydium,
            &raydium.base_mint,
            &raydium.quote_mint,
            base_reserve,
            quote_reserve,
            amount_in,
            raydium.fee_bps,
        )
    } else {
        None
    }
}

fn quote_raydium_token_to_quote(
    raydium: &RaydiumState,
    token_mint: &str,
    usdc_mint: &str,
    sol_mint: &str,
    token_amount_raw: f64,
) -> Option<SwapQuote> {
    let base_is_token = raydium.base_mint == token_mint;
    let quote_is_token = raydium.quote_mint == token_mint;
    let base_reserve = raydium.effective_base_reserve();
    let quote_reserve = raydium.effective_quote_reserve();

    if base_is_token && (raydium.quote_mint == usdc_mint || raydium.quote_mint == sol_mint) {
        quote_constant_product(
            Venue::Raydium,
            &raydium.base_mint,
            &raydium.quote_mint,
            base_reserve,
            quote_reserve,
            token_amount_raw,
            raydium.fee_bps,
        )
    } else if quote_is_token && (raydium.base_mint == usdc_mint || raydium.base_mint == sol_mint) {
        quote_constant_product(
            Venue::Raydium,
            &raydium.quote_mint,
            &raydium.base_mint,
            quote_reserve,
            base_reserve,
            token_amount_raw,
            raydium.fee_bps,
        )
    } else {
        None
    }
}

fn quote_constant_product(
    venue: Venue,
    token_in: &str,
    token_out: &str,
    reserve_in: u64,
    reserve_out: u64,
    amount_in: f64,
    fee_bps: f64,
) -> Option<SwapQuote> {
    if reserve_in == 0 || reserve_out == 0 || amount_in <= 0.0 || !amount_in.is_finite() {
        return None;
    }

    let reserve_in = reserve_in as f64;
    let reserve_out = reserve_out as f64;
    let amount_in_after_fee = amount_in * (1.0 - fee_bps / 10_000.0);
    let amount_out = (amount_in_after_fee * reserve_out) / (reserve_in + amount_in_after_fee);
    if amount_out <= 0.0 || !amount_out.is_finite() {
        return None;
    }

    let spot_price = reserve_in / reserve_out;
    let effective_price = amount_in / amount_out;
    let spot_out_per_in = reserve_out / reserve_in;
    let effective_out_per_in = amount_out / amount_in;
    let price_impact_bps =
        ((spot_out_per_in - effective_out_per_in) / spot_out_per_in).max(0.0) * 10_000.0;

    Some(SwapQuote {
        venue,
        token_in: token_in.to_string(),
        token_out: token_out.to_string(),
        amount_in,
        amount_out,
        spot_price,
        effective_price,
        fee_bps,
        price_impact_bps,
    })
}

fn build_arbitrage_quote(
    pump: &PumpState,
    buy_venue: Venue,
    sell_venue: Venue,
    buy_leg: SwapQuote,
    sell_leg: SwapQuote,
    trade_size_usdc: f64,
    final_amount_usdc: f64,
    sol_usdc_price: f64,
) -> Option<ArbitrageQuote> {
    build_arbitrage_quote_from_parts(
        pump.token_mint.clone(),
        buy_venue,
        sell_venue,
        buy_leg,
        sell_leg,
        trade_size_usdc,
        final_amount_usdc,
        sol_usdc_price,
    )
}

fn build_arbitrage_quote_from_parts(
    token_mint: String,
    buy_venue: Venue,
    sell_venue: Venue,
    buy_leg: SwapQuote,
    sell_leg: SwapQuote,
    trade_size_usdc: f64,
    final_amount_usdc: f64,
    sol_usdc_price: f64,
) -> Option<ArbitrageQuote> {
    if final_amount_usdc <= 0.0 || !final_amount_usdc.is_finite() {
        return None;
    }

    let gross_profit_usdc = final_amount_usdc - trade_size_usdc;
    let slippage_buffer_usdc = final_amount_usdc * (SLIPPAGE_BUFFER_BPS / 10_000.0);
    let gas_cost_usdc = GAS_COST_SOL * sol_usdc_price;
    let net_profit_usdc = gross_profit_usdc - slippage_buffer_usdc - gas_cost_usdc;
    let profit_pct = (net_profit_usdc / trade_size_usdc) * 100.0;
    let total_price_impact_bps = buy_leg.price_impact_bps + sell_leg.price_impact_bps;
    let buy_price_usdc =
        trade_size_usdc / normalize_token_amount(&buy_leg.token_out, buy_leg.amount_out);
    let sell_price_usdc =
        final_amount_usdc / normalize_token_amount(&buy_leg.token_out, buy_leg.amount_out);

    Some(ArbitrageQuote {
        token_mint,
        buy_venue,
        sell_venue,
        buy_price_usdc,
        sell_price_usdc,
        trade_size_usdc,
        final_amount_usdc,
        profit_pct,
        total_price_impact_bps,
        buy_leg,
        sell_leg,
        costs: CostBreakdown {
            gross_profit_usdc,
            slippage_buffer_usdc,
            gas_cost_usdc,
            net_profit_usdc,
        },
    })
}

fn amount_to_usdc(
    mint: &str,
    amount_raw: f64,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
) -> Option<f64> {
    if mint == usdc_mint {
        Some(raw_to_ui(amount_raw, 6))
    } else if mint == sol_mint {
        Some(raw_to_ui(amount_raw, 9) * sol_usdc_price)
    } else {
        None
    }
}

fn usdc_to_raw(
    mint: &str,
    amount_usdc: f64,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
) -> Option<f64> {
    if mint == usdc_mint {
        Some(ui_to_raw(amount_usdc, 6))
    } else if mint == sol_mint {
        Some(ui_to_raw(amount_usdc / sol_usdc_price, 9))
    } else {
        None
    }
}

fn normalize_token_amount(mint: &str, amount_raw: f64) -> f64 {
    raw_to_ui(amount_raw, decimals_for_mint(mint))
}

fn decimals_for_mint(mint: &str) -> u8 {
    match mint {
        "So11111111111111111111111111111111111111112" => 9,
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => 6,
        _ => DEFAULT_TOKEN_DECIMALS,
    }
}

fn ui_to_raw(amount: f64, decimals: u8) -> f64 {
    amount * 10_f64.powi(decimals as i32)
}

fn raw_to_ui(amount: f64, decimals: u8) -> f64 {
    amount / 10_f64.powi(decimals as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: &str = "So11111111111111111111111111111111111111112";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const TOKEN: &str = "Token111111111111111111111111111111111111111";

    fn pump_state() -> PumpState {
        PumpState {
            sol_reserve: 100_000_000_000,
            token_reserve: 10_000_000_000_000,
            token_mint: TOKEN.to_string(),
            price_history: Vec::new(),
        }
    }

    fn raydium_state(
        base_mint: &str,
        quote_mint: &str,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> RaydiumState {
        RaydiumState {
            pool_address: "pool".to_string(),
            venue: crate::model::state::RaydiumVenue::AmmV4,
            amm_config: None,
            base_mint: base_mint.to_string(),
            quote_mint: quote_mint.to_string(),
            base_vault: None,
            quote_vault: None,
            observation_key: None,
            base_reserve,
            quote_reserve,
            base_decimals: if base_mint == SOL { 9 } else { 6 },
            quote_decimals: if quote_mint == SOL { 9 } else { 6 },
            sqrt_price_x64: None,
            liquidity: 0,
            tick_current: None,
            tick_spacing: None,
            base_fee_owed: 0,
            quote_fee_owed: 0,
            fee_bps: RAYDIUM_FEE_BPS,
            price_history: Vec::new(),
        }
    }

    #[test]
    fn raydium_traded_token_rejects_unrelated_pool() {
        let pool = raydium_state("A", "B", 1_000, 1_000);

        assert!(raydium_traded_token(&pool, USDC, SOL).is_none());
    }

    #[test]
    fn quotes_profitable_pump_to_raydium_path() {
        let pump = pump_state();
        let raydium = raydium_state(TOKEN, SOL, 10_000_000_000_000, 150_000_000_000);

        let quote = quote_best_two_leg_arbitrage(&pump, &raydium, USDC, SOL, 100.0, 100.0).unwrap();

        assert_eq!(quote.buy_venue, Venue::Pump);
        assert_eq!(quote.sell_venue, Venue::Raydium);
        assert!(quote.costs.net_profit_usdc > 0.0);
        assert!(quote.total_price_impact_bps > 0.0);
    }

    #[test]
    fn quote_requires_matching_token() {
        let pump = pump_state();
        let raydium = raydium_state("OtherToken", SOL, 10_000_000_000_000, 150_000_000_000);

        assert!(quote_best_two_leg_arbitrage(&pump, &raydium, USDC, SOL, 100.0, 100.0).is_none());
    }
}
