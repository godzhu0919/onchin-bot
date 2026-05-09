use crate::model::state::{MeteoraState, PumpSwapState, RaydiumState, WhirlpoolState};
use anyhow::{Context, Result};
use orca_whirlpools_client::{get_oracle_address, SwapV2, SwapV2InstructionArgs};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar::rent,
};
use std::str::FromStr;

pub const PUMP_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const METEORA_DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
pub const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const PUMP_FEE_PROGRAM_ID: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

pub const PUMP_AMM_BUY_EXACT_QUOTE_IN_DISCRIMINATOR: [u8; 8] =
    [198, 46, 21, 82, 180, 217, 232, 112];
pub const PUMP_AMM_BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
pub const PUMP_AMM_SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
pub const RAYDIUM_CLMM_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
pub const RAYDIUM_CLMM_SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
pub const RAYDIUM_CLMM_TICK_ARRAY_BITMAP_EXTENSION_SEED: &[u8] =
    b"pool_tick_array_bitmap_extension";
pub const METEORA_DLMM_SWAP_V2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];
pub const METEORA_DLMM_INITIALIZE_BITMAP_EXTENSION_DISCRIMINATOR: [u8; 8] =
    [47, 157, 226, 180, 12, 240, 33, 71];
pub const SPL_TOKEN_SYNC_NATIVE_DISCRIMINATOR: u8 = 17;

pub const PUMP_AMM_BUY_EXACT_QUOTE_IN_ACCOUNTS: [&str; 23] = [
    "pool",
    "user",
    "global_config",
    "base_mint",
    "quote_mint",
    "user_base_token_account",
    "user_quote_token_account",
    "pool_base_token_account",
    "pool_quote_token_account",
    "protocol_fee_recipient",
    "protocol_fee_recipient_token_account",
    "base_token_program",
    "quote_token_program",
    "system_program",
    "associated_token_program",
    "event_authority",
    "program",
    "coin_creator_vault_ata",
    "coin_creator_vault_authority",
    "global_volume_accumulator",
    "user_volume_accumulator",
    "fee_config",
    "fee_program",
];

pub const PUMP_AMM_SELL_ACCOUNTS: [&str; 21] = [
    "pool",
    "user",
    "global_config",
    "base_mint",
    "quote_mint",
    "user_base_token_account",
    "user_quote_token_account",
    "pool_base_token_account",
    "pool_quote_token_account",
    "protocol_fee_recipient",
    "protocol_fee_recipient_token_account",
    "base_token_program",
    "quote_token_program",
    "system_program",
    "associated_token_program",
    "event_authority",
    "program",
    "coin_creator_vault_ata",
    "coin_creator_vault_authority",
    "fee_config",
    "fee_program",
];

pub const RAYDIUM_CLMM_SWAP_ACCOUNTS: [&str; 10] = [
    "payer",
    "amm_config",
    "pool_state",
    "input_token_account",
    "output_token_account",
    "input_vault",
    "output_vault",
    "observation_state",
    "token_program",
    "tick_array",
];

pub const RAYDIUM_CLMM_SWAP_V2_ACCOUNTS: [&str; 13] = [
    "payer",
    "amm_config",
    "pool_state",
    "input_token_account",
    "output_token_account",
    "input_vault",
    "output_vault",
    "observation_state",
    "token_program",
    "token_program2022",
    "memo_program",
    "input_vault_mint",
    "output_vault_mint",
];

pub const METEORA_DLMM_SWAP_V2_ACCOUNTS: [&str; 16] = [
    "lb_pair",
    "bin_array_bitmap_extension_or_program",
    "reserve_x",
    "reserve_y",
    "user_token_in",
    "user_token_out",
    "token_x_mint",
    "token_y_mint",
    "oracle",
    "host_fee_in_or_program",
    "user",
    "token_x_program",
    "token_y_program",
    "memo_program",
    "event_authority",
    "program",
];

pub const METEORA_DAMM_V2_SWAP_V2_ACCOUNTS: [&str; 14] = [
    "pool_authority",
    "pool",
    "input_token_account",
    "output_token_account",
    "token_a_vault",
    "token_b_vault",
    "token_a_mint",
    "token_b_mint",
    "payer",
    "token_a_program",
    "token_b_program",
    "referral_token_account",
    "event_authority",
    "program",
];

pub const ORCA_WHIRLPOOL_SWAP_V2_ACCOUNTS: [&str; 15] = [
    "token_program_a",
    "token_program_b",
    "memo_program",
    "token_authority",
    "whirlpool",
    "token_mint_a",
    "token_mint_b",
    "token_owner_account_a",
    "token_vault_a",
    "token_owner_account_b",
    "token_vault_b",
    "tick_array0",
    "tick_array1",
    "tick_array2",
    "oracle",
];

const PUMP_FEE_CONFIG_AUTHORITY: [u8; 32] = [
    12, 20, 222, 252, 130, 94, 198, 118, 148, 37, 8, 24, 187, 101, 64, 101, 244, 41, 141, 49, 86,
    213, 113, 180, 212, 248, 9, 12, 24, 233, 168, 99,
];

#[derive(Debug, Clone)]
pub struct PumpAmmDerivedAccounts {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub global_config: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub user_base_token_account: Pubkey,
    pub user_quote_token_account: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub protocol_fee_recipient: Pubkey,
    pub protocol_fee_recipient_token_account: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub system_program: Pubkey,
    pub associated_token_program: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
    pub coin_creator_vault_ata: Pubkey,
    pub coin_creator_vault_authority: Pubkey,
    pub global_volume_accumulator: Pubkey,
    pub user_volume_accumulator: Pubkey,
    pub fee_config: Pubkey,
    pub fee_program: Pubkey,
}

#[derive(Debug, Clone)]
pub struct RaydiumClmmDerivedAccounts {
    pub payer: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    pub token_program: Pubkey,
    pub token_program2022: Pubkey,
    pub memo_program: Pubkey,
    pub input_vault_mint: Pubkey,
    pub output_vault_mint: Pubkey,
    pub tick_array: Pubkey,
    pub is_base_input: bool,
}

#[derive(Debug, Clone)]
pub struct MeteoraDlmmDerivedAccounts {
    pub lb_pair: Pubkey,
    pub bin_array_bitmap_extension_or_program: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub user_token_in: Pubkey,
    pub user_token_out: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub host_fee_in_or_program: Pubkey,
    pub user: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub memo_program: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
    pub swap_x_to_y: bool,
}

#[derive(Debug, Clone)]
pub struct MeteoraDammV2DerivedAccounts {
    pub pool_authority: Pubkey,
    pub pool: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub payer: Pubkey,
    pub token_a_program: Pubkey,
    pub token_b_program: Pubkey,
    pub referral_token_account: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
    pub swap_a_to_b: bool,
}

#[derive(Debug, Clone)]
pub struct WhirlpoolDerivedAccounts {
    pub token_program_a: Pubkey,
    pub token_program_b: Pubkey,
    pub memo_program: Pubkey,
    pub token_authority: Pubkey,
    pub whirlpool: Pubkey,
    pub token_mint_a: Pubkey,
    pub token_mint_b: Pubkey,
    pub token_owner_account_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_owner_account_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub tick_array0: Pubkey,
    pub tick_array1: Pubkey,
    pub tick_array2: Pubkey,
    pub oracle: Pubkey,
    pub a_to_b: bool,
}

impl RaydiumClmmDerivedAccounts {
    pub fn swap_accounts(&self) -> [Pubkey; 10] {
        [
            self.payer,
            self.amm_config,
            self.pool_state,
            self.input_token_account,
            self.output_token_account,
            self.input_vault,
            self.output_vault,
            self.observation_state,
            self.token_program,
            self.tick_array,
        ]
    }

    pub fn swap_v2_accounts(&self) -> [Pubkey; 13] {
        [
            self.payer,
            self.amm_config,
            self.pool_state,
            self.input_token_account,
            self.output_token_account,
            self.input_vault,
            self.output_vault,
            self.observation_state,
            self.token_program,
            self.token_program2022,
            self.memo_program,
            self.input_vault_mint,
            self.output_vault_mint,
        ]
    }
}

impl PumpAmmDerivedAccounts {
    pub fn buy_exact_quote_in_accounts(&self) -> [Pubkey; 23] {
        [
            self.pool,
            self.user,
            self.global_config,
            self.base_mint,
            self.quote_mint,
            self.user_base_token_account,
            self.user_quote_token_account,
            self.pool_base_token_account,
            self.pool_quote_token_account,
            self.protocol_fee_recipient,
            self.protocol_fee_recipient_token_account,
            self.base_token_program,
            self.quote_token_program,
            self.system_program,
            self.associated_token_program,
            self.event_authority,
            self.program,
            self.coin_creator_vault_ata,
            self.coin_creator_vault_authority,
            self.global_volume_accumulator,
            self.user_volume_accumulator,
            self.fee_config,
            self.fee_program,
        ]
    }

    pub fn sell_accounts(&self) -> [Pubkey; 21] {
        [
            self.pool,
            self.user,
            self.global_config,
            self.base_mint,
            self.quote_mint,
            self.user_base_token_account,
            self.user_quote_token_account,
            self.pool_base_token_account,
            self.pool_quote_token_account,
            self.protocol_fee_recipient,
            self.protocol_fee_recipient_token_account,
            self.base_token_program,
            self.quote_token_program,
            self.system_program,
            self.associated_token_program,
            self.event_authority,
            self.program,
            self.coin_creator_vault_ata,
            self.coin_creator_vault_authority,
            self.fee_config,
            self.fee_program,
        ]
    }
}

impl MeteoraDlmmDerivedAccounts {
    pub fn swap_v2_accounts(&self) -> [Pubkey; 16] {
        [
            self.lb_pair,
            self.bin_array_bitmap_extension_or_program,
            self.reserve_x,
            self.reserve_y,
            self.user_token_in,
            self.user_token_out,
            self.token_x_mint,
            self.token_y_mint,
            self.oracle,
            self.host_fee_in_or_program,
            self.user,
            self.token_x_program,
            self.token_y_program,
            self.memo_program,
            self.event_authority,
            self.program,
        ]
    }
}

impl MeteoraDammV2DerivedAccounts {
    pub fn swap_v2_accounts(&self) -> [Pubkey; 14] {
        [
            self.pool_authority,
            self.pool,
            self.input_token_account,
            self.output_token_account,
            self.token_a_vault,
            self.token_b_vault,
            self.token_a_mint,
            self.token_b_mint,
            self.payer,
            self.token_a_program,
            self.token_b_program,
            self.referral_token_account,
            self.event_authority,
            self.program,
        ]
    }
}

impl WhirlpoolDerivedAccounts {
    pub fn swap_v2_accounts(&self) -> [Pubkey; 15] {
        [
            self.token_program_a,
            self.token_program_b,
            self.memo_program,
            self.token_authority,
            self.whirlpool,
            self.token_mint_a,
            self.token_mint_b,
            self.token_owner_account_a,
            self.token_vault_a,
            self.token_owner_account_b,
            self.token_vault_b,
            self.tick_array0,
            self.tick_array1,
            self.tick_array2,
            self.oracle,
        ]
    }
}

pub fn derive_pump_amm_accounts(
    pool: &PumpSwapState,
    user: &Pubkey,
    protocol_fee_recipient: &Pubkey,
) -> Result<PumpAmmDerivedAccounts> {
    let token_program = pubkey(TOKEN_PROGRAM_ID)?;
    derive_pump_amm_accounts_with_token_programs(
        pool,
        user,
        protocol_fee_recipient,
        &token_program,
        &token_program,
    )
}

pub fn derive_pump_amm_accounts_with_token_programs(
    pool: &PumpSwapState,
    user: &Pubkey,
    protocol_fee_recipient: &Pubkey,
    base_token_program: &Pubkey,
    quote_token_program: &Pubkey,
) -> Result<PumpAmmDerivedAccounts> {
    let program = pubkey(PUMP_AMM_PROGRAM_ID)?;
    let system_program = pubkey(SYSTEM_PROGRAM_ID)?;
    let associated_token_program = pubkey(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    let fee_program = pubkey(PUMP_FEE_PROGRAM_ID)?;

    let pool_pubkey = pubkey(&pool.pool_address)?;
    let base_mint = pubkey(&pool.base_mint)?;
    let quote_mint = pubkey(&pool.quote_mint)?;
    let pool_base_token_account = pubkey(&pool.base_vault)?;
    let pool_quote_token_account = pubkey(&pool.quote_vault)?;
    let coin_creator = pool
        .coin_creator
        .as_ref()
        .context("PumpSwap pool missing coin_creator")?;
    let coin_creator = pubkey(coin_creator)?;

    let global_config = pda(&[b"global_config"], &program);
    let event_authority = pda(&[b"__event_authority"], &program);
    let coin_creator_vault_authority = pda(&[b"creator_vault", coin_creator.as_ref()], &program);
    let global_volume_accumulator = pda(&[b"global_volume_accumulator"], &program);
    let user_volume_accumulator = pda(&[b"user_volume_accumulator", user.as_ref()], &program);
    let fee_config = pda(
        &[b"fee_config", PUMP_FEE_CONFIG_AUTHORITY.as_ref()],
        &fee_program,
    );

    Ok(PumpAmmDerivedAccounts {
        pool: pool_pubkey,
        user: *user,
        global_config,
        base_mint,
        quote_mint,
        user_base_token_account: associated_token_address(user, &base_mint, base_token_program)?,
        user_quote_token_account: associated_token_address(user, &quote_mint, quote_token_program)?,
        pool_base_token_account,
        pool_quote_token_account,
        protocol_fee_recipient: *protocol_fee_recipient,
        protocol_fee_recipient_token_account: associated_token_address(
            protocol_fee_recipient,
            &quote_mint,
            quote_token_program,
        )?,
        base_token_program: *base_token_program,
        quote_token_program: *quote_token_program,
        system_program,
        associated_token_program,
        event_authority,
        program,
        coin_creator_vault_ata: associated_token_address(
            &coin_creator_vault_authority,
            &quote_mint,
            quote_token_program,
        )?,
        coin_creator_vault_authority,
        global_volume_accumulator,
        user_volume_accumulator,
        fee_config,
        fee_program,
    })
}

pub fn pump_amm_global_config_address() -> Result<Pubkey> {
    Ok(pda(&[b"global_config"], &pubkey(PUMP_AMM_PROGRAM_ID)?))
}

pub fn derive_raydium_clmm_accounts(
    pool: &RaydiumState,
    user: &Pubkey,
    input_mint: &str,
    tick_array: &str,
) -> Result<RaydiumClmmDerivedAccounts> {
    let token_program = pubkey(TOKEN_PROGRAM_ID)?;
    derive_raydium_clmm_accounts_with_token_programs(
        pool,
        user,
        input_mint,
        &token_program,
        &token_program,
        tick_array,
    )
}

pub fn derive_raydium_clmm_accounts_with_token_programs(
    pool: &RaydiumState,
    user: &Pubkey,
    input_mint: &str,
    input_token_program: &Pubkey,
    output_token_program: &Pubkey,
    tick_array: &str,
) -> Result<RaydiumClmmDerivedAccounts> {
    let token_program = pubkey(TOKEN_PROGRAM_ID)?;
    let token_program2022 = pubkey(TOKEN_2022_PROGRAM_ID)?;
    let memo_program = pubkey(MEMO_PROGRAM_ID)?;
    let pool_state = pubkey(&pool.pool_address)?;
    let amm_config = pubkey(
        pool.amm_config
            .as_ref()
            .context("Raydium CLMM pool missing amm_config")?,
    )?;
    let observation_state = pubkey(
        pool.observation_key
            .as_ref()
            .context("Raydium CLMM pool missing observation_key")?,
    )?;
    let base_vault = pubkey(
        pool.base_vault
            .as_ref()
            .context("Raydium CLMM pool missing token_vault_0")?,
    )?;
    let quote_vault = pubkey(
        pool.quote_vault
            .as_ref()
            .context("Raydium CLMM pool missing token_vault_1")?,
    )?;
    let base_mint = pubkey(&pool.base_mint)?;
    let quote_mint = pubkey(&pool.quote_mint)?;
    let tick_array = pubkey(tick_array)?;

    let is_base_input = if input_mint == pool.base_mint {
        true
    } else if input_mint == pool.quote_mint {
        false
    } else {
        anyhow::bail!(
            "Raydium CLMM input mint {} does not match pool mints {} / {}",
            input_mint,
            pool.base_mint,
            pool.quote_mint
        );
    };

    let (input_mint, output_mint, input_vault, output_vault) = if is_base_input {
        (base_mint, quote_mint, base_vault, quote_vault)
    } else {
        (quote_mint, base_mint, quote_vault, base_vault)
    };

    Ok(RaydiumClmmDerivedAccounts {
        payer: *user,
        amm_config,
        pool_state,
        input_token_account: associated_token_address(user, &input_mint, input_token_program)?,
        output_token_account: associated_token_address(user, &output_mint, output_token_program)?,
        input_vault,
        output_vault,
        observation_state,
        token_program,
        token_program2022,
        memo_program,
        input_vault_mint: input_mint,
        output_vault_mint: output_mint,
        tick_array,
        is_base_input,
    })
}

pub fn derive_raydium_clmm_exact_input_accounts_with_token_programs(
    pool: &RaydiumState,
    user: &Pubkey,
    input_mint: &str,
    output_mint: &str,
    input_token_program: &Pubkey,
    output_token_program: &Pubkey,
    tick_array: &str,
) -> Result<RaydiumClmmDerivedAccounts> {
    let token_program = pubkey(TOKEN_PROGRAM_ID)?;
    let token_program2022 = pubkey(TOKEN_2022_PROGRAM_ID)?;
    let memo_program = pubkey(MEMO_PROGRAM_ID)?;
    let pool_state = pubkey(&pool.pool_address)?;
    let amm_config = pubkey(
        pool.amm_config
            .as_ref()
            .context("Raydium CLMM pool missing amm_config")?,
    )?;
    let observation_state = pubkey(
        pool.observation_key
            .as_ref()
            .context("Raydium CLMM pool missing observation_key")?,
    )?;
    let base_vault = pubkey(
        pool.base_vault
            .as_ref()
            .context("Raydium CLMM pool missing token_vault_0")?,
    )?;
    let quote_vault = pubkey(
        pool.quote_vault
            .as_ref()
            .context("Raydium CLMM pool missing token_vault_1")?,
    )?;
    let base_mint = pubkey(&pool.base_mint)?;
    let quote_mint = pubkey(&pool.quote_mint)?;
    let tick_array = pubkey(tick_array)?;

    let (input_mint_pubkey, output_mint_pubkey, input_vault, output_vault) =
        if input_mint == pool.base_mint && output_mint == pool.quote_mint {
            (base_mint, quote_mint, base_vault, quote_vault)
        } else if input_mint == pool.quote_mint && output_mint == pool.base_mint {
            (quote_mint, base_mint, quote_vault, base_vault)
        } else {
            anyhow::bail!(
                "Raydium CLMM exact-input mint path {} -> {} does not match pool mints {} / {}",
                input_mint,
                output_mint,
                pool.base_mint,
                pool.quote_mint
            );
        };

    Ok(RaydiumClmmDerivedAccounts {
        payer: *user,
        amm_config,
        pool_state,
        input_token_account: associated_token_address(
            user,
            &input_mint_pubkey,
            input_token_program,
        )?,
        output_token_account: associated_token_address(
            user,
            &output_mint_pubkey,
            output_token_program,
        )?,
        input_vault,
        output_vault,
        observation_state,
        token_program,
        token_program2022,
        memo_program,
        input_vault_mint: input_mint_pubkey,
        output_vault_mint: output_mint_pubkey,
        tick_array,
        is_base_input: true,
    })
}

pub fn derive_meteora_dlmm_accounts_with_token_programs(
    pool: &MeteoraState,
    user: &Pubkey,
    input_mint: &str,
    input_token_program: &Pubkey,
    output_token_program: &Pubkey,
    use_bitmap_extension: bool,
) -> Result<MeteoraDlmmDerivedAccounts> {
    let program = pubkey(METEORA_DLMM_PROGRAM_ID)?;
    let memo_program = pubkey(MEMO_PROGRAM_ID)?;
    let lb_pair = pubkey(&pool.pool_address)?;
    let token_x_mint = pubkey(&pool.token_x_mint)?;
    let token_y_mint = pubkey(&pool.token_y_mint)?;
    let reserve_x = pubkey(&pool.reserve_x)?;
    let reserve_y = pubkey(&pool.reserve_y)?;
    let oracle = pda(&[b"oracle", lb_pair.as_ref()], &program);
    let event_authority = pda(&[b"__event_authority"], &program);
    let bitmap_extension = pda(&[b"bitmap", lb_pair.as_ref()], &program);

    let swap_x_to_y = if input_mint == pool.token_x_mint {
        true
    } else if input_mint == pool.token_y_mint {
        false
    } else {
        anyhow::bail!(
            "Meteora DLMM input mint {} does not match pool mints {} / {}",
            input_mint,
            pool.token_x_mint,
            pool.token_y_mint
        );
    };

    let (input_mint, output_mint) = if swap_x_to_y {
        (token_x_mint, token_y_mint)
    } else {
        (token_y_mint, token_x_mint)
    };

    Ok(MeteoraDlmmDerivedAccounts {
        lb_pair,
        bin_array_bitmap_extension_or_program: if use_bitmap_extension {
            bitmap_extension
        } else {
            program
        },
        reserve_x,
        reserve_y,
        user_token_in: associated_token_address(user, &input_mint, input_token_program)?,
        user_token_out: associated_token_address(user, &output_mint, output_token_program)?,
        token_x_mint,
        token_y_mint,
        oracle,
        host_fee_in_or_program: program,
        user: *user,
        token_x_program: *if swap_x_to_y {
            input_token_program
        } else {
            output_token_program
        },
        token_y_program: *if swap_x_to_y {
            output_token_program
        } else {
            input_token_program
        },
        memo_program,
        event_authority,
        program,
        swap_x_to_y,
    })
}

pub fn derive_meteora_damm_v2_accounts_with_token_programs(
    pool: &MeteoraState,
    user: &Pubkey,
    token_a_program: &Pubkey,
    token_b_program: &Pubkey,
    input_mint: &str,
) -> Result<MeteoraDammV2DerivedAccounts> {
    let program = pubkey(METEORA_DAMM_V2_PROGRAM_ID)?;
    let pool_authority = pda(&[b"pool_authority"], &program);
    let event_authority = pda(&[b"__event_authority"], &program);
    let pool_pubkey = pubkey(&pool.pool_address)?;
    let token_a_mint = pubkey(&pool.token_x_mint)?;
    let token_b_mint = pubkey(&pool.token_y_mint)?;
    let token_a_vault = pubkey(&pool.reserve_x)?;
    let token_b_vault = pubkey(&pool.reserve_y)?;

    let swap_a_to_b = if input_mint == pool.token_x_mint {
        true
    } else if input_mint == pool.token_y_mint {
        false
    } else {
        anyhow::bail!(
            "Meteora DAMM v2 input mint {} does not match pool mints {} / {}",
            input_mint,
            pool.token_x_mint,
            pool.token_y_mint
        );
    };

    let (input_mint_pubkey, output_mint_pubkey) = if swap_a_to_b {
        (token_a_mint, token_b_mint)
    } else {
        (token_b_mint, token_a_mint)
    };
    let input_token_program = if swap_a_to_b {
        token_a_program
    } else {
        token_b_program
    };
    let output_token_program = if swap_a_to_b {
        token_b_program
    } else {
        token_a_program
    };

    Ok(MeteoraDammV2DerivedAccounts {
        pool_authority,
        pool: pool_pubkey,
        input_token_account: associated_token_address(
            user,
            &input_mint_pubkey,
            input_token_program,
        )?,
        output_token_account: associated_token_address(
            user,
            &output_mint_pubkey,
            output_token_program,
        )?,
        token_a_vault,
        token_b_vault,
        token_a_mint,
        token_b_mint,
        payer: *user,
        token_a_program: *token_a_program,
        token_b_program: *token_b_program,
        referral_token_account: program,
        event_authority,
        program,
        swap_a_to_b,
    })
}

pub fn derive_whirlpool_accounts_with_token_programs(
    pool: &WhirlpoolState,
    user: &Pubkey,
    input_mint: &str,
    token_program_a: &Pubkey,
    token_program_b: &Pubkey,
    tick_arrays: [Pubkey; 3],
) -> Result<WhirlpoolDerivedAccounts> {
    let memo_program = pubkey(MEMO_PROGRAM_ID)?;
    let whirlpool = pubkey(&pool.pool_address)?;
    let token_mint_a = pubkey(&pool.token_mint_a)?;
    let token_mint_b = pubkey(&pool.token_mint_b)?;
    let token_vault_a = pubkey(&pool.token_vault_a)?;
    let token_vault_b = pubkey(&pool.token_vault_b)?;
    let (oracle, _) = get_oracle_address(&whirlpool)
        .map_err(|error| anyhow::anyhow!("derive Whirlpool oracle PDA failed: {error}"))?;

    let a_to_b = if input_mint == pool.token_mint_a {
        true
    } else if input_mint == pool.token_mint_b {
        false
    } else {
        anyhow::bail!(
            "Whirlpool input mint {} does not match pool mints {} / {}",
            input_mint,
            pool.token_mint_a,
            pool.token_mint_b
        );
    };

    Ok(WhirlpoolDerivedAccounts {
        token_program_a: *token_program_a,
        token_program_b: *token_program_b,
        memo_program,
        token_authority: *user,
        whirlpool,
        token_mint_a,
        token_mint_b,
        token_owner_account_a: associated_token_address(user, &token_mint_a, token_program_a)?,
        token_vault_a,
        token_owner_account_b: associated_token_address(user, &token_mint_b, token_program_b)?,
        token_vault_b,
        tick_array0: tick_arrays[0],
        tick_array1: tick_arrays[1],
        tick_array2: tick_arrays[2],
        oracle,
        a_to_b,
    })
}

pub fn derive_meteora_dlmm_bitmap_extension_address(pool_address: &str) -> Result<Pubkey> {
    let program = pubkey(METEORA_DLMM_PROGRAM_ID)?;
    let lb_pair = pubkey(pool_address)?;
    Ok(pda(&[b"bitmap", lb_pair.as_ref()], &program))
}

pub fn build_initialize_meteora_dlmm_bitmap_extension(
    pool_address: &str,
    funder: &Pubkey,
) -> Result<Instruction> {
    let lb_pair = pubkey(pool_address)?;
    let bin_array_bitmap_extension = derive_meteora_dlmm_bitmap_extension_address(pool_address)?;

    Ok(Instruction {
        program_id: pubkey(METEORA_DLMM_PROGRAM_ID)?,
        accounts: vec![
            AccountMeta::new_readonly(lb_pair, false),
            AccountMeta::new(bin_array_bitmap_extension, false),
            AccountMeta::new(*funder, true),
            AccountMeta::new_readonly(pubkey(SYSTEM_PROGRAM_ID)?, false),
            AccountMeta::new_readonly(rent::ID, false),
        ],
        data: METEORA_DLMM_INITIALIZE_BITMAP_EXTENSION_DISCRIMINATOR.to_vec(),
    })
}

pub fn build_pump_amm_buy_exact_quote_in(
    accounts: &[Pubkey; 23],
    spendable_quote_in: u64,
    min_base_amount_out: u64,
    track_volume: bool,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&PUMP_AMM_BUY_EXACT_QUOTE_IN_DISCRIMINATOR);
    data.extend_from_slice(&spendable_quote_in.to_le_bytes());
    data.extend_from_slice(&min_base_amount_out.to_le_bytes());
    data.push(track_volume as u8);

    Ok(Instruction {
        program_id: pubkey(PUMP_AMM_PROGRAM_ID)?,
        accounts: build_pump_buy_account_metas(accounts),
        data,
    })
}

pub fn build_pump_amm_buy(
    accounts: &[Pubkey; 23],
    base_amount_out: u64,
    max_quote_amount_in: u64,
    track_volume: bool,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&PUMP_AMM_BUY_DISCRIMINATOR);
    data.extend_from_slice(&base_amount_out.to_le_bytes());
    data.extend_from_slice(&max_quote_amount_in.to_le_bytes());
    data.push(track_volume as u8);

    Ok(Instruction {
        program_id: pubkey(PUMP_AMM_PROGRAM_ID)?,
        accounts: build_pump_buy_account_metas(accounts),
        data,
    })
}

pub fn build_pump_amm_sell(
    accounts: &[Pubkey; 21],
    base_amount_in: u64,
    min_quote_amount_out: u64,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&PUMP_AMM_SELL_DISCRIMINATOR);
    data.extend_from_slice(&base_amount_in.to_le_bytes());
    data.extend_from_slice(&min_quote_amount_out.to_le_bytes());

    Ok(Instruction {
        program_id: pubkey(PUMP_AMM_PROGRAM_ID)?,
        accounts: build_pump_sell_account_metas(accounts),
        data,
    })
}

pub fn build_create_associated_token_account_idempotent(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Instruction> {
    build_create_associated_token_account_with_data(payer, owner, mint, token_program, vec![1])
}

pub fn build_create_associated_token_account(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Instruction> {
    build_create_associated_token_account_with_data(payer, owner, mint, token_program, Vec::new())
}

fn build_create_associated_token_account_with_data(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    data: Vec<u8>,
) -> Result<Instruction> {
    let associated_token_program = pubkey(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    let ata = associated_token_address(owner, mint, token_program)?;
    Ok(Instruction {
        program_id: associated_token_program,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(pubkey(SYSTEM_PROGRAM_ID)?, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data,
    })
}

pub fn build_sync_native(token_account: &Pubkey) -> Result<Instruction> {
    Ok(Instruction {
        program_id: pubkey(TOKEN_PROGRAM_ID)?,
        accounts: vec![AccountMeta::new(*token_account, false)],
        data: vec![SPL_TOKEN_SYNC_NATIVE_DISCRIMINATOR],
    })
}

pub fn build_raydium_clmm_swap(
    accounts: &[Pubkey; 10],
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    Ok(Instruction {
        program_id: pubkey(RAYDIUM_CLMM_PROGRAM_ID)?,
        accounts: build_raydium_swap_account_metas(accounts),
        data,
    })
}

pub fn build_raydium_clmm_swap_v2(
    accounts: &[Pubkey; 13],
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    Ok(Instruction {
        program_id: pubkey(RAYDIUM_CLMM_PROGRAM_ID)?,
        accounts: build_raydium_swap_v2_account_metas(accounts),
        data,
    })
}

pub fn derive_raydium_clmm_tickarray_bitmap_extension(pool_state: &Pubkey) -> Result<Pubkey> {
    let program_id = pubkey(RAYDIUM_CLMM_PROGRAM_ID)?;
    let (address, _) = Pubkey::find_program_address(
        &[
            RAYDIUM_CLMM_TICK_ARRAY_BITMAP_EXTENSION_SEED,
            pool_state.as_ref(),
        ],
        &program_id,
    );
    Ok(address)
}

pub fn build_raydium_clmm_swap_v2_with_remaining_tick_arrays(
    accounts: &[Pubkey; 13],
    remaining_tick_arrays: &[Pubkey],
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<Instruction> {
    let mut instruction = build_raydium_clmm_swap_v2(
        accounts,
        amount,
        other_amount_threshold,
        sqrt_price_limit_x64,
        is_base_input,
    )?;
    instruction.accounts.extend(
        remaining_tick_arrays
            .iter()
            .map(|tick_array| AccountMeta::new(*tick_array, false)),
    );
    let bitmap_extension = derive_raydium_clmm_tickarray_bitmap_extension(&accounts[2])?;
    instruction
        .accounts
        .push(AccountMeta::new_readonly(bitmap_extension, false));
    Ok(instruction)
}

pub fn build_raydium_clmm_swap_v2_with_bitmap_extension_and_remaining_tick_arrays(
    accounts: &[Pubkey; 13],
    bitmap_extension: &Pubkey,
    remaining_tick_arrays: &[Pubkey],
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<Instruction> {
    let mut instruction = build_raydium_clmm_swap_v2(
        accounts,
        amount,
        other_amount_threshold,
        sqrt_price_limit_x64,
        is_base_input,
    )?;
    instruction.accounts.extend(
        remaining_tick_arrays
            .iter()
            .map(|tick_array| AccountMeta::new(*tick_array, false)),
    );
    instruction
        .accounts
        .push(AccountMeta::new_readonly(*bitmap_extension, false));
    Ok(instruction)
}

pub fn build_meteora_dlmm_swap_v2(
    accounts: &[Pubkey; 16],
    remaining_bin_arrays: &[Pubkey],
    amount_in: u64,
    min_amount_out: u64,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(28);
    data.extend_from_slice(&METEORA_DLMM_SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    // RemainingAccountsInfo { slices: vec![] }
    data.extend_from_slice(&0u32.to_le_bytes());

    let mut instruction = Instruction {
        program_id: pubkey(METEORA_DLMM_PROGRAM_ID)?,
        accounts: build_meteora_swap_v2_account_metas(accounts),
        data,
    };
    instruction.accounts.extend(
        remaining_bin_arrays
            .iter()
            .map(|bin_array| AccountMeta::new(*bin_array, false)),
    );
    Ok(instruction)
}

pub fn build_meteora_damm_v2_swap_v2(
    accounts: &[Pubkey; 14],
    amount_in: u64,
    min_amount_out: u64,
) -> Result<Instruction> {
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&METEORA_DLMM_SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.push(0);

    Ok(Instruction {
        program_id: pubkey(METEORA_DAMM_V2_PROGRAM_ID)?,
        accounts: build_meteora_damm_v2_swap_v2_account_metas(accounts),
        data,
    })
}

pub fn build_whirlpool_swap_v2(
    accounts: &WhirlpoolDerivedAccounts,
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit: u128,
    amount_specified_is_input: bool,
) -> Result<Instruction> {
    let instruction = SwapV2 {
        token_program_a: accounts.token_program_a,
        token_program_b: accounts.token_program_b,
        memo_program: accounts.memo_program,
        token_authority: accounts.token_authority,
        whirlpool: accounts.whirlpool,
        token_mint_a: accounts.token_mint_a,
        token_mint_b: accounts.token_mint_b,
        token_owner_account_a: accounts.token_owner_account_a,
        token_vault_a: accounts.token_vault_a,
        token_owner_account_b: accounts.token_owner_account_b,
        token_vault_b: accounts.token_vault_b,
        tick_array0: accounts.tick_array0,
        tick_array1: accounts.tick_array1,
        tick_array2: accounts.tick_array2,
        oracle: accounts.oracle,
    }
    .instruction(SwapV2InstructionArgs {
        amount,
        other_amount_threshold,
        sqrt_price_limit,
        amount_specified_is_input,
        a_to_b: accounts.a_to_b,
        remaining_accounts_info: None,
    });

    Ok(Instruction {
        program_id: instruction.program_id,
        accounts: instruction
            .accounts
            .into_iter()
            .map(|account| AccountMeta {
                pubkey: account.pubkey,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
            })
            .collect(),
        data: instruction.data,
    })
}

fn build_pump_buy_account_metas(accounts: &[Pubkey; 23]) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new(accounts[0], false),
        AccountMeta::new(accounts[1], true),
        AccountMeta::new_readonly(accounts[2], false),
        AccountMeta::new_readonly(accounts[3], false),
        AccountMeta::new_readonly(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new(accounts[6], false),
        AccountMeta::new(accounts[7], false),
        AccountMeta::new(accounts[8], false),
        AccountMeta::new_readonly(accounts[9], false),
        AccountMeta::new(accounts[10], false),
        AccountMeta::new_readonly(accounts[11], false),
        AccountMeta::new_readonly(accounts[12], false),
        AccountMeta::new_readonly(accounts[13], false),
        AccountMeta::new_readonly(accounts[14], false),
        AccountMeta::new_readonly(accounts[15], false),
        AccountMeta::new_readonly(accounts[16], false),
        AccountMeta::new(accounts[17], false),
        AccountMeta::new_readonly(accounts[18], false),
        AccountMeta::new_readonly(accounts[19], false),
        AccountMeta::new(accounts[20], false),
        AccountMeta::new_readonly(accounts[21], false),
        AccountMeta::new_readonly(accounts[22], false),
    ]
}

fn build_pump_sell_account_metas(accounts: &[Pubkey; 21]) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new(accounts[0], false),
        AccountMeta::new(accounts[1], true),
        AccountMeta::new_readonly(accounts[2], false),
        AccountMeta::new_readonly(accounts[3], false),
        AccountMeta::new_readonly(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new(accounts[6], false),
        AccountMeta::new(accounts[7], false),
        AccountMeta::new(accounts[8], false),
        AccountMeta::new_readonly(accounts[9], false),
        AccountMeta::new(accounts[10], false),
        AccountMeta::new_readonly(accounts[11], false),
        AccountMeta::new_readonly(accounts[12], false),
        AccountMeta::new_readonly(accounts[13], false),
        AccountMeta::new_readonly(accounts[14], false),
        AccountMeta::new_readonly(accounts[15], false),
        AccountMeta::new_readonly(accounts[16], false),
        AccountMeta::new(accounts[17], false),
        AccountMeta::new_readonly(accounts[18], false),
        AccountMeta::new_readonly(accounts[19], false),
        AccountMeta::new_readonly(accounts[20], false),
    ]
}

fn build_raydium_swap_account_metas(accounts: &[Pubkey; 10]) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new_readonly(accounts[0], true),
        AccountMeta::new_readonly(accounts[1], false),
        AccountMeta::new(accounts[2], false),
        AccountMeta::new(accounts[3], false),
        AccountMeta::new(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new(accounts[6], false),
        AccountMeta::new(accounts[7], false),
        AccountMeta::new_readonly(accounts[8], false),
        AccountMeta::new(accounts[9], false),
    ]
}

fn build_raydium_swap_v2_account_metas(accounts: &[Pubkey; 13]) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new_readonly(accounts[0], true),
        AccountMeta::new_readonly(accounts[1], false),
        AccountMeta::new(accounts[2], false),
        AccountMeta::new(accounts[3], false),
        AccountMeta::new(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new(accounts[6], false),
        AccountMeta::new(accounts[7], false),
        AccountMeta::new_readonly(accounts[8], false),
        AccountMeta::new_readonly(accounts[9], false),
        AccountMeta::new_readonly(accounts[10], false),
        AccountMeta::new_readonly(accounts[11], false),
        AccountMeta::new_readonly(accounts[12], false),
    ]
}

fn build_meteora_swap_v2_account_metas(accounts: &[Pubkey; 16]) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new(accounts[0], false),
        AccountMeta::new_readonly(accounts[1], false),
        AccountMeta::new(accounts[2], false),
        AccountMeta::new(accounts[3], false),
        AccountMeta::new(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new_readonly(accounts[6], false),
        AccountMeta::new_readonly(accounts[7], false),
        AccountMeta::new(accounts[8], false),
        AccountMeta::new(accounts[9], false),
        AccountMeta::new_readonly(accounts[10], true),
        AccountMeta::new_readonly(accounts[11], false),
        AccountMeta::new_readonly(accounts[12], false),
        AccountMeta::new_readonly(accounts[13], false),
        AccountMeta::new_readonly(accounts[14], false),
        AccountMeta::new_readonly(accounts[15], false),
    ]
}

fn build_meteora_damm_v2_swap_v2_account_metas(accounts: &[Pubkey; 14]) -> Vec<AccountMeta> {
    let mut metas = vec![
        AccountMeta::new_readonly(accounts[0], false),
        AccountMeta::new(accounts[1], false),
        AccountMeta::new(accounts[2], false),
        AccountMeta::new(accounts[3], false),
        AccountMeta::new(accounts[4], false),
        AccountMeta::new(accounts[5], false),
        AccountMeta::new_readonly(accounts[6], false),
        AccountMeta::new_readonly(accounts[7], false),
        AccountMeta::new_readonly(accounts[8], true),
        AccountMeta::new_readonly(accounts[9], false),
        AccountMeta::new_readonly(accounts[10], false),
    ];
    if accounts[11] == accounts[13] {
        metas.push(AccountMeta::new_readonly(accounts[11], false));
    } else {
        metas.push(AccountMeta::new(accounts[11], false));
    }
    metas.push(AccountMeta::new_readonly(accounts[12], false));
    metas.push(AccountMeta::new_readonly(accounts[13], false));
    metas
}

fn pubkey(value: &str) -> Result<Pubkey> {
    Pubkey::from_str(value).with_context(|| format!("invalid pubkey {value}"))
}

fn pda(seeds: &[&[u8]], program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program_id).0
}

pub fn associated_token_address(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Pubkey> {
    let associated_token_program = pubkey(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    Ok(pda(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &associated_token_program,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys<const N: usize>() -> [Pubkey; N] {
        std::array::from_fn(|i| Pubkey::new_from_array([i as u8 + 1; 32]))
    }

    #[test]
    fn builds_pump_buy_exact_quote_in_from_official_idl_order() {
        let accounts = keys::<23>();
        let ix = build_pump_amm_buy_exact_quote_in(&accounts, 123, 456, true).unwrap();

        assert_eq!(ix.program_id, pubkey(PUMP_AMM_PROGRAM_ID).unwrap());
        assert_eq!(
            ix.accounts.len(),
            PUMP_AMM_BUY_EXACT_QUOTE_IN_ACCOUNTS.len()
        );
        assert_eq!(&ix.data[..8], &PUMP_AMM_BUY_EXACT_QUOTE_IN_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 123);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 456);
        assert_eq!(ix.data[24], 1);
        assert!(ix.accounts[1].is_signer);
        assert!(ix.accounts[0].is_writable);
        assert!(!ix.accounts[2].is_writable);
        assert!(ix.accounts[20].is_writable);
    }

    #[test]
    fn builds_pump_buy_from_official_idl_order() {
        let accounts = keys::<23>();
        let ix = build_pump_amm_buy(&accounts, 111, 222, false).unwrap();

        assert_eq!(
            ix.accounts.len(),
            PUMP_AMM_BUY_EXACT_QUOTE_IN_ACCOUNTS.len()
        );
        assert_eq!(&ix.data[..8], &PUMP_AMM_BUY_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 111);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 222);
        assert_eq!(ix.data[24], 0);
        assert!(ix.accounts[1].is_signer);
        assert!(ix.accounts[20].is_writable);
    }

    #[test]
    fn builds_pump_sell_from_official_idl_order() {
        let accounts = keys::<21>();
        let ix = build_pump_amm_sell(&accounts, 789, 321).unwrap();

        assert_eq!(ix.accounts.len(), PUMP_AMM_SELL_ACCOUNTS.len());
        assert_eq!(&ix.data[..8], &PUMP_AMM_SELL_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 789);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 321);
        assert!(ix.accounts[1].is_signer);
        assert!(ix.accounts[17].is_writable);
        assert!(!ix.accounts[19].is_writable);
    }

    #[test]
    fn builds_raydium_clmm_swap_from_official_idl_order() {
        let accounts = keys::<10>();
        let ix = build_raydium_clmm_swap(&accounts, 1000, 990, 0, true).unwrap();

        assert_eq!(ix.program_id, pubkey(RAYDIUM_CLMM_PROGRAM_ID).unwrap());
        assert_eq!(ix.accounts.len(), RAYDIUM_CLMM_SWAP_ACCOUNTS.len());
        assert_eq!(&ix.data[..8], &RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 1000);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 990);
        assert_eq!(u128::from_le_bytes(ix.data[24..40].try_into().unwrap()), 0);
        assert_eq!(ix.data[40], 1);
        assert!(ix.accounts[0].is_signer);
        assert!(!ix.accounts[0].is_writable);
        assert!(ix.accounts[2].is_writable);
        assert!(ix.accounts[9].is_writable);
    }

    #[test]
    fn derives_pump_amm_accounts_for_buy_and_sell_builders() {
        let program = pubkey(PUMP_AMM_PROGRAM_ID).unwrap();
        let token_program = pubkey(TOKEN_PROGRAM_ID).unwrap();
        let pool = PumpSwapState {
            pool_address: Pubkey::new_from_array([1; 32]).to_string(),
            base_mint: Pubkey::new_from_array([2; 32]).to_string(),
            quote_mint: Pubkey::new_from_array([3; 32]).to_string(),
            base_vault: Pubkey::new_from_array([4; 32]).to_string(),
            quote_vault: Pubkey::new_from_array([5; 32]).to_string(),
            coin_creator: Some(Pubkey::new_from_array([6; 32]).to_string()),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            base_reserve: 0,
            quote_reserve: 0,
            price_history: Vec::new(),
        };
        let user = Pubkey::new_from_array([7; 32]);
        let protocol_fee_recipient = Pubkey::new_from_array([8; 32]);

        let accounts = derive_pump_amm_accounts(&pool, &user, &protocol_fee_recipient).unwrap();
        let buy_accounts = accounts.buy_exact_quote_in_accounts();
        let sell_accounts = accounts.sell_accounts();

        assert_eq!(
            buy_accounts.len(),
            PUMP_AMM_BUY_EXACT_QUOTE_IN_ACCOUNTS.len()
        );
        assert_eq!(sell_accounts.len(), PUMP_AMM_SELL_ACCOUNTS.len());
        assert_eq!(buy_accounts[1], user);
        assert_eq!(sell_accounts[1], user);
        assert_eq!(buy_accounts[2], pda(&[b"global_config"], &program));
        assert_eq!(
            buy_accounts[5],
            associated_token_address(&user, &accounts.base_mint, &token_program).unwrap()
        );
        assert_eq!(
            buy_accounts[6],
            associated_token_address(&user, &accounts.quote_mint, &token_program).unwrap()
        );
        assert_eq!(
            buy_accounts[10],
            associated_token_address(
                &protocol_fee_recipient,
                &accounts.quote_mint,
                &token_program
            )
            .unwrap()
        );
        assert_eq!(buy_accounts[15], pda(&[b"__event_authority"], &program));
        assert_eq!(buy_accounts[16], program);
        assert_eq!(sell_accounts[19], accounts.fee_config);
        assert_eq!(sell_accounts[20], accounts.fee_program);
    }

    #[test]
    fn derives_raydium_clmm_accounts_for_base_input() {
        let token_program = pubkey(TOKEN_PROGRAM_ID).unwrap();
        let user = Pubkey::new_from_array([1; 32]);
        let pool = RaydiumState {
            pool_address: Pubkey::new_from_array([2; 32]).to_string(),
            venue: crate::model::state::RaydiumVenue::Clmm,
            amm_config: Some(Pubkey::new_from_array([3; 32]).to_string()),
            base_mint: Pubkey::new_from_array([4; 32]).to_string(),
            quote_mint: Pubkey::new_from_array([5; 32]).to_string(),
            base_vault: Some(Pubkey::new_from_array([6; 32]).to_string()),
            quote_vault: Some(Pubkey::new_from_array([7; 32]).to_string()),
            observation_key: Some(Pubkey::new_from_array([8; 32]).to_string()),
            base_reserve: 0,
            quote_reserve: 0,
            base_decimals: 6,
            quote_decimals: 9,
            sqrt_price_x64: None,
            liquidity: 0,
            tick_current: None,
            tick_spacing: None,
            base_fee_owed: 0,
            quote_fee_owed: 0,
            fee_bps: 25.0,
            price_history: Vec::new(),
        };
        let tick_array = Pubkey::new_from_array([9; 32]).to_string();

        let accounts =
            derive_raydium_clmm_accounts(&pool, &user, &pool.base_mint, &tick_array).unwrap();
        let swap_accounts = accounts.swap_accounts();

        assert_eq!(swap_accounts.len(), RAYDIUM_CLMM_SWAP_ACCOUNTS.len());
        assert!(accounts.is_base_input);
        assert_eq!(swap_accounts[0], user);
        assert_eq!(swap_accounts[1].to_string(), pool.amm_config.unwrap());
        assert_eq!(swap_accounts[2].to_string(), pool.pool_address);
        assert_eq!(
            swap_accounts[3],
            associated_token_address(
                &user,
                &Pubkey::from_str(&pool.base_mint).unwrap(),
                &token_program
            )
            .unwrap()
        );
        assert_eq!(
            swap_accounts[4],
            associated_token_address(
                &user,
                &Pubkey::from_str(&pool.quote_mint).unwrap(),
                &token_program
            )
            .unwrap()
        );
        assert_eq!(swap_accounts[5].to_string(), pool.base_vault.unwrap());
        assert_eq!(swap_accounts[6].to_string(), pool.quote_vault.unwrap());
        assert_eq!(swap_accounts[9].to_string(), tick_array);
    }

    #[test]
    fn rejects_raydium_clmm_accounts_for_unrelated_input_mint() {
        let user = Pubkey::new_from_array([1; 32]);
        let pool = RaydiumState {
            pool_address: Pubkey::new_from_array([2; 32]).to_string(),
            venue: crate::model::state::RaydiumVenue::Clmm,
            amm_config: Some(Pubkey::new_from_array([3; 32]).to_string()),
            base_mint: Pubkey::new_from_array([4; 32]).to_string(),
            quote_mint: Pubkey::new_from_array([5; 32]).to_string(),
            base_vault: Some(Pubkey::new_from_array([6; 32]).to_string()),
            quote_vault: Some(Pubkey::new_from_array([7; 32]).to_string()),
            observation_key: Some(Pubkey::new_from_array([8; 32]).to_string()),
            base_reserve: 0,
            quote_reserve: 0,
            base_decimals: 6,
            quote_decimals: 9,
            sqrt_price_x64: None,
            liquidity: 0,
            tick_current: None,
            tick_spacing: None,
            base_fee_owed: 0,
            quote_fee_owed: 0,
            fee_bps: 25.0,
            price_history: Vec::new(),
        };
        let tick_array = Pubkey::new_from_array([9; 32]).to_string();
        let unrelated = Pubkey::new_from_array([10; 32]).to_string();

        assert!(derive_raydium_clmm_accounts(&pool, &user, &unrelated, &tick_array).is_err());
    }

    #[test]
    fn derives_meteora_dlmm_accounts_for_token_x_input() {
        let user = Pubkey::new_from_array([1; 32]);
        let token_program = pubkey(TOKEN_PROGRAM_ID).unwrap();
        let token_program_2022 = pubkey(TOKEN_2022_PROGRAM_ID).unwrap();
        let pool = MeteoraState {
            pool_address: Pubkey::new_from_array([2; 32]).to_string(),
            active_id: 0,
            bin_step: 25,
            base_factor: 12000,
            variable_fee_control: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            volatility_accumulator: 0,
            token_x_mint: Pubkey::new_from_array([3; 32]).to_string(),
            token_y_mint: Pubkey::new_from_array([4; 32]).to_string(),
            reserve_x: Pubkey::new_from_array([5; 32]).to_string(),
            reserve_y: Pubkey::new_from_array([6; 32]).to_string(),
            bin_array_bitmap: [0u64; 16],
            token_x_amount: 0,
            token_y_amount: 0,
            fee_bps: 30.0,
            damm_v2_pool_data: None,
            price_history: Vec::new(),
        };

        let accounts = derive_meteora_dlmm_accounts_with_token_programs(
            &pool,
            &user,
            &pool.token_x_mint,
            &token_program,
            &token_program_2022,
            false,
        )
        .unwrap();

        let swap_accounts = accounts.swap_v2_accounts();
        assert_eq!(swap_accounts.len(), METEORA_DLMM_SWAP_V2_ACCOUNTS.len());
        assert!(accounts.swap_x_to_y);
        assert_eq!(swap_accounts[0].to_string(), pool.pool_address);
        assert_eq!(swap_accounts[1], pubkey(METEORA_DLMM_PROGRAM_ID).unwrap());
        assert_eq!(
            swap_accounts[8],
            pda(
                &[b"oracle", swap_accounts[0].as_ref()],
                &pubkey(METEORA_DLMM_PROGRAM_ID).unwrap()
            )
        );
        assert_eq!(
            swap_accounts[4],
            associated_token_address(
                &user,
                &Pubkey::from_str(&pool.token_x_mint).unwrap(),
                &token_program
            )
            .unwrap()
        );
        assert_eq!(
            swap_accounts[5],
            associated_token_address(
                &user,
                &Pubkey::from_str(&pool.token_y_mint).unwrap(),
                &token_program_2022
            )
            .unwrap()
        );
    }

    #[test]
    fn builds_meteora_dlmm_swap_v2_with_remaining_bin_arrays() {
        let accounts = keys::<16>();
        let remaining = vec![
            Pubkey::new_from_array([42; 32]),
            Pubkey::new_from_array([43; 32]),
        ];
        let ix = build_meteora_dlmm_swap_v2(&accounts, &remaining, 1234, 567).unwrap();

        assert_eq!(ix.program_id, pubkey(METEORA_DLMM_PROGRAM_ID).unwrap());
        assert_eq!(&ix.data[..8], &METEORA_DLMM_SWAP_V2_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 1234);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 567);
        assert_eq!(u32::from_le_bytes(ix.data[24..28].try_into().unwrap()), 0);
        assert_eq!(
            ix.accounts.len(),
            METEORA_DLMM_SWAP_V2_ACCOUNTS.len() + remaining.len()
        );
        assert!(ix.accounts[0].is_writable);
        assert!(ix.accounts[9].is_writable);
        assert!(ix.accounts[10].is_signer);
        assert!(ix.accounts.last().unwrap().is_writable);
    }

    #[test]
    fn builds_raydium_clmm_swap_v2_with_tick_arrays_before_bitmap_extension() {
        let accounts = keys::<13>();
        let remaining = vec![
            Pubkey::new_from_array([51; 32]),
            Pubkey::new_from_array([52; 32]),
        ];
        let bitmap_extension =
            derive_raydium_clmm_tickarray_bitmap_extension(&accounts[2]).unwrap();
        let ix = build_raydium_clmm_swap_v2_with_bitmap_extension_and_remaining_tick_arrays(
            &accounts,
            &bitmap_extension,
            &remaining,
            1234,
            567,
            0,
            true,
        )
        .unwrap();

        assert_eq!(ix.accounts[13].pubkey, remaining[0]);
        assert_eq!(ix.accounts[14].pubkey, remaining[1]);
        assert_eq!(ix.accounts[15].pubkey, bitmap_extension);
        assert!(ix.accounts[13].is_writable);
        assert!(ix.accounts[14].is_writable);
        assert!(!ix.accounts[15].is_writable);
    }

    #[test]
    fn builds_raydium_clmm_swap_v2_with_auto_bitmap_extension_after_tick_arrays() {
        let accounts = keys::<13>();
        let remaining = vec![
            Pubkey::new_from_array([51; 32]),
            Pubkey::new_from_array([52; 32]),
        ];
        let ix = build_raydium_clmm_swap_v2_with_remaining_tick_arrays(
            &accounts, &remaining, 1234, 567, 0, true,
        )
        .unwrap();
        let bitmap_extension =
            derive_raydium_clmm_tickarray_bitmap_extension(&accounts[2]).unwrap();

        assert_eq!(ix.accounts[13].pubkey, remaining[0]);
        assert_eq!(ix.accounts[14].pubkey, remaining[1]);
        assert_eq!(ix.accounts[15].pubkey, bitmap_extension);
        assert!(ix.accounts[13].is_writable);
        assert!(ix.accounts[14].is_writable);
        assert!(!ix.accounts[15].is_writable);
    }

    #[test]
    fn builds_whirlpool_swap_v2_with_generated_layout() {
        let user = Pubkey::new_from_array([1; 32]);
        let accounts = WhirlpoolDerivedAccounts {
            token_program_a: pubkey(TOKEN_PROGRAM_ID).unwrap(),
            token_program_b: pubkey(TOKEN_2022_PROGRAM_ID).unwrap(),
            memo_program: pubkey(MEMO_PROGRAM_ID).unwrap(),
            token_authority: user,
            whirlpool: Pubkey::new_from_array([2; 32]),
            token_mint_a: Pubkey::new_from_array([3; 32]),
            token_mint_b: Pubkey::new_from_array([4; 32]),
            token_owner_account_a: Pubkey::new_from_array([5; 32]),
            token_vault_a: Pubkey::new_from_array([6; 32]),
            token_owner_account_b: Pubkey::new_from_array([7; 32]),
            token_vault_b: Pubkey::new_from_array([8; 32]),
            tick_array0: Pubkey::new_from_array([9; 32]),
            tick_array1: Pubkey::new_from_array([10; 32]),
            tick_array2: Pubkey::new_from_array([11; 32]),
            oracle: Pubkey::new_from_array([12; 32]),
            a_to_b: true,
        };

        let ix = build_whirlpool_swap_v2(&accounts, 1234, 567, 0, true).unwrap();
        let swap_accounts = accounts.swap_v2_accounts();

        assert_eq!(ix.program_id, pubkey(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap());
        assert_eq!(ix.accounts.len(), ORCA_WHIRLPOOL_SWAP_V2_ACCOUNTS.len());
        assert_eq!(ix.accounts[3].pubkey, user);
        assert!(ix.accounts[3].is_signer);
        assert_eq!(ix.accounts[14].pubkey, accounts.oracle);
        assert_eq!(&ix.data[..8], &[43, 4, 237, 11, 26, 201, 30, 98]);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 1234);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 567);
        assert_eq!(swap_accounts.len(), ORCA_WHIRLPOOL_SWAP_V2_ACCOUNTS.len());
    }

    #[test]
    fn builds_ata_create_idempotent_and_sync_native() {
        let payer = Pubkey::new_from_array([1; 32]);
        let owner = Pubkey::new_from_array([2; 32]);
        let mint = Pubkey::new_from_array([3; 32]);
        let token_program = pubkey(TOKEN_PROGRAM_ID).unwrap();
        let ata_ix =
            build_create_associated_token_account_idempotent(&payer, &owner, &mint, &token_program)
                .unwrap();

        assert_eq!(
            ata_ix.program_id,
            pubkey(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        );
        assert_eq!(ata_ix.data, vec![1]);
        assert!(ata_ix.accounts[0].is_signer);
        assert!(ata_ix.accounts[1].is_writable);

        let sync_ix = build_sync_native(&ata_ix.accounts[1].pubkey).unwrap();
        assert_eq!(sync_ix.program_id, token_program);
        assert_eq!(sync_ix.data, vec![SPL_TOKEN_SYNC_NATIVE_DISCRIMINATOR]);
        assert!(sync_ix.accounts[0].is_writable);
    }
}
