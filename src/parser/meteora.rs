use crate::model::state::MeteoraState;
use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const METEORA_LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
const METEORA_BITMAP_EXTENSION_DISCRIMINATOR: [u8; 8] = [80, 111, 124, 113, 55, 237, 18, 5];
const METEORA_LB_PAIR_MIN_LEN: usize = 232;
const METEORA_LB_PAIR_BITMAP_OFFSET: usize = 584;
const METEORA_LB_PAIR_BITMAP_WORDS: usize = 16;
const METEORA_BITMAP_EXTENSION_LB_PAIR_OFFSET: usize = 8;
const METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK: usize = 8;
const METEORA_BITMAP_EXTENSION_CHUNKS: usize = 12;
const METEORA_BITMAP_EXTENSION_POSITIVE_BITMAP_OFFSET: usize =
    METEORA_BITMAP_EXTENSION_LB_PAIR_OFFSET + 32;
const METEORA_BITMAP_EXTENSION_NEGATIVE_BITMAP_OFFSET: usize =
    METEORA_BITMAP_EXTENSION_POSITIVE_BITMAP_OFFSET
        + METEORA_BITMAP_EXTENSION_CHUNKS
            * METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK
            * std::mem::size_of::<u64>();
const BIN_ARRAY_SEED: &[u8] = b"bin_array";
const BIN_ARRAY_BINS: usize = 70;
const BIN_ARRAY_HEADER_LEN: usize = 56;
const BIN_LEN: usize = 144;
const BIN_ARRAY_MIN_LEN: usize = BIN_ARRAY_HEADER_LEN + BIN_ARRAY_BINS * BIN_LEN;
pub const BIN_ARRAY_BITMAP_SIZE: i64 = 512;
pub const EXTENSION_BIN_ARRAY_BITMAP_SIZE: i64 = 12;
pub const BIN_ARRAY_INDEX_BOUND_MIN: i64 =
    -(BIN_ARRAY_BITMAP_SIZE * (EXTENSION_BIN_ARRAY_BITMAP_SIZE + 1));
pub const BIN_ARRAY_INDEX_BOUND_MAX: i64 =
    BIN_ARRAY_BITMAP_SIZE * (EXTENSION_BIN_ARRAY_BITMAP_SIZE + 1) - 1;

#[derive(Debug, Clone)]
pub struct MeteoraBinArrayState {
    pub lb_pair: String,
    pub index: i64,
    pub bins: Vec<MeteoraBinState>,
}

#[derive(Debug, Clone)]
pub struct MeteoraBinState {
    pub bin_id: i32,
    pub amount_x: u64,
    pub amount_y: u64,
    pub price: u128,
    pub liquidity_supply: u128,
}

#[derive(Debug, Clone)]
pub struct MeteoraBitmapExtensionState {
    pub lb_pair: String,
    pub positive_bin_array_bitmap:
        [[u64; METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK]; METEORA_BITMAP_EXTENSION_CHUNKS],
    pub negative_bin_array_bitmap:
        [[u64; METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK]; METEORA_BITMAP_EXTENSION_CHUNKS],
}

pub fn parse_meteora_state(data: &[u8], pool_address: &str) -> Result<MeteoraState> {
    if data.len() < METEORA_LB_PAIR_MIN_LEN {
        return Err(anyhow!("Meteora data too short: {} bytes", data.len()));
    }

    if data[..8] != METEORA_LB_PAIR_DISCRIMINATOR {
        return Err(anyhow!("Invalid Meteora LbPair discriminator"));
    }

    let base_factor = read_u16(data, 8)?;
    let variable_fee_control = read_u32(data, 16)?;
    let protocol_share = read_u16(data, 32)?;
    let base_fee_power_factor = read_u8(data, 34)?;
    let volatility_accumulator = read_u32(data, 40)?;
    let active_id = read_i32(data, 76)?;
    let bin_step = read_u16(data, 80)?;
    let token_x_mint = pubkey_at(data, 88)?;
    let token_y_mint = pubkey_at(data, 120)?;
    let reserve_x = pubkey_at(data, 152)?;
    let reserve_y = pubkey_at(data, 184)?;
    let bin_array_bitmap = if data.len()
        >= METEORA_LB_PAIR_BITMAP_OFFSET + METEORA_LB_PAIR_BITMAP_WORDS * std::mem::size_of::<u64>()
    {
        read_u64_array::<METEORA_LB_PAIR_BITMAP_WORDS>(data, METEORA_LB_PAIR_BITMAP_OFFSET)?
    } else {
        [0u64; METEORA_LB_PAIR_BITMAP_WORDS]
    };

    let mut state = MeteoraState {
        pool_address: pool_address.to_string(),
        active_id,
        bin_step,
        base_factor,
        variable_fee_control,
        protocol_share,
        base_fee_power_factor,
        volatility_accumulator,
        token_x_mint,
        token_y_mint,
        reserve_x,
        reserve_y,
        bin_array_bitmap,
        token_x_amount: 0,
        token_y_amount: 0,
        fee_bps: 0.0,
        damm_v2_pool_data: None,
        price_history: Vec::new(),
    };
    state.fee_bps = state.current_total_fee_rate() as f64 / 100_000.0;

    Ok(state)
}

pub fn bin_array_index(active_id: i32) -> i64 {
    let bins_per_array = BIN_ARRAY_BINS as i32;
    let mut index = active_id / bins_per_array;
    if active_id < 0 && active_id % bins_per_array != 0 {
        index -= 1;
    }
    i64::from(index)
}

pub fn derive_bin_array_address(lb_pair: &str, index: i64) -> Result<String> {
    let lb_pair = Pubkey::from_str(lb_pair)?;
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID)?;
    let (address, _) = Pubkey::find_program_address(
        &[BIN_ARRAY_SEED, lb_pair.as_ref(), &index.to_le_bytes()],
        &program_id,
    );
    Ok(address.to_string())
}

pub fn derive_nearby_bin_array_addresses(
    lb_pair: &str,
    active_id: i32,
    count_each_side: i64,
) -> Result<Vec<(String, i64)>> {
    let current = bin_array_index(active_id);
    (-count_each_side..=count_each_side)
        .map(|offset| {
            let index = current + offset;
            derive_bin_array_address(lb_pair, index).map(|address| (address, index))
        })
        .collect()
}

pub fn default_bitmap_has_bin_array_liquidity(bitmap: &[u64; 16], bin_array_index: i64) -> bool {
    if !(-BIN_ARRAY_BITMAP_SIZE..BIN_ARRAY_BITMAP_SIZE).contains(&bin_array_index) {
        return false;
    }

    let actual_index = (bin_array_index + BIN_ARRAY_BITMAP_SIZE) as usize;
    let word_index = actual_index / 64;
    let bit_index = actual_index % 64;
    bitmap
        .get(word_index)
        .is_some_and(|word| (word & (1u64 << bit_index)) != 0)
}

pub fn parse_bitmap_extension(data: &[u8]) -> Result<MeteoraBitmapExtensionState> {
    let min_len = METEORA_BITMAP_EXTENSION_NEGATIVE_BITMAP_OFFSET
        + METEORA_BITMAP_EXTENSION_CHUNKS
            * METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK
            * std::mem::size_of::<u64>();
    if data.len() < min_len {
        return Err(anyhow!(
            "Meteora bitmap extension too short: {} bytes",
            data.len()
        ));
    }
    if data[..8] != METEORA_BITMAP_EXTENSION_DISCRIMINATOR {
        return Err(anyhow!("Invalid Meteora bitmap extension discriminator"));
    }

    let lb_pair = pubkey_at(data, METEORA_BITMAP_EXTENSION_LB_PAIR_OFFSET)?;
    let positive_bin_array_bitmap = read_u64_matrix::<
        METEORA_BITMAP_EXTENSION_CHUNKS,
        METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK,
    >(data, METEORA_BITMAP_EXTENSION_POSITIVE_BITMAP_OFFSET)?;
    let negative_bin_array_bitmap = read_u64_matrix::<
        METEORA_BITMAP_EXTENSION_CHUNKS,
        METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK,
    >(data, METEORA_BITMAP_EXTENSION_NEGATIVE_BITMAP_OFFSET)?;

    Ok(MeteoraBitmapExtensionState {
        lb_pair,
        positive_bin_array_bitmap,
        negative_bin_array_bitmap,
    })
}

pub fn is_overflow_default_bin_array_bitmap(bin_array_index: i64) -> bool {
    bin_array_index > (BIN_ARRAY_BITMAP_SIZE - 1) || bin_array_index < -BIN_ARRAY_BITMAP_SIZE
}

pub fn bitmap_extension_has_bin_array_liquidity(
    state: &MeteoraBitmapExtensionState,
    bin_array_index: i64,
) -> bool {
    if !is_overflow_default_bin_array_bitmap(bin_array_index) {
        return false;
    }

    let idx = if bin_array_index < 0 {
        (-(bin_array_index + 1)) - BIN_ARRAY_BITMAP_SIZE
    } else {
        bin_array_index - BIN_ARRAY_BITMAP_SIZE
    };
    if idx < 0 {
        return false;
    }

    let bitmap_offset = (idx / BIN_ARRAY_BITMAP_SIZE) as usize;
    let offset_to_u64_in_bitmap = (idx / 64) as usize;
    let offset_to_bit = (idx % 64) as usize;
    let offset_to_u64_in_chunk = offset_to_u64_in_bitmap % METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK;

    let bitmap = if bin_array_index < 0 {
        state.negative_bin_array_bitmap.get(bitmap_offset)
    } else {
        state.positive_bin_array_bitmap.get(bitmap_offset)
    };

    bitmap
        .and_then(|chunk| chunk.get(offset_to_u64_in_chunk))
        .is_some_and(|word| (word & (1u64 << offset_to_bit)) != 0)
}

pub fn parse_bin_array(data: &[u8]) -> Result<MeteoraBinArrayState> {
    if data.len() < BIN_ARRAY_MIN_LEN {
        return Err(anyhow!("Meteora bin array too short: {} bytes", data.len()));
    }

    let index = read_i64(data, 8)?;
    let lb_pair = pubkey_at(data, 24)?;
    let start_bin_id = index
        .checked_mul(BIN_ARRAY_BINS as i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| anyhow!("Meteora bin array index out of range: {}", index))?;
    let mut bins = Vec::with_capacity(BIN_ARRAY_BINS);

    for local_index in 0..BIN_ARRAY_BINS {
        let offset = BIN_ARRAY_HEADER_LEN + local_index * BIN_LEN;
        let amount_x = read_u64(data, offset)?;
        let amount_y = read_u64(data, offset + 8)?;
        let price = read_u128(data, offset + 16)?;
        let liquidity_supply = read_u128(data, offset + 32)?;
        let bin_id = start_bin_id + local_index as i32;

        if amount_x > 0 || amount_y > 0 || liquidity_supply > 0 {
            bins.push(MeteoraBinState {
                bin_id,
                amount_x,
                amount_y,
                price,
                liquidity_supply,
            });
        }
    }

    Ok(MeteoraBinArrayState {
        lb_pair,
        index,
        bins,
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

fn read_i32(data: &[u8], offset: usize) -> Result<i32> {
    let end = offset + 4;
    if data.len() < end {
        return Err(anyhow!("i32 offset {} out of range {}", offset, data.len()));
    }
    Ok(i32::from_le_bytes(data[offset..end].try_into()?))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let end = offset + 2;
    if data.len() < end {
        return Err(anyhow!("u16 offset {} out of range {}", offset, data.len()));
    }
    Ok(u16::from_le_bytes(data[offset..end].try_into()?))
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8> {
    let end = offset + 1;
    if data.len() < end {
        return Err(anyhow!("u8 offset {} out of range {}", offset, data.len()));
    }
    Ok(data[offset])
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset + 4;
    if data.len() < end {
        return Err(anyhow!("u32 offset {} out of range {}", offset, data.len()));
    }
    Ok(u32::from_le_bytes(data[offset..end].try_into()?))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64> {
    let end = offset + 8;
    if data.len() < end {
        return Err(anyhow!("i64 offset {} out of range {}", offset, data.len()));
    }
    Ok(i64::from_le_bytes(data[offset..end].try_into()?))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let end = offset + 8;
    if data.len() < end {
        return Err(anyhow!("u64 offset {} out of range {}", offset, data.len()));
    }
    Ok(u64::from_le_bytes(data[offset..end].try_into()?))
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

fn read_u64_array<const N: usize>(data: &[u8], offset: usize) -> Result<[u64; N]> {
    let end = offset + N * std::mem::size_of::<u64>();
    if data.len() < end {
        return Err(anyhow!(
            "u64 array offset {} len {} out of range {}",
            offset,
            N,
            data.len()
        ));
    }

    let mut out = [0u64; N];
    for (index, slot) in out.iter_mut().enumerate() {
        let start = offset + index * 8;
        *slot = u64::from_le_bytes(data[start..start + 8].try_into()?);
    }
    Ok(out)
}

fn read_u64_matrix<const ROWS: usize, const COLS: usize>(
    data: &[u8],
    offset: usize,
) -> Result<[[u64; COLS]; ROWS]> {
    let mut out = [[0u64; COLS]; ROWS];
    for (row_index, row) in out.iter_mut().enumerate() {
        let row_offset = offset + row_index * COLS * std::mem::size_of::<u64>();
        *row = read_u64_array::<COLS>(data, row_offset)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lb_pair_layout() {
        let mut data = vec![0u8; METEORA_LB_PAIR_MIN_LEN];
        data[..8].copy_from_slice(&METEORA_LB_PAIR_DISCRIMINATOR);
        data[76..80].copy_from_slice(&10i32.to_le_bytes());
        data[80..82].copy_from_slice(&25u16.to_le_bytes());

        let token_x = [1u8; 32];
        let token_y = [2u8; 32];
        let reserve_x = [3u8; 32];
        let reserve_y = [4u8; 32];
        data[88..120].copy_from_slice(&token_x);
        data[120..152].copy_from_slice(&token_y);
        data[152..184].copy_from_slice(&reserve_x);
        data[184..216].copy_from_slice(&reserve_y);

        let state = parse_meteora_state(&data, "pool").unwrap();

        assert_eq!(state.pool_address, "pool");
        assert_eq!(state.active_id, 10);
        assert_eq!(state.bin_step, 25);
        assert_eq!(state.token_x_mint, bs58::encode(token_x).into_string());
        assert_eq!(state.token_y_mint, bs58::encode(token_y).into_string());
        assert_eq!(state.reserve_x, bs58::encode(reserve_x).into_string());
        assert_eq!(state.reserve_y, bs58::encode(reserve_y).into_string());
        assert_eq!(state.bin_array_bitmap, [0u64; 16]);
    }

    #[test]
    fn rejects_invalid_discriminator() {
        let data = vec![0u8; METEORA_LB_PAIR_MIN_LEN];
        assert!(parse_meteora_state(&data, "pool").is_err());
    }

    #[test]
    fn calculates_negative_bin_array_index() {
        assert_eq!(bin_array_index(-1), -1);
        assert_eq!(bin_array_index(-70), -1);
        assert_eq!(bin_array_index(-71), -2);
        assert_eq!(bin_array_index(0), 0);
        assert_eq!(bin_array_index(70), 1);
    }

    #[test]
    fn detects_default_bitmap_liquidity_bits() {
        let mut bitmap = [0u64; 16];
        bitmap[8] |= 1u64 << 0; // bin array index 0
        bitmap[7] |= 1u64 << 63; // bin array index -1
        bitmap[15] |= 1u64 << 63; // bin array index 511

        assert!(default_bitmap_has_bin_array_liquidity(&bitmap, 0));
        assert!(default_bitmap_has_bin_array_liquidity(&bitmap, -1));
        assert!(default_bitmap_has_bin_array_liquidity(&bitmap, 511));
        assert!(!default_bitmap_has_bin_array_liquidity(&bitmap, 1));
        assert!(!default_bitmap_has_bin_array_liquidity(&bitmap, 512));
    }

    #[test]
    fn detects_bitmap_extension_liquidity_bits() {
        let mut positive =
            [[0u64; METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK]; METEORA_BITMAP_EXTENSION_CHUNKS];
        let mut negative =
            [[0u64; METEORA_BITMAP_EXTENSION_WORDS_PER_CHUNK]; METEORA_BITMAP_EXTENSION_CHUNKS];
        positive[0][0] |= 1u64 << 0; // 512
        positive[1][0] |= 1u64 << 0; // 1024
        negative[0][0] |= 1u64 << 0; // -513

        let state = MeteoraBitmapExtensionState {
            lb_pair: "pool".to_string(),
            positive_bin_array_bitmap: positive,
            negative_bin_array_bitmap: negative,
        };

        assert!(bitmap_extension_has_bin_array_liquidity(&state, 512));
        assert!(bitmap_extension_has_bin_array_liquidity(&state, 1024));
        assert!(bitmap_extension_has_bin_array_liquidity(&state, -513));
        assert!(!bitmap_extension_has_bin_array_liquidity(&state, 1536));
    }

    #[test]
    fn parses_bin_array_layout() {
        let mut data = vec![0u8; BIN_ARRAY_MIN_LEN];
        data[8..16].copy_from_slice(&2i64.to_le_bytes());
        data[24..56].copy_from_slice(&[7u8; 32]);
        let first_bin = BIN_ARRAY_HEADER_LEN;
        data[first_bin..first_bin + 8].copy_from_slice(&10u64.to_le_bytes());
        data[first_bin + 8..first_bin + 16].copy_from_slice(&20u64.to_le_bytes());
        data[first_bin + 16..first_bin + 32].copy_from_slice(&30u128.to_le_bytes());
        data[first_bin + 32..first_bin + 48].copy_from_slice(&40u128.to_le_bytes());

        let state = parse_bin_array(&data).unwrap();

        assert_eq!(state.index, 2);
        assert_eq!(state.lb_pair, bs58::encode([7u8; 32]).into_string());
        assert_eq!(state.bins.len(), 1);
        assert_eq!(state.bins[0].bin_id, 140);
        assert_eq!(state.bins[0].amount_x, 10);
        assert_eq!(state.bins[0].amount_y, 20);
    }
}
