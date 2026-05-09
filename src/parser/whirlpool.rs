use crate::model::state::{WhirlpoolState, WhirlpoolTickArrayState};
use anyhow::{anyhow, Result};
use orca_whirlpools_client::{TickArray as OrcaTickArray, Whirlpool as OrcaWhirlpool};
use std::str::FromStr;

pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

pub fn derive_nearby_tick_array_addresses(
    whirlpool: &str,
    tick_current_index: i32,
    tick_spacing: u16,
    count_each_side: i32,
) -> Result<Vec<(String, i32)>> {
    let whirlpool = solana_sdk::pubkey::Pubkey::from_str(whirlpool)?;
    let current_start_tick_index =
        orca_whirlpools_core::get_tick_array_start_tick_index(tick_current_index, tick_spacing);
    let offset = orca_whirlpools_core::TICK_ARRAY_SIZE as i32 * tick_spacing as i32;
    (-count_each_side..=count_each_side)
        .map(|relative| {
            let start_tick_index = current_start_tick_index + relative * offset;
            orca_whirlpools_client::get_tick_array_address(&whirlpool, start_tick_index)
                .map(|(address, _)| (address.to_string(), start_tick_index))
                .map_err(|error| anyhow!("derive Whirlpool tick array PDA failed: {error}"))
        })
        .collect()
}

pub fn parse_whirlpool_state(data: &[u8], pool_address: &str) -> Result<WhirlpoolState> {
    let state = OrcaWhirlpool::from_bytes(data)
        .map_err(|error| anyhow!("Whirlpool data parse failed: {error}"))?;

    Ok(WhirlpoolState {
        pool_address: pool_address.to_string(),
        whirlpools_config: state.whirlpools_config.to_string(),
        tick_spacing: state.tick_spacing,
        fee_rate: state.fee_rate,
        protocol_fee_rate: state.protocol_fee_rate,
        liquidity: state.liquidity,
        sqrt_price: state.sqrt_price,
        tick_current_index: state.tick_current_index,
        token_mint_a: state.token_mint_a.to_string(),
        token_vault_a: state.token_vault_a.to_string(),
        token_mint_b: state.token_mint_b.to_string(),
        token_vault_b: state.token_vault_b.to_string(),
        fee_bps: state.fee_rate as f64 / 100.0,
        price_history: Vec::new(),
    })
}

pub fn parse_tick_array(data: &[u8]) -> Result<WhirlpoolTickArrayState> {
    let tick_array = OrcaTickArray::from_bytes(data)
        .map_err(|error| anyhow!("Whirlpool tick array parse failed: {error}"))?;
    let facade: orca_whirlpools_core::TickArrayFacade = tick_array.clone().into();
    let initialized_tick_count = facade.ticks.iter().filter(|tick| tick.initialized).count();

    Ok(WhirlpoolTickArrayState {
        whirlpool: match &tick_array {
            OrcaTickArray::FixedTickArray(array) => array.whirlpool.to_string(),
            OrcaTickArray::DynamicTickArray(array) => array.whirlpool.to_string(),
        },
        start_tick_index: facade.start_tick_index,
        initialized_tick_count,
        tick_array: facade,
    })
}
