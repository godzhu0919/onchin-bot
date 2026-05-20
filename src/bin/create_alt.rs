use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use solana_address_lookup_table_interface::{
    instruction::{derive_lookup_table_address, ProgramInstruction},
    program as lookup_table_program,
};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use std::{env, str::FromStr, time::Duration};
use tokio::time::sleep;

const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let rpc_url = env::var("RPC_HTTP_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_string());
    let payer = parse_wallet_keypair()?;
    let payer_pubkey = payer.pubkey();
    let recent_slot = get_slot(&rpc_url).await?;
    let recent_blockhash = get_recent_blockhash(&rpc_url).await?;
    let (lookup_table_address, bump_seed) = derive_lookup_table_address(&payer_pubkey, recent_slot);

    let instruction = Instruction {
        program_id: lookup_table_program::id(),
        accounts: vec![
            AccountMeta::new(lookup_table_address, false),
            AccountMeta::new_readonly(payer_pubkey, true),
            AccountMeta::new(payer_pubkey, true),
            AccountMeta::new_readonly(Pubkey::from_str(SYSTEM_PROGRAM)?, false),
        ],
        data: bincode::serialize(&ProgramInstruction::CreateLookupTable {
            recent_slot,
            bump_seed,
        })
        .context("serialize create lookup table instruction")?,
    };

    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer_pubkey),
        &[&payer],
        recent_blockhash,
    );
    let transaction_base64 = general_purpose::STANDARD
        .encode(bincode::serialize(&transaction).context("serialize create ALT transaction")?);
    let signature = send_transaction(&rpc_url, &transaction_base64).await?;
    wait_for_confirmation(&rpc_url, &signature).await?;

    println!("ALT={}", lookup_table_address);
    println!("signature={}", signature);

    Ok(())
}

fn parse_wallet_keypair() -> Result<Keypair> {
    let private_key_str =
        env::var("WALLET_PRIVATE_KEY").context("WALLET_PRIVATE_KEY not found in environment")?;

    if private_key_str.trim().starts_with('[') {
        let bytes: Vec<u8> =
            serde_json::from_str(&private_key_str).context("parse JSON wallet key")?;
        return keypair_from_bytes(&bytes);
    }

    let bytes = bs58::decode(private_key_str.trim())
        .into_vec()
        .context("decode base58 wallet key")?;
    keypair_from_bytes(&bytes)
}

fn keypair_from_bytes(bytes: &[u8]) -> Result<Keypair> {
    match bytes.len() {
        64 => Keypair::try_from(bytes).context("create keypair from 64 bytes"),
        32 => {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(bytes);
            Ok(Keypair::new_from_array(seed))
        }
        len => anyhow::bail!("invalid wallet key length: {}", len),
    }
}

async fn get_slot(rpc_url: &str) -> Result<u64> {
    let body = rpc_request(
        rpc_url,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSlot",
            "params": [{"commitment": "finalized"}]
        }),
    )
    .await?;

    body.get("result")
        .and_then(Value::as_u64)
        .context("parse getSlot result")
}

async fn get_recent_blockhash(rpc_url: &str) -> Result<Hash> {
    let body = rpc_request(
        rpc_url,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestBlockhash",
            "params": [{"commitment": "processed"}]
        }),
    )
    .await?;

    let blockhash = body
        .get("result")
        .and_then(|value| value.get("value"))
        .and_then(|value| value.get("blockhash"))
        .and_then(Value::as_str)
        .context("parse latest blockhash")?;
    Hash::from_str(blockhash).context("parse latest blockhash string")
}

async fn send_transaction(rpc_url: &str, transaction_base64: &str) -> Result<String> {
    let body = rpc_request(
        rpc_url,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                transaction_base64,
                {
                    "encoding": "base64",
                    "skipPreflight": false,
                    "preflightCommitment": "processed",
                    "maxRetries": 5
                }
            ]
        }),
    )
    .await?;

    body.get("result")
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("parse sendTransaction signature")
}

async fn wait_for_confirmation(rpc_url: &str, signature: &str) -> Result<()> {
    for _ in 0..120 {
        let body = rpc_request(
            rpc_url,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getSignatureStatuses",
                "params": [[signature], {"searchTransactionHistory": true}]
            }),
        )
        .await?;

        let status = body
            .get("result")
            .and_then(|value| value.get("value"))
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_object);

        if let Some(status) = status {
            if let Some(error) = status.get("err").filter(|error| !error.is_null()) {
                anyhow::bail!("create ALT transaction failed: {}", error);
            }
            if status
                .get("confirmationStatus")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "confirmed" || value == "finalized")
            {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(250)).await;
    }

    anyhow::bail!("create ALT transaction not confirmed: {}", signature)
}

async fn rpc_request(rpc_url: &str, request: Value) -> Result<Value> {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .with_context(|| format!("RPC request failed: {}", rpc_url))?;
    let body: Value = response.json().await.context("parse RPC response JSON")?;
    if let Some(error) = body.get("error") {
        anyhow::bail!("RPC error: {}", error);
    }
    Ok(body)
}
