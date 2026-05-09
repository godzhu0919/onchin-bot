use crate::model::state::RaydiumState;
use crate::parser::raydium_clmm::{ClmmTickArrayState, ClmmTickState};

const Q64: f64 = 18_446_744_073_709_551_616.0;

#[derive(Debug, Clone)]
pub struct ClmmExactInQuote {
    pub amount_in: f64,
    pub amount_out: f64,
    pub fee_amount: f64,
    pub sqrt_price_next_x64: u128,
    pub crossed_tick: bool,
    pub touched_tick_array_start_indexes: Vec<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClmmQuoteError {
    MissingPoolFields,
    InvalidAmount,
    InvalidLiquidity,
    MissingTickArray,
    MissingBoundaryTick,
    TickCrossingRequired,
    InvalidPrice,
    UnsupportedMint,
}

pub fn quote_exact_input_single_range(
    pool: &RaydiumState,
    tick_arrays: &[&ClmmTickArrayState],
    input_mint: &str,
    amount_in: f64,
) -> Result<ClmmExactInQuote, ClmmQuoteError> {
    if amount_in <= 0.0 || !amount_in.is_finite() {
        return Err(ClmmQuoteError::InvalidAmount);
    }
    if pool.liquidity == 0 {
        return Err(ClmmQuoteError::InvalidLiquidity);
    }

    let sqrt_price_x64 = pool
        .sqrt_price_x64
        .ok_or(ClmmQuoteError::MissingPoolFields)?;
    let tick_current = pool.tick_current.ok_or(ClmmQuoteError::MissingPoolFields)?;
    let _tick_spacing = pool.tick_spacing.ok_or(ClmmQuoteError::MissingPoolFields)?;
    let zero_for_one = if input_mint == pool.base_mint {
        true
    } else if input_mint == pool.quote_mint {
        false
    } else {
        return Err(ClmmQuoteError::UnsupportedMint);
    };

    let next_tick = next_initialized_tick(tick_arrays, tick_current, zero_for_one)
        .ok_or(ClmmQuoteError::MissingBoundaryTick)?;
    let touched_tick_array_start_indexes =
        tick_array_start_for_initialized_tick(tick_arrays, next_tick)
            .map(|start| vec![start])
            .ok_or(ClmmQuoteError::MissingTickArray)?;
    let boundary_sqrt_price_x64 = sqrt_price_at_tick(next_tick);
    let current_sqrt_price = sqrt_price_x64 as f64 / Q64;
    let boundary_sqrt_price = boundary_sqrt_price_x64 as f64 / Q64;
    if current_sqrt_price <= 0.0 || boundary_sqrt_price <= 0.0 {
        return Err(ClmmQuoteError::InvalidPrice);
    }

    let amount_less_fee = amount_in * (1.0 - pool.fee_bps / 10_000.0);
    let liquidity = pool.liquidity as f64;
    let (sqrt_price_next, amount_out, crossed_tick) = if zero_for_one {
        if boundary_sqrt_price >= current_sqrt_price {
            return Err(ClmmQuoteError::InvalidPrice);
        }
        let max_amount_in_before_tick = liquidity * (current_sqrt_price - boundary_sqrt_price)
            / (current_sqrt_price * boundary_sqrt_price);
        if amount_less_fee >= max_amount_in_before_tick {
            return Err(ClmmQuoteError::TickCrossingRequired);
        }
        let next =
            (liquidity * current_sqrt_price) / (liquidity + amount_less_fee * current_sqrt_price);
        let out = liquidity * (current_sqrt_price - next);
        (next, out, false)
    } else {
        if boundary_sqrt_price <= current_sqrt_price {
            return Err(ClmmQuoteError::InvalidPrice);
        }
        let max_amount_in_before_tick = liquidity * (boundary_sqrt_price - current_sqrt_price);
        if amount_less_fee >= max_amount_in_before_tick {
            return Err(ClmmQuoteError::TickCrossingRequired);
        }
        let next = current_sqrt_price + amount_less_fee / liquidity;
        let out = liquidity * (1.0 / current_sqrt_price - 1.0 / next);
        (next, out, false)
    };

    if amount_out <= 0.0 || !amount_out.is_finite() || sqrt_price_next <= 0.0 {
        return Err(ClmmQuoteError::InvalidPrice);
    }

    Ok(ClmmExactInQuote {
        amount_in,
        amount_out,
        fee_amount: amount_in - amount_less_fee,
        sqrt_price_next_x64: (sqrt_price_next * Q64) as u128,
        crossed_tick,
        touched_tick_array_start_indexes,
    })
}

pub fn quote_exact_input(
    pool: &RaydiumState,
    tick_arrays: &[&ClmmTickArrayState],
    input_mint: &str,
    amount_in: f64,
) -> Result<ClmmExactInQuote, ClmmQuoteError> {
    if amount_in <= 0.0 || !amount_in.is_finite() {
        return Err(ClmmQuoteError::InvalidAmount);
    }

    let sqrt_price_x64 = pool
        .sqrt_price_x64
        .ok_or(ClmmQuoteError::MissingPoolFields)?;
    let mut tick_current = pool.tick_current.ok_or(ClmmQuoteError::MissingPoolFields)?;
    let _tick_spacing = pool.tick_spacing.ok_or(ClmmQuoteError::MissingPoolFields)?;
    let zero_for_one = if input_mint == pool.base_mint {
        true
    } else if input_mint == pool.quote_mint {
        false
    } else {
        return Err(ClmmQuoteError::UnsupportedMint);
    };

    let mut liquidity = pool.liquidity as f64;
    if liquidity <= 0.0 || !liquidity.is_finite() {
        return Err(ClmmQuoteError::InvalidLiquidity);
    }

    let mut current_sqrt_price = sqrt_price_x64 as f64 / Q64;
    if current_sqrt_price <= 0.0 || !current_sqrt_price.is_finite() {
        return Err(ClmmQuoteError::InvalidPrice);
    }

    let fee_amount = amount_in * (pool.fee_bps / 10_000.0);
    let mut remaining = amount_in - fee_amount;
    let mut amount_out = 0.0;
    let mut crossed_tick = false;
    let mut touched_tick_array_start_indexes = Vec::new();

    for _ in 0..128 {
        if remaining <= 0.0 {
            break;
        }

        let (next_tick_state, tick_array_start_index) =
            next_initialized_tick_state_with_array(tick_arrays, tick_current, zero_for_one)
                .ok_or(ClmmQuoteError::MissingBoundaryTick)?;
        push_unique_start_index(
            &mut touched_tick_array_start_indexes,
            tick_array_start_index,
        );
        let boundary_sqrt_price_x64 = sqrt_price_at_tick(next_tick_state.tick);
        let boundary_sqrt_price = boundary_sqrt_price_x64 as f64 / Q64;
        if boundary_sqrt_price <= 0.0 || !boundary_sqrt_price.is_finite() {
            return Err(ClmmQuoteError::InvalidPrice);
        }

        if zero_for_one {
            if boundary_sqrt_price >= current_sqrt_price {
                return Err(ClmmQuoteError::InvalidPrice);
            }
            let max_amount_in_before_tick = liquidity * (current_sqrt_price - boundary_sqrt_price)
                / (current_sqrt_price * boundary_sqrt_price);
            if remaining < max_amount_in_before_tick {
                let next =
                    (liquidity * current_sqrt_price) / (liquidity + remaining * current_sqrt_price);
                amount_out += liquidity * (current_sqrt_price - next);
                current_sqrt_price = next;
                remaining = 0.0;
                break;
            }

            amount_out += liquidity * (current_sqrt_price - boundary_sqrt_price);
            remaining -= max_amount_in_before_tick;
            current_sqrt_price = boundary_sqrt_price;
            liquidity -= next_tick_state.liquidity_net as f64;
            tick_current = next_tick_state.tick - 1;
        } else {
            if boundary_sqrt_price <= current_sqrt_price {
                return Err(ClmmQuoteError::InvalidPrice);
            }
            let max_amount_in_before_tick = liquidity * (boundary_sqrt_price - current_sqrt_price);
            if remaining < max_amount_in_before_tick {
                let next = current_sqrt_price + remaining / liquidity;
                amount_out += liquidity * (1.0 / current_sqrt_price - 1.0 / next);
                current_sqrt_price = next;
                remaining = 0.0;
                break;
            }

            amount_out += liquidity * (1.0 / current_sqrt_price - 1.0 / boundary_sqrt_price);
            remaining -= max_amount_in_before_tick;
            current_sqrt_price = boundary_sqrt_price;
            liquidity += next_tick_state.liquidity_net as f64;
            tick_current = next_tick_state.tick;
        }

        crossed_tick = true;
        if liquidity <= 0.0 || !liquidity.is_finite() {
            return Err(ClmmQuoteError::InvalidLiquidity);
        }
    }

    if remaining > 0.0 {
        return Err(ClmmQuoteError::MissingBoundaryTick);
    }
    if amount_out <= 0.0 || !amount_out.is_finite() || current_sqrt_price <= 0.0 {
        return Err(ClmmQuoteError::InvalidPrice);
    }

    Ok(ClmmExactInQuote {
        amount_in,
        amount_out,
        fee_amount,
        sqrt_price_next_x64: (current_sqrt_price * Q64) as u128,
        crossed_tick,
        touched_tick_array_start_indexes,
    })
}

fn next_initialized_tick(
    tick_arrays: &[&ClmmTickArrayState],
    tick_current: i32,
    zero_for_one: bool,
) -> Option<i32> {
    let mut ticks: Vec<i32> = tick_arrays
        .iter()
        .flat_map(|array| array.initialized_ticks.iter().map(|tick| tick.tick))
        .collect();
    if ticks.is_empty() {
        return None;
    }
    ticks.sort_unstable();
    ticks.dedup();

    if zero_for_one {
        ticks.into_iter().rev().find(|tick| *tick <= tick_current)
    } else {
        ticks.into_iter().find(|tick| *tick > tick_current)
    }
}

fn next_initialized_tick_state<'a>(
    tick_arrays: &'a [&'a ClmmTickArrayState],
    tick_current: i32,
    zero_for_one: bool,
) -> Option<&'a ClmmTickState> {
    next_initialized_tick_state_with_array(tick_arrays, tick_current, zero_for_one)
        .map(|(tick, _)| tick)
}

fn next_initialized_tick_state_with_array<'a>(
    tick_arrays: &'a [&'a ClmmTickArrayState],
    tick_current: i32,
    zero_for_one: bool,
) -> Option<(&'a ClmmTickState, i32)> {
    if zero_for_one {
        tick_arrays
            .iter()
            .flat_map(|array| {
                array
                    .initialized_ticks
                    .iter()
                    .map(move |tick| (tick, array.start_tick_index))
            })
            .filter(|(tick, _)| tick.tick <= tick_current)
            .max_by_key(|(tick, _)| tick.tick)
    } else {
        tick_arrays
            .iter()
            .flat_map(|array| {
                array
                    .initialized_ticks
                    .iter()
                    .map(move |tick| (tick, array.start_tick_index))
            })
            .filter(|(tick, _)| tick.tick > tick_current)
            .min_by_key(|(tick, _)| tick.tick)
    }
}

fn tick_array_start_for_initialized_tick(
    tick_arrays: &[&ClmmTickArrayState],
    target_tick: i32,
) -> Option<i32> {
    tick_arrays.iter().find_map(|array| {
        array
            .initialized_ticks
            .iter()
            .any(|tick| tick.tick == target_tick)
            .then_some(array.start_tick_index)
    })
}

fn push_unique_start_index(indexes: &mut Vec<i32>, start_index: i32) {
    if !indexes.contains(&start_index) {
        indexes.push(start_index);
    }
}

fn sqrt_price_at_tick(tick: i32) -> u128 {
    let price = 1.0001_f64.powi(tick);
    (price.sqrt() * Q64) as u128
}

#[allow(dead_code)]
fn _tick_spacing_is_used_for_api_stability(_tick_spacing: u16) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::state::{RaydiumState, RaydiumVenue};
    use crate::parser::raydium_clmm::{ClmmTickArrayState, ClmmTickState};

    fn pool() -> RaydiumState {
        RaydiumState {
            pool_address: "pool".to_string(),
            venue: RaydiumVenue::Clmm,
            amm_config: Some("amm_config".to_string()),
            base_mint: "TOKEN".to_string(),
            quote_mint: "SOL".to_string(),
            base_vault: None,
            quote_vault: None,
            observation_key: Some("observation".to_string()),
            base_reserve: 0,
            quote_reserve: 0,
            base_decimals: 6,
            quote_decimals: 9,
            sqrt_price_x64: Some(Q64 as u128),
            liquidity: 1_000_000_000_000,
            tick_current: Some(0),
            tick_spacing: Some(60),
            base_fee_owed: 0,
            quote_fee_owed: 0,
            fee_bps: 25.0,
            price_history: Vec::new(),
        }
    }

    fn ticks() -> ClmmTickArrayState {
        ClmmTickArrayState {
            pool_id: "pool".to_string(),
            start_tick_index: -3600,
            initialized_tick_count: 3,
            initialized_ticks: vec![
                ClmmTickState {
                    tick: -120,
                    liquidity_net: 0,
                    liquidity_gross: 1,
                },
                ClmmTickState {
                    tick: -60,
                    liquidity_net: 0,
                    liquidity_gross: 1,
                },
                ClmmTickState {
                    tick: 60,
                    liquidity_net: 0,
                    liquidity_gross: 1,
                },
            ],
        }
    }

    #[test]
    fn quotes_token_zero_for_one_inside_current_range() {
        let pool = pool();
        let ticks = ticks();
        let quote = quote_exact_input_single_range(&pool, &[&ticks], "TOKEN", 1_000.0).unwrap();

        assert!(quote.amount_out > 0.0);
        assert!(!quote.crossed_tick);
    }

    #[test]
    fn blocks_trade_that_would_cross_tick() {
        let pool = pool();
        let ticks = ticks();
        let err = quote_exact_input_single_range(&pool, &[&ticks], "TOKEN", 10_000_000_000.0)
            .unwrap_err();

        assert_eq!(err, ClmmQuoteError::TickCrossingRequired);
    }

    #[test]
    fn quotes_across_initialized_ticks() {
        let pool = pool();
        let ticks = ticks();
        let quote = quote_exact_input(&pool, &[&ticks], "TOKEN", 4_000_000_000.0).unwrap();

        assert!(quote.amount_out > 0.0);
        assert!(quote.crossed_tick);
    }
}
