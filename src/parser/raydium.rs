use crate::model::state::{RaydiumState, RaydiumVenue};
use anyhow::{anyhow, Result};

const RAYDIUM_CPMM_POOL_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
const RAYDIUM_AMM_V4_POOL_LEN: usize = 752;
const RAYDIUM_CPMM_POOL_LEN: usize = 637;
const RAYDIUM_CLMM_MIN_POOL_LEN: usize = 1_400;
const RAYDIUM_CPMM_DEFAULT_FEE_BPS: f64 = 25.0;
const RAYDIUM_CLMM_DEFAULT_FEE_BPS: f64 = 25.0;

/// Parse Raydium AMM V4 pool state from account data
pub fn parse_raydium_state(data: &[u8], pool_address: &str) -> Result<RaydiumState> {
    if data.len() >= RAYDIUM_CLMM_MIN_POOL_LEN && data[..8] == RAYDIUM_CPMM_POOL_DISCRIMINATOR {
        return parse_raydium_clmm_pool(data, pool_address);
    }

    if data.len() >= RAYDIUM_CPMM_POOL_LEN && data[..8] == RAYDIUM_CPMM_POOL_DISCRIMINATOR {
        return parse_raydium_cpmm_pool(data, pool_address);
    }

    parse_raydium_pool_internal(data, pool_address)
}

pub fn parse_raydium_pool(data: &[u8]) -> Result<RaydiumState> {
    parse_raydium_pool_internal(data, "unknown")
}

fn parse_raydium_pool_internal(data: &[u8], pool_address: &str) -> Result<RaydiumState> {
    if data.len() < RAYDIUM_AMM_V4_POOL_LEN {
        return Err(anyhow!("Invalid Raydium pool data length: {}", data.len()));
    }

    let base_vault = pubkey_at(data, 336)?;
    let quote_vault = pubkey_at(data, 368)?;
    let base_mint = pubkey_at(data, 400)?;
    let quote_mint = pubkey_at(data, 432)?;
    let base_decimals = amm_v4_decimal(data, 32).unwrap_or_else(|| default_decimals(&base_mint));
    let quote_decimals = amm_v4_decimal(data, 40).unwrap_or_else(|| default_decimals(&quote_mint));

    Ok(RaydiumState {
        pool_address: pool_address.to_string(),
        venue: RaydiumVenue::AmmV4,
        amm_config: None,
        base_mint,
        quote_mint,
        base_vault: Some(base_vault),
        quote_vault: Some(quote_vault),
        observation_key: None,
        base_reserve: 0,
        quote_reserve: 0,
        base_decimals,
        quote_decimals,
        sqrt_price_x64: None,
        liquidity: 0,
        tick_current: None,
        tick_spacing: None,
        base_fee_owed: 0,
        quote_fee_owed: 0,
        fee_bps: crate::strategy::quote::RAYDIUM_FEE_BPS,
        price_history: Vec::new(),
    })
}

fn parse_raydium_cpmm_pool(data: &[u8], pool_address: &str) -> Result<RaydiumState> {
    if data.len() < RAYDIUM_CPMM_POOL_LEN {
        return Err(anyhow!(
            "Invalid Raydium CPMM pool data length: {}",
            data.len()
        ));
    }

    let token_0_vault = pubkey_at(data, 72)?;
    let token_1_vault = pubkey_at(data, 104)?;
    let token_0_mint = pubkey_at(data, 168)?;
    let token_1_mint = pubkey_at(data, 200)?;
    let token_0_decimals = default_decimals(&token_0_mint);
    let token_1_decimals = default_decimals(&token_1_mint);

    let protocol_fees_token_0 = read_u64(data, 341)?;
    let protocol_fees_token_1 = read_u64(data, 349)?;
    let fund_fees_token_0 = read_u64(data, 357)?;
    let fund_fees_token_1 = read_u64(data, 365)?;
    let creator_fees_token_0 = read_u64(data, 389)?;
    let creator_fees_token_1 = read_u64(data, 397)?;

    Ok(RaydiumState {
        pool_address: pool_address.to_string(),
        venue: RaydiumVenue::Cpmm,
        amm_config: None,
        base_mint: token_0_mint,
        quote_mint: token_1_mint,
        base_vault: Some(token_0_vault),
        quote_vault: Some(token_1_vault),
        observation_key: None,
        base_reserve: 0,
        quote_reserve: 0,
        base_decimals: token_0_decimals,
        quote_decimals: token_1_decimals,
        sqrt_price_x64: None,
        liquidity: 0,
        tick_current: None,
        tick_spacing: None,
        base_fee_owed: protocol_fees_token_0
            .saturating_add(fund_fees_token_0)
            .saturating_add(creator_fees_token_0),
        quote_fee_owed: protocol_fees_token_1
            .saturating_add(fund_fees_token_1)
            .saturating_add(creator_fees_token_1),
        fee_bps: RAYDIUM_CPMM_DEFAULT_FEE_BPS,
        price_history: Vec::new(),
    })
}

fn parse_raydium_clmm_pool(data: &[u8], pool_address: &str) -> Result<RaydiumState> {
    let amm_config = pubkey_at(data, 9)?;
    let token_0_mint = pubkey_at(data, 73)?;
    let token_1_mint = pubkey_at(data, 105)?;
    let token_0_vault = pubkey_at(data, 137)?;
    let token_1_vault = pubkey_at(data, 169)?;
    let observation_key = pubkey_at(data, 201)?;
    let mint_decimals_0 = data
        .get(233)
        .copied()
        .ok_or_else(|| anyhow!("CLMM mint_decimals_0 out of range {}", data.len()))?;
    let mint_decimals_1 = data
        .get(234)
        .copied()
        .ok_or_else(|| anyhow!("CLMM mint_decimals_1 out of range {}", data.len()))?;
    let tick_spacing = read_u16(data, 235)?;
    let liquidity = read_u128(data, 237)?;
    let sqrt_price_x64 = read_u128(data, 253)?;
    let tick_current = read_i32(data, 269)?;

    Ok(RaydiumState {
        pool_address: pool_address.to_string(),
        venue: RaydiumVenue::Clmm,
        amm_config: Some(amm_config),
        base_mint: token_0_mint,
        quote_mint: token_1_mint,
        base_vault: Some(token_0_vault),
        quote_vault: Some(token_1_vault),
        observation_key: Some(observation_key),
        base_reserve: 0,
        quote_reserve: 0,
        base_decimals: mint_decimals_0,
        quote_decimals: mint_decimals_1,
        sqrt_price_x64: Some(sqrt_price_x64),
        liquidity,
        tick_current: Some(tick_current),
        tick_spacing: Some(tick_spacing),
        base_fee_owed: 0,
        quote_fee_owed: 0,
        fee_bps: RAYDIUM_CLMM_DEFAULT_FEE_BPS,
        price_history: Vec::new(),
    })
}

fn pubkey_at(data: &[u8], offset: usize) -> Result<String> {
    let end = offset + 32;
    if data.len() < end {
        return Err(anyhow!(
            "pubkey offset {} out of range {}",
            offset,
            data.len()
        ));
    }
    Ok(bs58::encode(&data[offset..end]).into_string())
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let end = offset + 8;
    if data.len() < end {
        return Err(anyhow!("u64 offset {} out of range {}", offset, data.len()));
    }
    Ok(u64::from_le_bytes(data[offset..end].try_into()?))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let end = offset + 2;
    if data.len() < end {
        return Err(anyhow!("u16 offset {} out of range {}", offset, data.len()));
    }
    Ok(u16::from_le_bytes(data[offset..end].try_into()?))
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32> {
    let end = offset + 4;
    if data.len() < end {
        return Err(anyhow!("i32 offset {} out of range {}", offset, data.len()));
    }
    Ok(i32::from_le_bytes(data[offset..end].try_into()?))
}

fn read_u128(data: &[u8], offset: usize) -> Result<u128> {
    let end = offset + 16;
    if data.len() < end {
        return Err(anyhow!(
            "u128 offset {} out of range {}",
            offset,
            data.len()
        ));
    }
    Ok(u128::from_le_bytes(data[offset..end].try_into()?))
}

fn amm_v4_decimal(data: &[u8], offset: usize) -> Option<u8> {
    let value = read_u64(data, offset).ok()?;
    u8::try_from(value).ok().filter(|decimal| *decimal <= 18)
}

fn default_decimals(mint: &str) -> u8 {
    match mint {
        "So11111111111111111111111111111111111111112" => 9,
        _ => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amm_v4_pool_layout() {
        let mut data = vec![0u8; RAYDIUM_AMM_V4_POOL_LEN];

        let base_vault = [1u8; 32];
        let quote_vault = [2u8; 32];
        let base_mint = [3u8; 32];
        let quote_mint = [4u8; 32];
        data[32..40].copy_from_slice(&6u64.to_le_bytes());
        data[40..48].copy_from_slice(&9u64.to_le_bytes());
        data[336..368].copy_from_slice(&base_vault);
        data[368..400].copy_from_slice(&quote_vault);
        data[400..432].copy_from_slice(&base_mint);
        data[432..464].copy_from_slice(&quote_mint);

        let state = parse_raydium_state(&data, "pool").unwrap();

        assert_eq!(state.venue, RaydiumVenue::AmmV4);
        assert_eq!(
            state.base_vault.unwrap(),
            bs58::encode(base_vault).into_string()
        );
        assert_eq!(
            state.quote_vault.unwrap(),
            bs58::encode(quote_vault).into_string()
        );
        assert_eq!(state.base_mint, bs58::encode(base_mint).into_string());
        assert_eq!(state.quote_mint, bs58::encode(quote_mint).into_string());
        assert_eq!(state.base_decimals, 6);
        assert_eq!(state.quote_decimals, 9);
        assert_eq!(state.base_reserve, 0);
        assert_eq!(state.quote_reserve, 0);
    }

    #[test]
    fn parses_cpmm_pool_layout() {
        let mut data = vec![0u8; RAYDIUM_CPMM_POOL_LEN];
        data[..8].copy_from_slice(&RAYDIUM_CPMM_POOL_DISCRIMINATOR);

        let token_0_vault = [1u8; 32];
        let token_1_vault = [2u8; 32];
        let token_0_mint = [3u8; 32];
        let token_1_mint = [4u8; 32];
        data[72..104].copy_from_slice(&token_0_vault);
        data[104..136].copy_from_slice(&token_1_vault);
        data[168..200].copy_from_slice(&token_0_mint);
        data[200..232].copy_from_slice(&token_1_mint);
        data[341..349].copy_from_slice(&10u64.to_le_bytes());
        data[357..365].copy_from_slice(&20u64.to_le_bytes());
        data[389..397].copy_from_slice(&30u64.to_le_bytes());

        let state = parse_raydium_state(&data, "pool").unwrap();

        assert_eq!(state.venue, RaydiumVenue::Cpmm);
        assert_eq!(
            state.base_vault.unwrap(),
            bs58::encode(token_0_vault).into_string()
        );
        assert_eq!(
            state.quote_vault.unwrap(),
            bs58::encode(token_1_vault).into_string()
        );
        assert_eq!(state.base_mint, bs58::encode(token_0_mint).into_string());
        assert_eq!(state.quote_mint, bs58::encode(token_1_mint).into_string());
        assert_eq!(state.base_fee_owed, 60);
    }

    #[test]
    fn parses_clmm_pool_layout_before_cpmm_layout() {
        let mut data = vec![0u8; RAYDIUM_CLMM_MIN_POOL_LEN];
        data[..8].copy_from_slice(&RAYDIUM_CPMM_POOL_DISCRIMINATOR);

        let token_0_mint = [3u8; 32];
        let token_1_mint = [4u8; 32];
        let token_0_vault = [5u8; 32];
        let token_1_vault = [6u8; 32];
        let amm_config = [7u8; 32];
        let observation_key = [8u8; 32];
        let sqrt_price_x64 = 2u128 << 64;
        data[9..41].copy_from_slice(&amm_config);
        data[73..105].copy_from_slice(&token_0_mint);
        data[105..137].copy_from_slice(&token_1_mint);
        data[137..169].copy_from_slice(&token_0_vault);
        data[169..201].copy_from_slice(&token_1_vault);
        data[201..233].copy_from_slice(&observation_key);
        data[233] = 6;
        data[234] = 9;
        data[235..237].copy_from_slice(&60u16.to_le_bytes());
        data[237..253].copy_from_slice(&100u128.to_le_bytes());
        data[253..269].copy_from_slice(&sqrt_price_x64.to_le_bytes());
        data[269..273].copy_from_slice(&123i32.to_le_bytes());

        let state = parse_raydium_state(&data, "pool").unwrap();

        assert_eq!(state.venue, RaydiumVenue::Clmm);
        assert_eq!(
            state.amm_config.unwrap(),
            bs58::encode(amm_config).into_string()
        );
        assert_eq!(state.base_mint, bs58::encode(token_0_mint).into_string());
        assert_eq!(state.quote_mint, bs58::encode(token_1_mint).into_string());
        assert_eq!(
            state.base_vault.unwrap(),
            bs58::encode(token_0_vault).into_string()
        );
        assert_eq!(
            state.quote_vault.unwrap(),
            bs58::encode(token_1_vault).into_string()
        );
        assert_eq!(
            state.observation_key.unwrap(),
            bs58::encode(observation_key).into_string()
        );
        assert_eq!(state.base_decimals, 6);
        assert_eq!(state.quote_decimals, 9);
        assert_eq!(state.sqrt_price_x64, Some(sqrt_price_x64));
        assert_eq!(state.liquidity, 100);
        assert_eq!(state.tick_spacing, Some(60));
        assert_eq!(state.tick_current, Some(123));
    }
}
