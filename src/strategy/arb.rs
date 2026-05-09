use crate::model::state::{MeteoraState, PumpState};

#[derive(Debug)]
pub struct ArbOpportunity {
    pub direction: String,
    pub profit_estimate: f64,
    pub intermediate_token: String,
}

/// Find triangular arbitrage: USDC -> Token -> USDC
/// Looks for opportunities where we can:
/// 1. Buy a token with USDC on one exchange
/// 2. Sell that token for USDC on another exchange
/// 3. Profit from the price difference
pub fn find_triangular_arb(
    pump: &PumpState,
    meteora: &MeteoraState,
    threshold: f64,
) -> Option<ArbOpportunity> {
    // Check if this is a valid triangular arbitrage setup
    // We need: USDC/Token on both exchanges

    // For Pump.fun: token_mint is the intermediate token, SOL is the quote
    // For Meteora: check if one of the tokens matches Pump's token

    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"; // USDC mint
    let sol_mint = "So11111111111111111111111111111111111111112"; // Wrapped SOL

    // Check if Meteora pool has the same token as Pump
    let token_matches =
        pump.token_mint == meteora.token_x_mint || pump.token_mint == meteora.token_y_mint;

    if !token_matches {
        return None;
    }

    // Determine which token is USDC in Meteora pool
    let meteora_has_usdc = meteora.token_x_mint == usdc_mint || meteora.token_y_mint == usdc_mint;

    let meteora_has_sol = meteora.token_x_mint == sol_mint || meteora.token_y_mint == sol_mint;

    if !meteora_has_usdc && !meteora_has_sol {
        return None;
    }

    // Calculate prices
    // Pump: price in SOL per token
    let pump_price_sol = if pump.token_reserve > 0 {
        pump.sol_reserve as f64 / pump.token_reserve as f64
    } else {
        return None;
    };

    // Meteora: price from DLMM
    let meteora_price = meteora.calculate_price();

    // If Meteora is Token/USDC, we can directly compare
    // If Meteora is Token/SOL, we need SOL/USDC price (assume ~$150 for now)
    let sol_usdc_price = 150.0;

    let (pump_price_usdc, meteora_price_usdc) = if meteora_has_usdc {
        // Direct USDC comparison
        (pump_price_sol * sol_usdc_price, meteora_price)
    } else {
        // Both in SOL terms
        (pump_price_sol, meteora_price)
    };

    // Calculate arbitrage opportunity
    let price_diff = (pump_price_usdc - meteora_price_usdc).abs();
    let price_diff_pct = price_diff / pump_price_usdc.min(meteora_price_usdc);

    if price_diff_pct > threshold {
        let direction = if pump_price_usdc > meteora_price_usdc {
            format!(
                "Buy on Meteora @ {:.6} -> Sell on Pump @ {:.6}",
                meteora_price_usdc, pump_price_usdc
            )
        } else {
            format!(
                "Buy on Pump @ {:.6} -> Sell on Meteora @ {:.6}",
                pump_price_usdc, meteora_price_usdc
            )
        };

        Some(ArbOpportunity {
            direction,
            profit_estimate: price_diff_pct * 100.0, // Convert to percentage
            intermediate_token: pump.token_mint.clone(),
        })
    } else {
        None
    }
}
