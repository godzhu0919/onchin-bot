#![allow(dead_code)]

mod model {
    pub mod state {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/state.rs"));
    }
}

mod parser {
    pub mod pumpswap {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/parser/pumpswap.rs"
        ));
    }
}

mod executor {
    pub mod swap {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/executor/swap.rs"));
    }
}

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

const RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const USER: &str = "7nKjMSrfaW32An9LvPPRCpHvEn2zxJc9KsDe7eosYsnb";
const GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";
const POOL: &str = "6KEMSWKzvqGC5cACxPrjMqWfJG9oWq7MZm2Hk5oHauLK";

const TX_ACCOUNTS: [&str; 24] = [
    "6KEMSWKzvqGC5cACxPrjMqWfJG9oWq7MZm2Hk5oHauLK",
    "7nKjMSrfaW32An9LvPPRCpHvEn2zxJc9KsDe7eosYsnb",
    "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw",
    "3QYMtgn5aJzYLdAUBC8hFx4hmGDiHoyJTYUTsnPspump",
    "So11111111111111111111111111111111111111112",
    "2EKoRBxm19upveNMrfxLMEzUy18pvnXFtvsEpS59vK57",
    "6JUXPNQL9X63CdkaaomyCsyrdnHEjrjCRQm6fnP6aWCb",
    "DtBUTRPqM3uYWZ1F6CszmjWd3MyDTwJ2kS2Tw5t2iNXh",
    "3azbXrBR7qPt29qY3cHSPj22LQ6R48fhVfg3pNYbwtJN",
    "9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz",
    "Bvtgim23rfocUzxVX9j9QFxTbBnH8JZxnaGLCEkXvjKS",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    "11111111111111111111111111111111",
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    "GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR",
    "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
    "56d58fZZDWJcN3LBsFbCXWBvD1F4gvhAavwRJavejPsA",
    "9ut1dQynSn32xwZdSH1iZZmjuWiKbm9VCtSZo5cwUkdS",
    "C2aFPdENg4A2HQsmrd5rTw5TaYBX5Ku887cWjbFKtZpw",
    "3Ef33BYriE9Sm5EEFEbGZXB34fSyu6n9D9xw3oGBtRvT",
    "5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx",
    "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ",
    "3ky7Yf6Tjmsr8rLfU6ec1hGu87ocg7amDit29txCt8Eh",
];

#[derive(Debug, serde::Deserialize)]
struct RpcResponse<T> {
    result: T,
}

#[derive(Debug, serde::Deserialize)]
struct AccountInfoResponse {
    value: AccountValue,
}

#[derive(Debug, serde::Deserialize)]
struct AccountValue {
    data: (String, String),
}

#[tokio::main]
async fn main() -> Result<()> {
    let pool_account: RpcResponse<AccountInfoResponse> = rpc_call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [POOL, { "encoding": "base64" }]
    }))
    .await?;
    let pool_data = general_purpose::STANDARD.decode(pool_account.result.value.data.0)?;
    let pool_state = parser::pumpswap::parse_pumpswap_pool(&pool_data, POOL)?;

    let fee_config_account: RpcResponse<AccountInfoResponse> = rpc_call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [GLOBAL_CONFIG, { "encoding": "base64" }]
    }))
    .await?;
    let fee_config_data =
        general_purpose::STANDARD.decode(fee_config_account.result.value.data.0)?;
    let fee_config = parser::pumpswap::parse_global_config_fee_recipients(&fee_config_data)?;
    let user = Pubkey::from_str(USER)?;
    let protocol_fee_recipient = Pubkey::from_str(TX_ACCOUNTS[9])?;
    let base_token_program = Pubkey::from_str(executor::swap::TOKEN_2022_PROGRAM_ID)?;
    let quote_token_program = Pubkey::from_str(executor::swap::TOKEN_PROGRAM_ID)?;
    let accounts = executor::swap::derive_pump_amm_accounts_with_token_programs(
        &pool_state,
        &user,
        &protocol_fee_recipient,
        &base_token_program,
        &quote_token_program,
    )?;

    let mut built_accounts: Vec<String> = accounts
        .buy_exact_quote_in_accounts()
        .iter()
        .map(ToString::to_string)
        .collect();
    let bonding_curve = derive_bonding_curve(&pool_state.base_mint)?;
    built_accounts.push(bonding_curve.to_string());
    let associated_bonding_curve_token2022 = executor::swap::associated_token_address(
        &bonding_curve,
        &Pubkey::from_str(&pool_state.base_mint)?,
        &base_token_program,
    )?;
    let associated_bonding_curve_token = executor::swap::associated_token_address(
        &bonding_curve,
        &Pubkey::from_str(&pool_state.base_mint)?,
        &Pubkey::from_str(executor::swap::TOKEN_PROGRAM_ID)?,
    )?;
    let user_volume_accumulator_wsol = executor::swap::associated_token_address(
        &accounts.user_volume_accumulator,
        &Pubkey::from_str(&pool_state.quote_mint)?,
        &quote_token_program,
    )?;
    let sharing_config = derive_sharing_config(&pool_state.base_mint)?;
    let pool_authority = derive_pool_authority(&pool_state.base_mint)?;
    let pump_user_volume_accumulator =
        derive_user_volume_accumulator("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P", USER)?;
    let pump_user_volume_accumulator_wsol = executor::swap::associated_token_address(
        &pump_user_volume_accumulator,
        &Pubkey::from_str(&pool_state.quote_mint)?,
        &quote_token_program,
    )?;

    println!("Pool state: {:?}", pool_state);
    println!("Fee config: {:?}", fee_config);
    println!("Built account count: {}", built_accounts.len());
    println!("Derived bonding_curve: {}", bonding_curve);
    println!(
        "Derived associated_bonding_curve (token2022): {}",
        associated_bonding_curve_token2022
    );
    println!(
        "Derived associated_bonding_curve (token): {}",
        associated_bonding_curve_token
    );
    println!(
        "Derived user_volume_accumulator_wsol_ata: {}",
        user_volume_accumulator_wsol
    );
    println!("Derived sharing_config: {}", sharing_config);
    println!("Derived pool_authority: {}", pool_authority);
    println!(
        "Derived pump_program_user_volume_accumulator: {}",
        pump_user_volume_accumulator
    );
    println!(
        "Derived pump_program_user_volume_accumulator_wsol_ata: {}",
        pump_user_volume_accumulator_wsol
    );
    println!("Expected remaining account: {}", TX_ACCOUNTS[23]);

    for (idx, expected) in TX_ACCOUNTS.iter().enumerate() {
        let built = built_accounts
            .get(idx)
            .map(String::as_str)
            .unwrap_or("<missing>");
        let marker = if *expected == built { "==" } else { "!!" };
        println!(
            "{:02} {} expected={} built={}",
            idx + 1,
            marker,
            expected,
            built
        );
    }

    Ok(())
}

async fn rpc_call<T: serde::de::DeserializeOwned>(body: serde_json::Value) -> Result<T> {
    let client = reqwest::Client::new();
    Ok(client
        .post(RPC_URL)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<T>()
        .await?)
}

fn derive_bonding_curve(mint: &str) -> Result<Pubkey> {
    let program_id = Pubkey::from_str("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P")?;
    let mint = Pubkey::from_str(mint)?;
    Ok(Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &program_id).0)
}

fn derive_sharing_config(mint: &str) -> Result<Pubkey> {
    let sharing_program = Pubkey::from_str("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ")?;
    let mint = Pubkey::from_str(mint)?;
    Ok(Pubkey::find_program_address(&[b"sharing-config", mint.as_ref()], &sharing_program).0)
}

fn derive_pool_authority(mint: &str) -> Result<Pubkey> {
    let program_id = Pubkey::from_str("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P")?;
    let mint = Pubkey::from_str(mint)?;
    Ok(Pubkey::find_program_address(&[b"pool-authority", mint.as_ref()], &program_id).0)
}

fn derive_user_volume_accumulator(program_id: &str, user: &str) -> Result<Pubkey> {
    let program_id = Pubkey::from_str(program_id)?;
    let user = Pubkey::from_str(user)?;
    Ok(Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &program_id).0)
}
