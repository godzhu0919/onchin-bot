use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const TWO_HOP_EXECUTOR_EXECUTE_TAG: u8 = 0;
pub const DEFAULT_SECOND_LEG_AMOUNT_IN_OFFSET: u16 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteTwoHopArgs {
    pub min_profit_raw: u64,
    pub second_leg_amount_in_offset: u16,
    pub first_leg_data: Vec<u8>,
    pub second_leg_data: Vec<u8>,
}

impl ExecuteTwoHopArgs {
    pub fn pack(
        &self,
        first_leg_accounts_len: usize,
        second_leg_accounts_len: usize,
    ) -> Result<Vec<u8>> {
        let first_leg_accounts_len = u8::try_from(first_leg_accounts_len).map_err(|_| {
            anyhow!(
                "first leg account list too long: {}",
                first_leg_accounts_len
            )
        })?;
        let second_leg_accounts_len = u8::try_from(second_leg_accounts_len).map_err(|_| {
            anyhow!(
                "second leg account list too long: {}",
                second_leg_accounts_len
            )
        })?;
        let first_leg_data_len = u16::try_from(self.first_leg_data.len())
            .map_err(|_| anyhow!("first leg instruction data too long"))?;
        let second_leg_data_len = u16::try_from(self.second_leg_data.len())
            .map_err(|_| anyhow!("second leg instruction data too long"))?;

        let mut out = Vec::with_capacity(
            1 + 8 + 2 + 1 + 1 + 2 + 2 + self.first_leg_data.len() + self.second_leg_data.len(),
        );
        out.push(TWO_HOP_EXECUTOR_EXECUTE_TAG);
        out.extend_from_slice(&self.min_profit_raw.to_le_bytes());
        out.extend_from_slice(&self.second_leg_amount_in_offset.to_le_bytes());
        out.push(first_leg_accounts_len);
        out.push(second_leg_accounts_len);
        out.extend_from_slice(&first_leg_data_len.to_le_bytes());
        out.extend_from_slice(&second_leg_data_len.to_le_bytes());
        out.extend_from_slice(&self.first_leg_data);
        out.extend_from_slice(&self.second_leg_data);
        Ok(out)
    }
}

pub fn build_execute_two_hop_instruction(
    program_id: &Pubkey,
    quote_token_account: &Pubkey,
    intermediate_token_account: &Pubkey,
    first_instruction: &Instruction,
    second_instruction: &Instruction,
    min_profit_raw: u64,
    second_leg_amount_in_offset: u16,
) -> Result<Instruction> {
    let mut accounts = Vec::with_capacity(
        4 + first_instruction.accounts.len() + second_instruction.accounts.len(),
    );
    accounts.push(AccountMeta::new(*quote_token_account, false));
    accounts.push(AccountMeta::new(*intermediate_token_account, false));
    accounts.push(AccountMeta::new_readonly(
        first_instruction.program_id,
        false,
    ));
    accounts.push(AccountMeta::new_readonly(
        second_instruction.program_id,
        false,
    ));
    accounts.extend(first_instruction.accounts.iter().cloned());
    accounts.extend(second_instruction.accounts.iter().cloned());

    let data = ExecuteTwoHopArgs {
        min_profit_raw,
        second_leg_amount_in_offset,
        first_leg_data: first_instruction.data.clone(),
        second_leg_data: second_instruction.data.clone(),
    }
    .pack(
        first_instruction.accounts.len(),
        second_instruction.accounts.len(),
    )?;

    Ok(Instruction {
        program_id: *program_id,
        accounts,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_execute_two_hop_args() {
        let data = ExecuteTwoHopArgs {
            min_profit_raw: 123,
            second_leg_amount_in_offset: DEFAULT_SECOND_LEG_AMOUNT_IN_OFFSET,
            first_leg_data: vec![1, 2, 3],
            second_leg_data: vec![4, 5, 6],
        }
        .pack(2, 3)
        .unwrap();

        assert_eq!(data[0], TWO_HOP_EXECUTOR_EXECUTE_TAG);
        assert_eq!(u64::from_le_bytes(data[1..9].try_into().unwrap()), 123);
        assert_eq!(
            u16::from_le_bytes(data[9..11].try_into().unwrap()),
            DEFAULT_SECOND_LEG_AMOUNT_IN_OFFSET
        );
        assert_eq!(data[11], 2);
        assert_eq!(data[12], 3);
    }
}
