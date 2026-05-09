use crate::model::state::PumpState;
use anyhow::{anyhow, Result};

pub fn parse_pump_state(data: &[u8]) -> Result<PumpState> {
    // Pump.fun bonding curve account structure (204 bytes version)
    // Offset 0-8: discriminator
    // Offset 8-16: virtual_sol_reserves (u64)
    // Offset 16-24: virtual_token_reserves (u64)
    // Offset 24-32: real_sol_reserves (u64)
    // Offset 32-40: real_token_reserves (u64)
    // Offset 40-72: token_mint (32 bytes)

    if data.len() < 72 {
        return Err(anyhow!("Pump data too short: {} bytes", data.len()));
    }

    // Use virtual reserves for price calculation (more accurate for bonding curve)
    let sol_reserve = u64::from_le_bytes(
        data[8..16]
            .try_into()
            .map_err(|_| anyhow!("Failed to parse sol_reserve"))?,
    );

    let token_reserve = u64::from_le_bytes(
        data[16..24]
            .try_into()
            .map_err(|_| anyhow!("Failed to parse token_reserve"))?,
    );

    let token_mint = bs58::encode(&data[40..72]).into_string();

    Ok(PumpState {
        sol_reserve,
        token_reserve,
        token_mint,
        price_history: Vec::new(),
    })
}
