#![allow(unexpected_cfgs)]

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use thiserror::Error;

const INSTRUCTION_TAG_EXECUTE_TWO_HOP: u8 = 0;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_MIN_LEN: usize = TOKEN_ACCOUNT_AMOUNT_OFFSET + 8;
const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX: usize = 3;
const RAYDIUM_CLMM_SWAP_V2_INPUT_MINT_ACCOUNT_INDEX: usize = 11;

entrypoint!(process_instruction);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteTwoHopArgs {
    pub min_profit_raw: u64,
    pub second_leg_amount_in_offset: u16,
    pub first_leg_accounts_len: u8,
    pub second_leg_accounts_len: u8,
    pub first_leg_data: Vec<u8>,
    pub second_leg_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwoHopExecutorInstruction {
    ExecuteTwoHop(ExecuteTwoHopArgs),
}

impl TwoHopExecutorInstruction {
    pub fn unpack(input: &[u8]) -> Result<Self, ProgramError> {
        let (tag, rest) = input
            .split_first()
            .ok_or(TwoHopExecutorError::InvalidInstruction)?;
        match *tag {
            INSTRUCTION_TAG_EXECUTE_TWO_HOP => {
                let args = ExecuteTwoHopArgs::unpack(rest)?;
                Ok(Self::ExecuteTwoHop(args))
            }
            _ => Err(TwoHopExecutorError::InvalidInstruction.into()),
        }
    }
}

impl ExecuteTwoHopArgs {
    pub fn unpack(input: &[u8]) -> Result<Self, ProgramError> {
        let mut cursor = Cursor::new(input);
        let min_profit_raw = cursor.read_u64()?;
        let second_leg_amount_in_offset = cursor.read_u16()?;
        let first_leg_accounts_len = cursor.read_u8()?;
        let second_leg_accounts_len = cursor.read_u8()?;
        let first_leg_data_len = cursor.read_u16()? as usize;
        let second_leg_data_len = cursor.read_u16()? as usize;
        let first_leg_data = cursor.read_bytes(first_leg_data_len)?.to_vec();
        let second_leg_data = cursor.read_bytes(second_leg_data_len)?.to_vec();
        if cursor.remaining_len() != 0 {
            return Err(TwoHopExecutorError::InvalidInstruction.into());
        }
        Ok(Self {
            min_profit_raw,
            second_leg_amount_in_offset,
            first_leg_accounts_len,
            second_leg_accounts_len,
            first_leg_data,
            second_leg_data,
        })
    }

    pub fn pack(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            1 + 8 + 2 + 1 + 1 + 2 + 2 + self.first_leg_data.len() + self.second_leg_data.len(),
        );
        out.push(INSTRUCTION_TAG_EXECUTE_TWO_HOP);
        out.extend_from_slice(&self.min_profit_raw.to_le_bytes());
        out.extend_from_slice(&self.second_leg_amount_in_offset.to_le_bytes());
        out.push(self.first_leg_accounts_len);
        out.push(self.second_leg_accounts_len);
        out.extend_from_slice(&(self.first_leg_data.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.second_leg_data.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.first_leg_data);
        out.extend_from_slice(&self.second_leg_data);
        out
    }
}

#[repr(u32)]
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TwoHopExecutorError {
    #[error("invalid instruction data")]
    InvalidInstruction = 1,
    #[error("not enough accounts")]
    NotEnoughAccounts = 2,
    #[error("second leg amount offset out of bounds")]
    InvalidAmountOffset = 3,
    #[error("token account data too short")]
    InvalidTokenAccount = 4,
    #[error("first leg did not increase intermediate balance")]
    FirstLegDidNotIncreaseIntermediateBalance = 5,
    #[error("profit below minimum threshold")]
    ProfitBelowThreshold = 6,
}

impl From<TwoHopExecutorError> for ProgramError {
    fn from(value: TwoHopExecutorError) -> Self {
        ProgramError::Custom(value as u32 + 1)
    }
}

pub fn process_instruction<'a>(
    _program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    match TwoHopExecutorInstruction::unpack(instruction_data)? {
        TwoHopExecutorInstruction::ExecuteTwoHop(args) => execute_two_hop(accounts, args),
    }
}

fn execute_two_hop<'a>(accounts: &'a [AccountInfo<'a>], args: ExecuteTwoHopArgs) -> ProgramResult {
    msg!(
        "two_hop_executor: execute_two_hop min_profit_raw={} second_leg_amount_in_offset={} first_leg_accounts_len={} second_leg_accounts_len={}",
        args.min_profit_raw,
        args.second_leg_amount_in_offset,
        args.first_leg_accounts_len,
        args.second_leg_accounts_len
    );
    let account_info_iter = &mut accounts.iter();
    let quote_token_account = next_account_info(account_info_iter).map_err(|_| {
        msg!("two_hop_executor: missing quote token account");
        ProgramError::from(TwoHopExecutorError::NotEnoughAccounts)
    })?;
    let intermediate_token_account = next_account_info(account_info_iter).map_err(|_| {
        msg!("two_hop_executor: missing intermediate token account");
        ProgramError::from(TwoHopExecutorError::NotEnoughAccounts)
    })?;
    let first_leg_program = next_account_info(account_info_iter).map_err(|_| {
        msg!("two_hop_executor: missing first leg program account");
        ProgramError::from(TwoHopExecutorError::NotEnoughAccounts)
    })?;
    let second_leg_program = next_account_info(account_info_iter).map_err(|_| {
        msg!("two_hop_executor: missing second leg program account");
        ProgramError::from(TwoHopExecutorError::NotEnoughAccounts)
    })?;

    let remaining_accounts = account_info_iter.as_slice();
    let first_len = args.first_leg_accounts_len as usize;
    let second_len = args.second_leg_accounts_len as usize;
    if remaining_accounts.len() < first_len + second_len {
        msg!(
            "two_hop_executor: not enough remaining accounts have={} need={}",
            remaining_accounts.len(),
            first_len + second_len
        );
        return Err(TwoHopExecutorError::NotEnoughAccounts.into());
    }
    let (first_leg_accounts, tail) = remaining_accounts.split_at(first_len);
    let (second_leg_accounts, _) = tail.split_at(second_len);

    let observed_intermediate_account = observed_intermediate_token_account(
        second_leg_program,
        second_leg_accounts,
        intermediate_token_account,
    );
    let quote_before = token_account_amount(quote_token_account)?;
    let intermediate_before = token_account_amount(observed_intermediate_account)?;

    let first_instruction = Instruction {
        program_id: *first_leg_program.key,
        accounts: account_infos_to_metas(first_leg_accounts),
        data: args.first_leg_data.clone(),
    };
    let first_cpi_accounts = cpi_account_infos(first_leg_program, first_leg_accounts);
    invoke(&first_instruction, &first_cpi_accounts).map_err(|error| {
        msg!(
            "two_hop_executor: first leg CPI failed program={} error={:?}",
            first_leg_program.key,
            error
        );
        error
    })?;

    let intermediate_after = token_account_amount(observed_intermediate_account)?;
    let first_leg_delta = intermediate_after
        .checked_sub(intermediate_before)
        .ok_or_else(|| {
            msg!(
                "two_hop_executor: intermediate balance decreased before={} after={}",
                intermediate_before,
                intermediate_after
            );
            TwoHopExecutorError::FirstLegDidNotIncreaseIntermediateBalance
        })?;
    if first_leg_delta == 0 {
        msg!(
            "two_hop_executor: first leg produced zero intermediate output before={} after={}",
            intermediate_before,
            intermediate_after
        );
        return Err(TwoHopExecutorError::FirstLegDidNotIncreaseIntermediateBalance.into());
    }
    msg!(
        "two_hop_executor: first leg settled intermediate_delta={}",
        first_leg_delta
    );
    let second_leg_amount_in =
        adjusted_second_leg_amount_in(second_leg_program, second_leg_accounts, first_leg_delta)?;

    let offset = args.second_leg_amount_in_offset as usize;
    if args.second_leg_data.len() < offset + 8 {
        msg!(
            "two_hop_executor: invalid second leg amount offset offset={} second_leg_data_len={}",
            offset,
            args.second_leg_data.len()
        );
        return Err(TwoHopExecutorError::InvalidAmountOffset.into());
    }
    let mut second_leg_data = args.second_leg_data.clone();
    second_leg_data[offset..offset + 8].copy_from_slice(&second_leg_amount_in.to_le_bytes());
    let second_instruction = Instruction {
        program_id: *second_leg_program.key,
        accounts: account_infos_to_metas(second_leg_accounts),
        data: second_leg_data,
    };
    let second_cpi_accounts = cpi_account_infos(second_leg_program, second_leg_accounts);
    invoke(&second_instruction, &second_cpi_accounts).map_err(|error| {
        msg!(
            "two_hop_executor: second leg CPI failed program={} error={:?} injected_amount_in={} observed_first_leg_delta={}",
            second_leg_program.key,
            error,
            second_leg_amount_in,
            first_leg_delta
        );
        error
    })?;

    let quote_after = token_account_amount(quote_token_account)?;
    if let Some((min_quote_after, shortfall)) =
        profit_threshold_shortfall(quote_before, quote_after, args.min_profit_raw)
    {
        msg!(
            "two_hop_executor: profit below threshold quote_before={} quote_after={} min_quote_after={} shortfall={}",
            quote_before,
            quote_after,
            min_quote_after,
            shortfall
        );
        return Err(TwoHopExecutorError::ProfitBelowThreshold.into());
    }
    if quote_after >= quote_before {
        msg!(
            "two_hop_executor: success quote_before={} quote_after={} realized_profit={}",
            quote_before,
            quote_after,
            quote_after - quote_before
        );
    } else {
        msg!(
            "two_hop_executor: success quote_before={} quote_after={} realized_loss={}",
            quote_before,
            quote_after,
            quote_before - quote_after
        );
    }

    Ok(())
}

fn profit_threshold_shortfall(
    quote_before: u64,
    quote_after: u64,
    min_profit_raw: u64,
) -> Option<(u64, u64)> {
    if min_profit_raw == 0 {
        return None;
    }
    let min_quote_after = quote_before.saturating_add(min_profit_raw);
    if quote_after < min_quote_after {
        Some((min_quote_after, min_quote_after.saturating_sub(quote_after)))
    } else {
        None
    }
}

fn adjusted_second_leg_amount_in(
    second_leg_program: &AccountInfo,
    second_leg_accounts: &[AccountInfo],
    observed_first_leg_delta: u64,
) -> Result<u64, ProgramError> {
    if observed_first_leg_delta == 0 {
        return Ok(0);
    }

    if second_leg_program.key.to_string() != RAYDIUM_CLMM_PROGRAM_ID {
        return Ok(observed_first_leg_delta);
    }

    let Some(source_token_account) =
        second_leg_accounts.get(RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX)
    else {
        msg!(
            "two_hop_executor: Raydium CLMM second leg missing source token account index={}",
            RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX
        );
        return Ok(observed_first_leg_delta);
    };

    let source_amount = token_account_amount(source_token_account)?;
    let input_mint = second_leg_accounts
        .get(RAYDIUM_CLMM_SWAP_V2_INPUT_MINT_ACCOUNT_INDEX)
        .map(|account| account.key.to_string())
        .unwrap_or_else(|| "<missing>".to_string());
    let adjusted_amount = observed_first_leg_delta.min(source_amount);
    if adjusted_amount == 0 {
        msg!(
            "two_hop_executor: Raydium CLMM second leg source token account {} has zero balance after first leg; observed_delta={} input_mint={}",
            source_token_account.key,
            observed_first_leg_delta,
            input_mint
        );
        return Err(TwoHopExecutorError::FirstLegDidNotIncreaseIntermediateBalance.into());
    }
    if adjusted_amount != observed_first_leg_delta {
        msg!(
            "two_hop_executor: clamping Raydium CLMM second leg amount to source balance observed_delta={} source_balance={} adjusted_amount={} source_account={} input_mint={}",
            observed_first_leg_delta,
            source_amount,
            adjusted_amount,
            source_token_account.key,
            input_mint
        );
    } else {
        msg!(
            "two_hop_executor: using Raydium CLMM second leg amount observed_delta={} source_balance={} adjusted_amount={} source_account={} input_mint={}",
            observed_first_leg_delta,
            source_amount,
            adjusted_amount,
            source_token_account.key,
            input_mint
        );
    }
    Ok(adjusted_amount)
}

fn observed_intermediate_token_account<'a>(
    second_leg_program: &AccountInfo<'a>,
    second_leg_accounts: &'a [AccountInfo<'a>],
    fallback_intermediate_token_account: &'a AccountInfo<'a>,
) -> &'a AccountInfo<'a> {
    if second_leg_program.key.to_string() != RAYDIUM_CLMM_PROGRAM_ID {
        return fallback_intermediate_token_account;
    }

    let Some(source_token_account) =
        second_leg_accounts.get(RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX)
    else {
        msg!(
            "two_hop_executor: Raydium CLMM second leg missing source token account index={} - falling back to provided intermediate account {}",
            RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX,
            fallback_intermediate_token_account.key
        );
        return fallback_intermediate_token_account;
    };

    if source_token_account.key != fallback_intermediate_token_account.key {
        msg!(
            "two_hop_executor: overriding observed intermediate account from {} to Raydium source token account {}",
            fallback_intermediate_token_account.key,
            source_token_account.key
        );
    }

    source_token_account
}

fn cpi_account_infos<'a>(
    program: &AccountInfo<'a>,
    accounts: &[AccountInfo<'a>],
) -> Vec<AccountInfo<'a>> {
    let mut out = Vec::with_capacity(accounts.len() + 1);
    out.push(program.clone());
    out.extend(accounts.iter().cloned());
    out
}

fn account_infos_to_metas(accounts: &[AccountInfo]) -> Vec<AccountMeta> {
    accounts
        .iter()
        .map(|account| {
            if account.is_writable {
                AccountMeta::new(*account.key, account.is_signer)
            } else {
                AccountMeta::new_readonly(*account.key, account.is_signer)
            }
        })
        .collect()
}

fn token_account_amount(account: &AccountInfo) -> Result<u64, ProgramError> {
    let data = account.try_borrow_data()?;
    if data.len() < TOKEN_ACCOUNT_MIN_LEN {
        msg!(
            "two_hop_executor: invalid token account len={} key={}",
            data.len(),
            account.key
        );
        return Err(TwoHopExecutorError::InvalidTokenAccount.into());
    }
    Ok(u64::from_le_bytes(
        data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_MIN_LEN]
            .try_into()
            .map_err(|_| ProgramError::from(TwoHopExecutorError::InvalidTokenAccount))?,
    ))
}

#[derive(Debug, Clone, Copy)]
struct Cursor<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, ProgramError> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, ProgramError> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
            ProgramError::from(TwoHopExecutorError::InvalidInstruction)
        })?))
    }

    fn read_u64(&mut self) -> Result<u64, ProgramError> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
            ProgramError::from(TwoHopExecutorError::InvalidInstruction)
        })?))
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ProgramError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(TwoHopExecutorError::InvalidInstruction)?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or(TwoHopExecutorError::InvalidInstruction)?;
        self.offset = end;
        Ok(bytes)
    }

    fn remaining_len(&self) -> usize {
        self.input.len().saturating_sub(self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn account_info_with_amount(
        key: Pubkey,
        owner: Pubkey,
        amount: u64,
    ) -> (Vec<u8>, AccountInfo<'static>) {
        let lamports = Box::leak(Box::new(0u64));
        let key = Box::leak(Box::new(key));
        let owner = Box::leak(Box::new(owner));
        let mut data = vec![0u8; 165];
        data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_MIN_LEN]
            .copy_from_slice(&amount.to_le_bytes());
        let data = Box::leak(data.into_boxed_slice());
        (
            data.to_vec(),
            AccountInfo::new(key, false, true, lamports, data, owner, false),
        )
    }

    fn account_info_with_data(key: Pubkey, owner: Pubkey, data_len: usize) -> AccountInfo<'static> {
        let lamports = Box::leak(Box::new(0u64));
        let key = Box::leak(Box::new(key));
        let owner = Box::leak(Box::new(owner));
        let data = Box::leak(vec![0u8; data_len].into_boxed_slice());
        AccountInfo::new(key, false, false, lamports, data, owner, false)
    }

    #[test]
    fn packs_and_unpacks_execute_two_hop_args() {
        let args = ExecuteTwoHopArgs {
            min_profit_raw: 123,
            second_leg_amount_in_offset: 8,
            first_leg_accounts_len: 5,
            second_leg_accounts_len: 7,
            first_leg_data: vec![1, 2, 3],
            second_leg_data: vec![4, 5, 6, 7],
        };

        let packed = args.pack();
        let unpacked = TwoHopExecutorInstruction::unpack(&packed).unwrap();

        assert_eq!(unpacked, TwoHopExecutorInstruction::ExecuteTwoHop(args));
    }

    #[test]
    fn reads_token_account_amount_from_spl_layout() {
        let program = Pubkey::new_unique();
        let (_data, account) = account_info_with_amount(Pubkey::new_unique(), program, 42_123_456);
        assert_eq!(token_account_amount(&account).unwrap(), 42_123_456);
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(TwoHopExecutorError::InvalidInstruction as u32, 1);
        assert_eq!(TwoHopExecutorError::NotEnoughAccounts as u32, 2);
        assert_eq!(TwoHopExecutorError::InvalidAmountOffset as u32, 3);
        assert_eq!(TwoHopExecutorError::InvalidTokenAccount as u32, 4);
        assert_eq!(
            TwoHopExecutorError::FirstLegDidNotIncreaseIntermediateBalance as u32,
            5
        );
        assert_eq!(TwoHopExecutorError::ProfitBelowThreshold as u32, 6);
    }

    #[test]
    fn zero_min_profit_does_not_enforce_break_even() {
        assert_eq!(profit_threshold_shortfall(1_000, 900, 0), None);
    }

    #[test]
    fn positive_min_profit_enforces_threshold() {
        assert_eq!(
            profit_threshold_shortfall(1_000, 1_020, 50),
            Some((1_050, 30))
        );
        assert_eq!(profit_threshold_shortfall(1_000, 1_050, 50), None);
    }

    #[test]
    fn raydium_clmm_second_leg_amount_is_clamped_to_source_balance() {
        let raydium_program = account_info_with_data(
            Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap(),
            Pubkey::new_unique(),
            0,
        );
        let token_owner = Pubkey::new_unique();
        let (_source_data, source_account) =
            account_info_with_amount(Pubkey::new_unique(), token_owner, 50);
        let mut second_leg_accounts = (0..13)
            .map(|_| account_info_with_data(Pubkey::new_unique(), Pubkey::new_unique(), 0))
            .collect::<Vec<_>>();
        second_leg_accounts[RAYDIUM_CLMM_SWAP_V2_INPUT_TOKEN_ACCOUNT_INDEX] = source_account;

        let adjusted =
            adjusted_second_leg_amount_in(&raydium_program, &second_leg_accounts, 75).unwrap();

        assert_eq!(adjusted, 50);
    }

    #[test]
    fn non_raydium_second_leg_amount_uses_observed_delta() {
        let other_program = account_info_with_data(Pubkey::new_unique(), Pubkey::new_unique(), 0);
        let second_leg_accounts = Vec::new();

        let adjusted =
            adjusted_second_leg_amount_in(&other_program, &second_leg_accounts, 75).unwrap();

        assert_eq!(adjusted, 75);
    }
}
