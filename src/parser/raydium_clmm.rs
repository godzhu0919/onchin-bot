use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const TICK_ARRAY_SEED: &[u8] = b"tick_array";
const POOL_TICK_ARRAY_BITMAP_SEED: &[u8] = b"pool_tick_array_bitmap_extension";
const TICK_ARRAY_SIZE: i32 = 60;
const TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [192, 155, 85, 205, 49, 249, 129, 42];
const TICK_ARRAY_HEADER_LEN: usize = 44;
const TICK_STATE_LEN: usize = 168;
const TICK_ARRAY_TICKS: usize = 60;
const TICK_ARRAY_MIN_LEN: usize = 8 + 32 + 4 + TICK_STATE_LEN * TICK_ARRAY_TICKS + 1 + 115;
const TICK_ARRAY_BITMAP_SIZE: i32 = 512;
const MIN_TICK: i32 = -443_636;
const MAX_TICK: i32 = 443_636;
const POOL_TICK_ARRAY_BITMAP_OFFSET: usize = 832;
const POOL_TICK_ARRAY_BITMAP_WORDS: usize = 16;
const BITMAP_EXTENSION_DISCRIMINATOR: [u8; 8] = [60, 150, 36, 219, 97, 128, 139, 153];
const BITMAP_EXTENSION_HEADER_LEN: usize = 8 + 32;
const BITMAP_EXTENSION_GROUPS: usize = 14;
const BITMAP_EXTENSION_WORDS_PER_GROUP: usize = 8;
const BITMAP_EXTENSION_BYTES_PER_GROUP: usize = BITMAP_EXTENSION_WORDS_PER_GROUP * 8;
const BITMAP_EXTENSION_MIN_LEN: usize =
    BITMAP_EXTENSION_HEADER_LEN + 2 * BITMAP_EXTENSION_GROUPS * BITMAP_EXTENSION_BYTES_PER_GROUP;

#[derive(Debug, Clone)]
pub struct ClmmTickArrayState {
    pub pool_id: String,
    pub start_tick_index: i32,
    pub initialized_tick_count: u8,
    pub initialized_ticks: Vec<ClmmTickState>,
}

#[derive(Debug, Clone)]
pub struct ClmmTickState {
    pub tick: i32,
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
}

pub fn tick_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = TICK_ARRAY_SIZE * i32::from(tick_spacing);
    let mut start = tick_index / ticks_in_array;
    if tick_index < 0 && tick_index % ticks_in_array != 0 {
        start -= 1;
    }
    start * ticks_in_array
}

pub fn nearby_tick_array_start_indexes(
    tick_index: i32,
    tick_spacing: u16,
    count_each_side: i32,
) -> Vec<i32> {
    let current = tick_array_start_index(tick_index, tick_spacing);
    let stride = TICK_ARRAY_SIZE * i32::from(tick_spacing);
    (-count_each_side..=count_each_side)
        .map(|offset| current + offset * stride)
        .collect()
}

pub fn derive_tick_array_address(pool_address: &str, start_tick_index: i32) -> Result<String> {
    let pool = Pubkey::from_str(pool_address)?;
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID)?;
    let (address, _) = Pubkey::find_program_address(
        &[
            TICK_ARRAY_SEED,
            pool.as_ref(),
            &start_tick_index.to_be_bytes(),
        ],
        &program_id,
    );
    Ok(address.to_string())
}

pub fn derive_tickarray_bitmap_extension_address(pool_address: &str) -> Result<String> {
    let pool = Pubkey::from_str(pool_address)?;
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID)?;
    let (address, _) =
        Pubkey::find_program_address(&[POOL_TICK_ARRAY_BITMAP_SEED, pool.as_ref()], &program_id);
    Ok(address.to_string())
}

pub fn derive_nearby_tick_array_addresses(
    pool_address: &str,
    tick_index: i32,
    tick_spacing: u16,
    count_each_side: i32,
) -> Result<Vec<(String, i32)>> {
    nearby_tick_array_start_indexes(tick_index, tick_spacing, count_each_side)
        .into_iter()
        .map(|start_index| {
            derive_tick_array_address(pool_address, start_index)
                .map(|address| (address, start_index))
        })
        .collect()
}

pub fn derive_tick_array_addresses_for_start_indexes(
    pool_address: &str,
    start_indexes: &[i32],
) -> Result<Vec<(String, i32)>> {
    start_indexes
        .iter()
        .copied()
        .map(|start_index| {
            derive_tick_array_address(pool_address, start_index)
                .map(|address| (address, start_index))
        })
        .collect()
}

pub fn initialized_tick_array_start_indexes_from_pool_state(
    pool_data: &[u8],
    tick_spacing: u16,
) -> Result<Vec<i32>> {
    let bitmap_end = POOL_TICK_ARRAY_BITMAP_OFFSET + POOL_TICK_ARRAY_BITMAP_WORDS * 8;
    if pool_data.len() < bitmap_end {
        return Err(anyhow!(
            "Invalid Raydium CLMM pool length for tick_array_bitmap: {}",
            pool_data.len()
        ));
    }

    let tick_count = tick_array_tick_count(tick_spacing)?;
    let mut start_indexes = Vec::new();
    for word_index in 0..POOL_TICK_ARRAY_BITMAP_WORDS {
        let offset = POOL_TICK_ARRAY_BITMAP_OFFSET + word_index * 8;
        let word = u64::from_le_bytes(pool_data[offset..offset + 8].try_into()?);
        push_word_initialized_start_indexes(
            &mut start_indexes,
            word,
            word_index,
            -TICK_ARRAY_BITMAP_SIZE,
            tick_count,
        );
    }
    Ok(start_indexes)
}

pub fn initialized_tick_array_start_indexes_from_bitmap_extension(
    extension_data: &[u8],
    tick_spacing: u16,
) -> Result<Vec<i32>> {
    if extension_data.len() < BITMAP_EXTENSION_MIN_LEN {
        return Err(anyhow!(
            "Invalid Raydium CLMM bitmap extension length: {}",
            extension_data.len()
        ));
    }
    if extension_data[..8] != BITMAP_EXTENSION_DISCRIMINATOR {
        return Err(anyhow!(
            "Invalid Raydium CLMM bitmap extension discriminator"
        ));
    }

    let tick_count = tick_array_tick_count(tick_spacing)?;
    let min_start = tick_array_start_index(MIN_TICK, tick_spacing);
    let max_start = tick_array_start_index(MAX_TICK, tick_spacing);
    let mut start_indexes = Vec::new();
    let mut start = min_start;
    while start <= max_start {
        if let Some((positive, group_index, bit_index)) =
            bitmap_extension_position(start, tick_spacing)?
        {
            if bitmap_extension_bit_is_set(extension_data, positive, group_index, bit_index)? {
                push_unique_start_index(&mut start_indexes, start);
            }
        }
        start = start
            .checked_add(tick_count)
            .ok_or_else(|| anyhow!("Raydium CLMM tick array scan overflow"))?;
    }
    Ok(start_indexes)
}

pub fn parse_tick_array(data: &[u8]) -> Result<ClmmTickArrayState> {
    if data.len() < TICK_ARRAY_MIN_LEN {
        return Err(anyhow!(
            "Invalid Raydium CLMM tick array length: {}",
            data.len()
        ));
    }
    if data[..8] != TICK_ARRAY_DISCRIMINATOR {
        return Err(anyhow!("Invalid Raydium CLMM tick array discriminator"));
    }

    let pool_id = bs58::encode(&data[8..40]).into_string();
    let start_tick_index = i32::from_le_bytes(data[40..44].try_into()?);
    let initialized_tick_count = data[TICK_ARRAY_HEADER_LEN + TICK_STATE_LEN * TICK_ARRAY_TICKS];
    let mut initialized_ticks = Vec::new();

    for index in 0..TICK_ARRAY_TICKS {
        let offset = TICK_ARRAY_HEADER_LEN + index * TICK_STATE_LEN;
        let tick = i32::from_le_bytes(data[offset..offset + 4].try_into()?);
        let liquidity_net = i128::from_le_bytes(data[offset + 4..offset + 20].try_into()?);
        let liquidity_gross = u128::from_le_bytes(data[offset + 20..offset + 36].try_into()?);
        if liquidity_gross > 0 {
            initialized_ticks.push(ClmmTickState {
                tick,
                liquidity_net,
                liquidity_gross,
            });
        }
    }

    Ok(ClmmTickArrayState {
        pool_id,
        start_tick_index,
        initialized_tick_count,
        initialized_ticks,
    })
}

fn tick_array_tick_count(tick_spacing: u16) -> Result<i32> {
    let tick_count = TICK_ARRAY_SIZE
        .checked_mul(i32::from(tick_spacing))
        .ok_or_else(|| anyhow!("Raydium CLMM tick array tick_count overflow"))?;
    if tick_count <= 0 {
        return Err(anyhow!(
            "Invalid Raydium CLMM tick_spacing {}",
            tick_spacing
        ));
    }
    Ok(tick_count)
}

fn push_word_initialized_start_indexes(
    start_indexes: &mut Vec<i32>,
    word: u64,
    word_index: usize,
    word_zero_offset: i32,
    tick_count: i32,
) {
    if word == 0 {
        return;
    }
    for bit in 0..64 {
        if word & (1u64 << bit) == 0 {
            continue;
        }
        let bitmap_offset = word_zero_offset + (word_index as i32 * 64) + bit;
        push_unique_start_index(start_indexes, bitmap_offset * tick_count);
    }
}

fn push_unique_start_index(start_indexes: &mut Vec<i32>, start_index: i32) {
    if !start_indexes.contains(&start_index) {
        start_indexes.push(start_index);
    }
}

fn bitmap_extension_position(
    start_tick_index: i32,
    tick_spacing: u16,
) -> Result<Option<(bool, usize, usize)>> {
    let tick_count = tick_array_tick_count(tick_spacing)?;
    let max_tick_in_tickarray_bitmap = tick_count
        .checked_mul(TICK_ARRAY_BITMAP_SIZE)
        .ok_or_else(|| anyhow!("Raydium CLMM bitmap tick range overflow"))?;

    if start_tick_index >= -max_tick_in_tickarray_bitmap
        && start_tick_index < max_tick_in_tickarray_bitmap
    {
        return Ok(None);
    }

    let abs_start = start_tick_index.abs();
    let mut group_index = abs_start / max_tick_in_tickarray_bitmap - 1;
    if start_tick_index < 0 && abs_start % max_tick_in_tickarray_bitmap == 0 {
        group_index -= 1;
    }
    if !(0..BITMAP_EXTENSION_GROUPS as i32).contains(&group_index) {
        return Ok(None);
    }

    let remainder = abs_start % max_tick_in_tickarray_bitmap;
    let mut bit_index = remainder / tick_count;
    if start_tick_index < 0 && remainder != 0 {
        bit_index = TICK_ARRAY_BITMAP_SIZE - bit_index;
    }
    if !(0..TICK_ARRAY_BITMAP_SIZE).contains(&bit_index) {
        return Ok(None);
    }

    Ok(Some((
        start_tick_index > 0,
        group_index as usize,
        bit_index as usize,
    )))
}

fn bitmap_extension_bit_is_set(
    extension_data: &[u8],
    positive: bool,
    group_index: usize,
    bit_index: usize,
) -> Result<bool> {
    let side_offset = if positive {
        0
    } else {
        BITMAP_EXTENSION_GROUPS * BITMAP_EXTENSION_BYTES_PER_GROUP
    };
    let group_offset =
        BITMAP_EXTENSION_HEADER_LEN + side_offset + group_index * BITMAP_EXTENSION_BYTES_PER_GROUP;
    let word_index = bit_index / 64;
    let bit = bit_index % 64;
    let word_offset = group_offset + word_index * 8;
    let word = u64::from_le_bytes(extension_data[word_offset..word_offset + 8].try_into()?);
    Ok(word & (1u64 << bit) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_negative_tick_array_start_index_like_raydium() {
        assert_eq!(tick_array_start_index(-1, 60), -3600);
        assert_eq!(tick_array_start_index(-3600, 60), -3600);
        assert_eq!(tick_array_start_index(3599, 60), 0);
        assert_eq!(tick_array_start_index(3600, 60), 3600);
    }

    #[test]
    fn parses_initialized_ticks() {
        let mut data = vec![0u8; TICK_ARRAY_MIN_LEN];
        data[..8].copy_from_slice(&TICK_ARRAY_DISCRIMINATOR);
        data[8..40].copy_from_slice(&[7u8; 32]);
        data[40..44].copy_from_slice(&(-3600i32).to_le_bytes());
        let tick_offset = TICK_ARRAY_HEADER_LEN + 2 * TICK_STATE_LEN;
        data[tick_offset..tick_offset + 4].copy_from_slice(&(-3480i32).to_le_bytes());
        data[tick_offset + 4..tick_offset + 20].copy_from_slice(&(-10i128).to_le_bytes());
        data[tick_offset + 20..tick_offset + 36].copy_from_slice(&10u128.to_le_bytes());
        data[TICK_ARRAY_HEADER_LEN + TICK_STATE_LEN * TICK_ARRAY_TICKS] = 1;

        let parsed = parse_tick_array(&data).unwrap();

        assert_eq!(parsed.start_tick_index, -3600);
        assert_eq!(parsed.initialized_tick_count, 1);
        assert_eq!(parsed.initialized_ticks.len(), 1);
        assert_eq!(parsed.initialized_ticks[0].tick, -3480);
    }

    #[test]
    fn parses_pool_state_tick_array_bitmap() {
        let mut data = vec![0u8; POOL_TICK_ARRAY_BITMAP_OFFSET + POOL_TICK_ARRAY_BITMAP_WORDS * 8];
        let word_offset = POOL_TICK_ARRAY_BITMAP_OFFSET + 8 * 8;
        let word = (1u64 << 0) | (1u64 << 2);
        data[word_offset..word_offset + 8].copy_from_slice(&word.to_le_bytes());

        let starts = initialized_tick_array_start_indexes_from_pool_state(&data, 1).unwrap();

        assert_eq!(starts, vec![0, 120]);
    }

    #[test]
    fn parses_bitmap_extension_initialized_tick_arrays() {
        let mut data = vec![0u8; BITMAP_EXTENSION_MIN_LEN];
        data[..8].copy_from_slice(&BITMAP_EXTENSION_DISCRIMINATOR);
        set_bitmap_extension_bit(&mut data, true, 0, 72);
        set_bitmap_extension_bit(&mut data, true, 0, 75);
        set_bitmap_extension_bit(&mut data, true, 13, 225);
        set_bitmap_extension_bit(&mut data, false, 13, 286);

        let starts = initialized_tick_array_start_indexes_from_bitmap_extension(&data, 1).unwrap();

        assert!(starts.contains(&35_040));
        assert!(starts.contains(&35_220));
        assert!(starts.contains(&443_580));
        assert!(starts.contains(&-443_640));
    }

    fn set_bitmap_extension_bit(
        data: &mut [u8],
        positive: bool,
        group_index: usize,
        bit_index: usize,
    ) {
        let side_offset = if positive {
            0
        } else {
            BITMAP_EXTENSION_GROUPS * BITMAP_EXTENSION_BYTES_PER_GROUP
        };
        let word_offset = BITMAP_EXTENSION_HEADER_LEN
            + side_offset
            + group_index * BITMAP_EXTENSION_BYTES_PER_GROUP
            + (bit_index / 64) * 8;
        let mut word = u64::from_le_bytes(data[word_offset..word_offset + 8].try_into().unwrap());
        word |= 1u64 << (bit_index % 64);
        data[word_offset..word_offset + 8].copy_from_slice(&word.to_le_bytes());
    }
}
