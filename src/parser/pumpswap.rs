use crate::model::state::PumpSwapState;
use anyhow::{anyhow, Result};

const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];
const TOKEN_ACCOUNT_MIN_LEN: usize = 72;
const GLOBAL_CONFIG_MIN_LEN: usize = 643;
const GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET: usize = 57;
const GLOBAL_CONFIG_RESERVED_FEE_RECIPIENT_OFFSET: usize = 385;
const GLOBAL_CONFIG_MAYHEM_MODE_ENABLED_OFFSET: usize = 417;
const GLOBAL_CONFIG_RESERVED_FEE_RECIPIENTS_OFFSET: usize = 418;
const GLOBAL_CONFIG_IS_CASHBACK_ENABLED_OFFSET: usize = 642;
const GLOBAL_CONFIG_BUYBACK_FEE_RECIPIENTS_OFFSET: usize = 643;
const GLOBAL_CONFIG_BUYBACK_BASIS_POINTS_OFFSET: usize = 899;
const GLOBAL_CONFIG_BUYBACK_MIN_LEN: usize = 907;
const PUBKEY_LEN: usize = 32;

#[derive(Debug, Clone)]
pub struct PumpSwapGlobalFeeConfig {
    pub protocol_fee_recipient: String,
    pub reserved_fee_recipient: Option<String>,
    pub reserved_fee_recipients: Vec<String>,
    pub mayhem_mode_enabled: bool,
    pub is_cashback_enabled: bool,
    pub buyback_fee_recipients: Vec<String>,
    pub buyback_basis_points: u64,
}

pub fn parse_pumpswap_pool(data: &[u8], pool_address: &str) -> Result<PumpSwapState> {
    if data.len() < 245 {
        return Err(anyhow!("PumpSwap pool data too short: {}", data.len()));
    }

    if data[0..8] != POOL_DISCRIMINATOR {
        return Err(anyhow!("Invalid PumpSwap pool discriminator"));
    }

    let base_mint = bs58::encode(&data[43..75]).into_string();
    let quote_mint = bs58::encode(&data[75..107]).into_string();
    let base_vault = bs58::encode(&data[139..171]).into_string();
    let quote_vault = bs58::encode(&data[171..203]).into_string();
    let coin_creator = bs58::encode(&data[211..243]).into_string();

    Ok(PumpSwapState {
        pool_address: pool_address.to_string(),
        base_mint,
        quote_mint,
        base_vault,
        quote_vault,
        coin_creator: Some(coin_creator),
        is_mayhem_mode: data[243] != 0,
        is_cashback_coin: data[244] != 0,
        base_reserve: 0,
        quote_reserve: 0,
        price_history: Vec::new(),
    })
}

pub fn parse_token_account_amount(data: &[u8]) -> Result<u64> {
    if data.len() < TOKEN_ACCOUNT_MIN_LEN {
        return Err(anyhow!("Token account data too short: {}", data.len()));
    }

    Ok(u64::from_le_bytes(data[64..72].try_into()?))
}

pub fn parse_global_config_fee_recipients(data: &[u8]) -> Result<PumpSwapGlobalFeeConfig> {
    if data.len() < GLOBAL_CONFIG_MIN_LEN {
        return Err(anyhow!("PumpSwap global config too short: {}", data.len()));
    }

    if data[0..8] != GLOBAL_CONFIG_DISCRIMINATOR {
        return Err(anyhow!("Invalid PumpSwap global config discriminator"));
    }

    let protocol_fee_recipient = (0..8)
        .find_map(|idx| {
            parse_optional_pubkey(
                data,
                GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET + idx * PUBKEY_LEN,
            )
        })
        .ok_or_else(|| anyhow!("PumpSwap global config has no protocol fee recipient"))?;

    let reserved_fee_recipient =
        parse_optional_pubkey(data, GLOBAL_CONFIG_RESERVED_FEE_RECIPIENT_OFFSET);
    let reserved_fee_recipients = (0..7)
        .filter_map(|idx| {
            parse_optional_pubkey(
                data,
                GLOBAL_CONFIG_RESERVED_FEE_RECIPIENTS_OFFSET + idx * PUBKEY_LEN,
            )
        })
        .collect();
    let (buyback_fee_recipients, buyback_basis_points) =
        if data.len() >= GLOBAL_CONFIG_BUYBACK_MIN_LEN {
            let recipients = (0..8)
                .filter_map(|idx| {
                    parse_optional_pubkey(
                        data,
                        GLOBAL_CONFIG_BUYBACK_FEE_RECIPIENTS_OFFSET + idx * PUBKEY_LEN,
                    )
                })
                .collect();
            let basis_points = u64::from_le_bytes(
                data[GLOBAL_CONFIG_BUYBACK_BASIS_POINTS_OFFSET
                    ..GLOBAL_CONFIG_BUYBACK_BASIS_POINTS_OFFSET + 8]
                    .try_into()?,
            );
            (recipients, basis_points)
        } else {
            (Vec::new(), 0)
        };

    Ok(PumpSwapGlobalFeeConfig {
        protocol_fee_recipient,
        reserved_fee_recipient,
        reserved_fee_recipients,
        mayhem_mode_enabled: data[GLOBAL_CONFIG_MAYHEM_MODE_ENABLED_OFFSET] != 0,
        is_cashback_enabled: data[GLOBAL_CONFIG_IS_CASHBACK_ENABLED_OFFSET] != 0,
        buyback_fee_recipients,
        buyback_basis_points,
    })
}

pub fn parse_global_config_protocol_fee_recipient(data: &[u8]) -> Result<String> {
    Ok(parse_global_config_fee_recipients(data)?.protocol_fee_recipient)
}

fn parse_optional_pubkey(data: &[u8], offset: usize) -> Option<String> {
    let bytes = data.get(offset..offset + PUBKEY_LEN)?;
    bytes
        .iter()
        .any(|byte| *byte != 0)
        .then(|| bs58::encode(bytes).into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_account_amount() {
        let mut data = vec![0u8; 165];
        data[64..72].copy_from_slice(&123_456u64.to_le_bytes());

        assert_eq!(parse_token_account_amount(&data).unwrap(), 123_456);
    }

    #[test]
    fn parses_pool_accounts_needed_for_swaps() {
        let mut data = vec![0u8; 245];
        data[..8].copy_from_slice(&POOL_DISCRIMINATOR);
        let base_mint = [1u8; 32];
        let quote_mint = [2u8; 32];
        let base_vault = [3u8; 32];
        let quote_vault = [4u8; 32];
        let coin_creator = [5u8; 32];
        data[43..75].copy_from_slice(&base_mint);
        data[75..107].copy_from_slice(&quote_mint);
        data[139..171].copy_from_slice(&base_vault);
        data[171..203].copy_from_slice(&quote_vault);
        data[211..243].copy_from_slice(&coin_creator);

        let state = parse_pumpswap_pool(&data, "pool").unwrap();

        assert_eq!(state.base_mint, bs58::encode(base_mint).into_string());
        assert_eq!(state.quote_mint, bs58::encode(quote_mint).into_string());
        assert_eq!(state.base_vault, bs58::encode(base_vault).into_string());
        assert_eq!(state.quote_vault, bs58::encode(quote_vault).into_string());
        assert_eq!(
            state.coin_creator.unwrap(),
            bs58::encode(coin_creator).into_string()
        );
        assert!(!state.is_mayhem_mode);
        assert!(!state.is_cashback_coin);
    }

    #[test]
    fn parses_first_nonzero_protocol_fee_recipient_from_global_config() {
        let mut data = vec![0u8; GLOBAL_CONFIG_MIN_LEN];
        data[..8].copy_from_slice(&GLOBAL_CONFIG_DISCRIMINATOR);
        let recipient = [9u8; 32];
        data[GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET
            ..GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET + 32]
            .copy_from_slice(&recipient);

        assert_eq!(
            parse_global_config_protocol_fee_recipient(&data).unwrap(),
            bs58::encode(recipient).into_string()
        );
    }

    #[test]
    fn parses_mayhem_fee_recipients_from_global_config() {
        let mut data = vec![0u8; GLOBAL_CONFIG_MIN_LEN];
        data[..8].copy_from_slice(&GLOBAL_CONFIG_DISCRIMINATOR);
        let protocol = [9u8; 32];
        let reserved = [7u8; 32];
        let extra_reserved = [6u8; 32];
        data[GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET
            ..GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET + 32]
            .copy_from_slice(&protocol);
        data[GLOBAL_CONFIG_RESERVED_FEE_RECIPIENT_OFFSET
            ..GLOBAL_CONFIG_RESERVED_FEE_RECIPIENT_OFFSET + 32]
            .copy_from_slice(&reserved);
        data[GLOBAL_CONFIG_RESERVED_FEE_RECIPIENTS_OFFSET
            ..GLOBAL_CONFIG_RESERVED_FEE_RECIPIENTS_OFFSET + 32]
            .copy_from_slice(&extra_reserved);
        data[GLOBAL_CONFIG_MAYHEM_MODE_ENABLED_OFFSET] = 1;
        data[GLOBAL_CONFIG_IS_CASHBACK_ENABLED_OFFSET] = 1;

        let parsed = parse_global_config_fee_recipients(&data).unwrap();

        assert_eq!(
            parsed.protocol_fee_recipient,
            bs58::encode(protocol).into_string()
        );
        assert_eq!(
            parsed.reserved_fee_recipient,
            Some(bs58::encode(reserved).into_string())
        );
        assert_eq!(
            parsed.reserved_fee_recipients,
            vec![bs58::encode(extra_reserved).into_string()]
        );
        assert!(parsed.mayhem_mode_enabled);
        assert!(parsed.is_cashback_enabled);
        assert!(parsed.buyback_fee_recipients.is_empty());
        assert_eq!(parsed.buyback_basis_points, 0);
    }

    #[test]
    fn parses_buyback_fee_recipients_from_extended_global_config() {
        let mut data = vec![0u8; GLOBAL_CONFIG_BUYBACK_MIN_LEN];
        data[..8].copy_from_slice(&GLOBAL_CONFIG_DISCRIMINATOR);
        let protocol = [9u8; 32];
        let buyback = [5u8; 32];
        data[GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET
            ..GLOBAL_CONFIG_PROTOCOL_FEE_RECIPIENTS_OFFSET + 32]
            .copy_from_slice(&protocol);
        data[GLOBAL_CONFIG_BUYBACK_FEE_RECIPIENTS_OFFSET
            ..GLOBAL_CONFIG_BUYBACK_FEE_RECIPIENTS_OFFSET + 32]
            .copy_from_slice(&buyback);
        data[GLOBAL_CONFIG_BUYBACK_BASIS_POINTS_OFFSET
            ..GLOBAL_CONFIG_BUYBACK_BASIS_POINTS_OFFSET + 8]
            .copy_from_slice(&5_000u64.to_le_bytes());

        let parsed = parse_global_config_fee_recipients(&data).unwrap();

        assert_eq!(
            parsed.buyback_fee_recipients,
            vec![bs58::encode(buyback).into_string()]
        );
        assert_eq!(parsed.buyback_basis_points, 5_000);
    }

    #[test]
    fn rejects_global_config_without_fee_recipient() {
        let mut data = vec![0u8; GLOBAL_CONFIG_MIN_LEN];
        data[..8].copy_from_slice(&GLOBAL_CONFIG_DISCRIMINATOR);

        assert!(parse_global_config_protocol_fee_recipient(&data).is_err());
    }

    #[test]
    fn rejects_invalid_pool_discriminator() {
        let data = vec![0u8; 245];

        assert!(parse_pumpswap_pool(&data, "pool").is_err());
    }
}
