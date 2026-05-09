use crate::model::state::MeteoraState;
use anyhow::{anyhow, Result};
use bytemuck::pod_read_unaligned;
use cp_amm::state::Pool;

pub const METEORA_DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const METEORA_DAMM_V2_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
pub const METEORA_DAMM_V2_POOL_LEN: usize = 8 + 1104;

pub fn parse_meteora_damm_v2_state(data: &[u8], pool_address: &str) -> Result<MeteoraState> {
    if data.len() < METEORA_DAMM_V2_POOL_LEN {
        return Err(anyhow!(
            "Meteora DAMM v2 data too short: {} bytes",
            data.len()
        ));
    }
    if data[..8] != METEORA_DAMM_V2_POOL_DISCRIMINATOR {
        return Err(anyhow!("Invalid Meteora DAMM v2 Pool discriminator"));
    }

    let pool: Pool = pod_read_unaligned(&data[8..8 + 1104]);
    let mut state = MeteoraState {
        pool_address: pool_address.to_string(),
        active_id: 0,
        bin_step: 0,
        base_factor: 0,
        variable_fee_control: 0,
        protocol_share: u16::from(pool.pool_fees.protocol_fee_percent),
        base_fee_power_factor: 0,
        volatility_accumulator: 0,
        token_x_mint: pool.token_a_mint.to_string(),
        token_y_mint: pool.token_b_mint.to_string(),
        reserve_x: pool.token_a_vault.to_string(),
        reserve_y: pool.token_b_vault.to_string(),
        bin_array_bitmap: [0u64; 16],
        token_x_amount: pool.token_a_amount,
        token_y_amount: pool.token_b_amount,
        fee_bps: 0.0,
        damm_v2_pool_data: Some(data[8..8 + 1104].to_vec()),
        price_history: Vec::new(),
    };
    state.fee_bps = pool.pool_fees.protocol_fee_percent as f64;
    Ok(state)
}
