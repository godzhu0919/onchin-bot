use crate::model::state::{PumpState, RaydiumState};
use crate::strategy::quote;
use std::collections::HashMap;

/// Slippage and fee constants (conservative estimates)
const PUMP_FEE_BPS: f64 = 100.0; // 1% fee
const RAYDIUM_FEE_BPS: f64 = 25.0; // 0.25% fee
const SLIPPAGE_BPS: f64 = 50.0; // 0.5% slippage buffer
const GAS_COST_SOL: f64 = 0.001; // ~0.001 SOL per transaction

/// Calculate real profit after fees, slippage, and gas
pub fn calculate_real_profit(
    buy_price: f64,
    sell_price: f64,
    buy_fee_bps: f64,
    sell_fee_bps: f64,
    amount_usdc: f64,
    sol_usdc_price: f64,
) -> f64 {
    // Apply buy fee
    let after_buy_fee = amount_usdc * (1.0 - buy_fee_bps / 10000.0);

    // Calculate tokens received at buy price
    let tokens = after_buy_fee / buy_price;

    // Calculate USDC from selling tokens
    let gross_sell = tokens * sell_price;

    // Apply sell fee
    let after_sell_fee = gross_sell * (1.0 - sell_fee_bps / 10000.0);

    // Apply slippage
    let after_slippage = after_sell_fee * (1.0 - SLIPPAGE_BPS / 10000.0);

    // Subtract gas costs (convert to USDC)
    let gas_cost_usdc = GAS_COST_SOL * sol_usdc_price; // atomic execution uses a single tx
    let final_amount = after_slippage - gas_cost_usdc;

    // Return profit
    final_amount - amount_usdc
}

fn raydium_token_price_usdc(
    raydium_state: &RaydiumState,
    token_mint: &str,
    sol_usdc_price: f64,
) -> Option<f64> {
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let sol_mint = "So11111111111111111111111111111111111111112";

    let base_is_token = raydium_state.base_mint == token_mint;
    let quote_is_token = raydium_state.quote_mint == token_mint;

    if !base_is_token && !quote_is_token {
        return None;
    }

    let spot = raydium_state.calculate_price();
    if spot <= 0.0 || !spot.is_finite() {
        return None;
    }

    if base_is_token && raydium_state.quote_mint == usdc_mint {
        Some(spot)
    } else if quote_is_token && raydium_state.base_mint == usdc_mint {
        Some(1.0 / spot)
    } else if base_is_token && raydium_state.quote_mint == sol_mint {
        Some(spot * sol_usdc_price)
    } else if quote_is_token && raydium_state.base_mint == sol_mint {
        Some((1.0 / spot) * sol_usdc_price)
    } else {
        None
    }
}

/// Calculate price impact for a given trade size using constant product AMM formula
pub fn calculate_price_impact(
    reserve_in: u64,
    reserve_out: u64,
    amount_in: u64,
    fee_bps: f64,
) -> f64 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0.0;
    }

    // Apply fee to input amount
    let amount_in_with_fee = (amount_in as f64) * (1.0 - fee_bps / 10000.0);

    // Constant product formula: (x + Δx) * (y - Δy) = x * y
    let k = reserve_in as f64 * reserve_out as f64;
    let new_reserve_in = reserve_in as f64 + amount_in_with_fee;
    let new_reserve_out = k / new_reserve_in;
    let amount_out = reserve_out as f64 - new_reserve_out;

    if amount_out <= 0.0 {
        return 100.0; // Invalid trade
    }

    // Calculate effective price vs spot price
    let spot_price = reserve_out as f64 / reserve_in as f64;
    let effective_price = amount_out / amount_in as f64;

    // Price impact is the difference
    ((spot_price - effective_price) / spot_price).abs() * 100.0
}

pub fn validate_arbitrage(
    pump_state: &PumpState,
    raydium_state: &RaydiumState,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ValidatedArbitrage> {
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let sol_mint = "So11111111111111111111111111111111111111112";
    validate_arbitrage_with_mints(
        pump_state,
        raydium_state,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    )
}

pub fn validate_arbitrage_with_mints(
    pump_state: &PumpState,
    raydium_state: &RaydiumState,
    usdc_mint: &str,
    sol_mint: &str,
    sol_usdc_price: f64,
    trade_size_usdc: f64,
) -> Option<ValidatedArbitrage> {
    let quote = quote::quote_best_two_leg_arbitrage(
        pump_state,
        raydium_state,
        usdc_mint,
        sol_mint,
        sol_usdc_price,
        trade_size_usdc,
    )?;

    if quote.total_price_impact_bps > 300.0 {
        return None;
    }

    if quote.costs.net_profit_usdc > 0.0 {
        Some(ValidatedArbitrage {
            buy_venue: quote.buy_venue.as_str().to_string(),
            sell_venue: quote.sell_venue.as_str().to_string(),
            buy_price: quote.buy_price_usdc,
            sell_price: quote.sell_price_usdc,
            profit_usdc: quote.costs.net_profit_usdc,
            profit_pct: quote.profit_pct,
            price_impact: quote.total_price_impact_bps / 100.0,
            trade_size_usdc,
        })
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedArbitrage {
    pub buy_venue: String,
    pub sell_venue: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub profit_usdc: f64,
    pub profit_pct: f64,
    pub price_impact: f64,
    pub trade_size_usdc: f64,
}

/// Find validated arbitrage opportunities with real profit calculations
pub fn find_validated_opportunities(
    pump_pools: &HashMap<String, PumpState>,
    raydium_pools: &HashMap<String, RaydiumState>,
    sol_usdc_price: f64,
    trade_sizes: &[f64], // Different trade sizes to test
) -> Vec<ValidatedArbitrage> {
    let mut opportunities = Vec::new();

    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let sol_mint = "So11111111111111111111111111111111111111112";

    for (_raydium_addr, raydium_state) in raydium_pools {
        let Some(raydium_token) = quote::raydium_traded_token(raydium_state, usdc_mint, sol_mint)
        else {
            continue;
        };

        for (_pump_addr, pump_state) in pump_pools {
            if pump_state.token_mint != raydium_token {
                continue;
            }

            // Test different trade sizes
            for &trade_size in trade_sizes {
                if let Some(arb) =
                    validate_arbitrage(pump_state, raydium_state, sol_usdc_price, trade_size)
                {
                    opportunities.push(arb);
                }
            }
        }
    }

    // Sort by profit percentage descending
    opportunities.sort_by(|a, b| b.profit_pct.partial_cmp(&a.profit_pct).unwrap());
    opportunities
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: &str = "So11111111111111111111111111111111111111112";

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
    fn raydium_price_handles_token_as_base_against_usdc() {
        let token = "Token111111111111111111111111111111111111111";
        let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let state = raydium_state(token, usdc, 100_000_000, 2_000_000);

        let price = raydium_token_price_usdc(&state, token, 100.0).unwrap();

        assert!((price - 0.02).abs() < f64::EPSILON);
    }

    #[test]
    fn raydium_price_handles_token_as_quote_against_usdc() {
        let token = "Token111111111111111111111111111111111111111";
        let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let state = raydium_state(usdc, token, 2_000_000, 100_000_000);

        let price = raydium_token_price_usdc(&state, token, 100.0).unwrap();

        assert!((price - 0.02).abs() < f64::EPSILON);
    }

    #[test]
    fn raydium_price_converts_sol_quote_to_usdc() {
        let token = "Token111111111111111111111111111111111111111";
        let sol = "So11111111111111111111111111111111111111112";
        let state = raydium_state(token, sol, 100_000_000, 1_000_000_000);

        let price = raydium_token_price_usdc(&state, token, 80.0).unwrap();

        assert!((price - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn raydium_price_rejects_unrelated_pool() {
        let token = "Token111111111111111111111111111111111111111";
        let other = "Other111111111111111111111111111111111111111";
        let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let state = raydium_state(other, usdc, 100_000_000, 2_000_000);

        assert!(raydium_token_price_usdc(&state, token, 100.0).is_none());
    }
}
