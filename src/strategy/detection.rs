use crate::model::state::{PumpState, RaydiumState};
use crate::strategy::execution::ValidatedArbitrage;

pub fn detect_arbitrage(
    pump_state: &PumpState,
    raydium_state: &RaydiumState,
    min_profit_threshold: f64,
    sol_price: f64,
) -> Option<ValidatedArbitrage> {
    let pump_price = pump_state.calculate_price();
    let raydium_price = raydium_state.calculate_price();

    let price_diff = (pump_price - raydium_price).abs();
    let profit_pct = (price_diff / pump_price.min(raydium_price)) * 100.0;

    if profit_pct < min_profit_threshold {
        return None;
    }

    let (buy_venue, sell_venue, buy_price, sell_price) = if pump_price < raydium_price {
        ("Pump", "Raydium", pump_price, raydium_price)
    } else {
        ("Raydium", "Pump", raydium_price, pump_price)
    };

    let trade_size_usdc = 10.0; // Default trade size
    let profit_usdc = price_diff * sol_price * 0.01;

    Some(ValidatedArbitrage {
        buy_venue: buy_venue.to_string(),
        sell_venue: sell_venue.to_string(),
        buy_price: buy_price * sol_price,
        sell_price: sell_price * sol_price,
        profit_pct,
        profit_usdc,
        trade_size_usdc,
        price_impact: 0.5, // Estimated 0.5% price impact
    })
}
