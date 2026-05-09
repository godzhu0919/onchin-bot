use crate::model::state::{MeteoraState, PumpState, RaydiumState};
use std::collections::HashMap;

/// Represents a potential triangular arbitrage path
#[derive(Debug, Clone)]
pub struct TriangularPath {
    pub start_token: String,
    pub intermediate_token: String,
    pub end_token: String,
    pub profit_pct: f64,
    pub path_description: String,
}

/// Find USDC -> Token -> USDC arbitrage opportunities
/// This looks for cases where we can:
/// 1. Start with USDC
/// 2. Buy a token on one exchange
/// 3. Sell that token back to USDC on another exchange
/// 4. End with more USDC than we started
pub fn find_usdc_triangular_arb(
    pump_pools: &HashMap<String, PumpState>,
    meteora_pools: &HashMap<String, MeteoraState>,
    raydium_pools: &HashMap<String, RaydiumState>,
    sol_usdc_price: f64,
    min_profit_pct: f64,
) -> Vec<TriangularPath> {
    let mut opportunities = Vec::new();

    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let sol_mint = "So11111111111111111111111111111111111111112";

    // For each Meteora pool, check if we can form a triangular path
    for (_meteora_addr, meteora_state) in meteora_pools {
        // Check if this Meteora pool has USDC or SOL
        let has_usdc =
            meteora_state.token_x_mint == usdc_mint || meteora_state.token_y_mint == usdc_mint;
        let has_sol =
            meteora_state.token_x_mint == sol_mint || meteora_state.token_y_mint == sol_mint;

        if !has_usdc && !has_sol {
            continue;
        }

        // Determine the intermediate token in Meteora pool
        let meteora_intermediate =
            if meteora_state.token_x_mint == usdc_mint || meteora_state.token_x_mint == sol_mint {
                &meteora_state.token_y_mint
            } else {
                &meteora_state.token_x_mint
            };

        // Find matching Pump pool with the same intermediate token
        for (_pump_addr, pump_state) in pump_pools {
            if &pump_state.token_mint != meteora_intermediate {
                continue;
            }

            // We found a matching pair! Calculate arbitrage
            // Pump: SOL/Token
            // Meteora: USDC/Token or SOL/Token

            let pump_price_sol = if pump_state.token_reserve > 0 {
                pump_state.sol_reserve as f64 / pump_state.token_reserve as f64
            } else {
                continue;
            };

            let meteora_price = meteora_state.calculate_price();

            // Skip if price is invalid
            if !meteora_price.is_finite() || meteora_price <= 0.0 {
                continue;
            }

            // Convert prices to USDC terms
            let pump_price_usdc = pump_price_sol * sol_usdc_price;
            let meteora_price_usdc = if has_usdc {
                meteora_price
            } else {
                meteora_price * sol_usdc_price
            };

            // Calculate profit percentage
            // Path 1: Buy on Pump, Sell on Meteora
            let profit_1 = ((meteora_price_usdc - pump_price_usdc) / pump_price_usdc) * 100.0;

            // Path 2: Buy on Meteora, Sell on Pump
            let profit_2 = ((pump_price_usdc - meteora_price_usdc) / meteora_price_usdc) * 100.0;

            if profit_1 > min_profit_pct {
                opportunities.push(TriangularPath {
                    start_token: "USDC".to_string(),
                    intermediate_token: meteora_intermediate.clone(),
                    end_token: "USDC".to_string(),
                    profit_pct: profit_1,
                    path_description: format!(
                        "USDC -> Buy {} on Pump@{:.6} -> Sell on Meteora@{:.6} -> USDC",
                        &meteora_intermediate[..8],
                        pump_price_usdc,
                        meteora_price_usdc
                    ),
                });
            }

            if profit_2 > min_profit_pct {
                opportunities.push(TriangularPath {
                    start_token: "USDC".to_string(),
                    intermediate_token: meteora_intermediate.clone(),
                    end_token: "USDC".to_string(),
                    profit_pct: profit_2,
                    path_description: format!(
                        "USDC -> Buy {} on Meteora@{:.6} -> Sell on Pump@{:.6} -> USDC",
                        &meteora_intermediate[..8],
                        meteora_price_usdc,
                        pump_price_usdc
                    ),
                });
            }
        }
    }

    // Check Pump vs Raydium arbitrage
    for (_raydium_addr, raydium_state) in raydium_pools {
        // Check if Raydium pool has USDC or SOL
        let has_usdc =
            raydium_state.base_mint == usdc_mint || raydium_state.quote_mint == usdc_mint;
        let has_sol = raydium_state.base_mint == sol_mint || raydium_state.quote_mint == sol_mint;

        if !has_usdc && !has_sol {
            continue;
        }

        // Determine the intermediate token
        let raydium_intermediate =
            if raydium_state.base_mint == usdc_mint || raydium_state.base_mint == sol_mint {
                &raydium_state.quote_mint
            } else {
                &raydium_state.base_mint
            };

        // Find matching Pump pool
        for (_pump_addr, pump_state) in pump_pools {
            if &pump_state.token_mint != raydium_intermediate {
                continue;
            }

            let pump_price_sol = if pump_state.token_reserve > 0 {
                pump_state.sol_reserve as f64 / pump_state.token_reserve as f64
            } else {
                continue;
            };

            let raydium_price = raydium_state.calculate_price();
            if !raydium_price.is_finite() || raydium_price <= 0.0 {
                continue;
            }

            // Convert to USDC terms
            let pump_price_usdc = pump_price_sol * sol_usdc_price;
            let raydium_price_usdc = if has_usdc {
                raydium_price
            } else {
                raydium_price * sol_usdc_price
            };

            // Calculate profit
            let profit_1 = ((raydium_price_usdc - pump_price_usdc) / pump_price_usdc) * 100.0;
            let profit_2 = ((pump_price_usdc - raydium_price_usdc) / raydium_price_usdc) * 100.0;

            if profit_1 > min_profit_pct {
                opportunities.push(TriangularPath {
                    start_token: "USDC".to_string(),
                    intermediate_token: raydium_intermediate.clone(),
                    end_token: "USDC".to_string(),
                    profit_pct: profit_1,
                    path_description: format!(
                        "USDC -> Buy {} on Pump@{:.6} -> Sell on Raydium@{:.6} -> USDC",
                        &raydium_intermediate[..8],
                        pump_price_usdc,
                        raydium_price_usdc
                    ),
                });
            }

            if profit_2 > min_profit_pct {
                opportunities.push(TriangularPath {
                    start_token: "USDC".to_string(),
                    intermediate_token: raydium_intermediate.clone(),
                    end_token: "USDC".to_string(),
                    profit_pct: profit_2,
                    path_description: format!(
                        "USDC -> Buy {} on Raydium@{:.6} -> Sell on Pump@{:.6} -> USDC",
                        &raydium_intermediate[..8],
                        raydium_price_usdc,
                        pump_price_usdc
                    ),
                });
            }
        }
    }

    // Sort by profit percentage descending
    opportunities.sort_by(|a, b| b.profit_pct.partial_cmp(&a.profit_pct).unwrap());
    opportunities
}
