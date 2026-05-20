use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;
use solana_address_lookup_table_interface::{
    instruction::ProgramInstruction as LookupTableProgramInstruction,
    program as lookup_table_program,
};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{v0, AddressLookupTableAccount, Message, VersionedMessage},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::{Transaction, VersionedTransaction},
};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const RAYDIUM_AMM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
const MAX_LEGACY_TRANSACTION_SIZE_BYTES: usize = 1232;
pub const LOOKUP_TABLE_MAX_ADDRESSES: usize = 256;

// Blockhash cache to avoid frequent RPC calls
struct BlockhashCache {
    blockhash: Option<Hash>,
    timestamp: Option<Instant>,
}

struct LookupTableCache {
    entries: std::collections::HashMap<Pubkey, (Vec<Pubkey>, Instant)>,
}

// Cache duration: 10 seconds (blockhash is valid for ~150 slots, which is about 2.5 minutes)
const CACHE_DURATION: Duration = Duration::from_secs(10);

// Global blockhash cache
static BLOCKHASH_CACHE: once_cell::sync::Lazy<Arc<RwLock<BlockhashCache>>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(RwLock::new(BlockhashCache {
            blockhash: None,
            timestamp: None,
        }))
    });
static LOOKUP_TABLE_CACHE: once_cell::sync::Lazy<Arc<std::sync::RwLock<LookupTableCache>>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(std::sync::RwLock::new(LookupTableCache {
            entries: std::collections::HashMap::new(),
        }))
    });
static RPC_HTTP_CLIENT: once_cell::sync::Lazy<reqwest::Client> =
    once_cell::sync::Lazy::new(reqwest::Client::new);
static BLOCKING_RPC_HTTP_CLIENT: once_cell::sync::Lazy<reqwest::blocking::Client> =
    once_cell::sync::Lazy::new(reqwest::blocking::Client::new);
const LOOKUP_TABLE_CACHE_DURATION: Duration = Duration::from_secs(600);

pub async fn get_recent_blockhash(rpc_url: &str) -> Result<Hash> {
    // Check cache first
    {
        let cache = BLOCKHASH_CACHE.read().await;
        if let (Some(blockhash), Some(timestamp)) = (&cache.blockhash, &cache.timestamp) {
            if Instant::now().duration_since(*timestamp) < CACHE_DURATION {
                tracing::debug!("Using cached blockhash");
                return Ok(*blockhash);
            }
        }
    }

    // Cache miss, fetch from RPC
    tracing::debug!("Fetching new blockhash from RPC");
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLatestBlockhash",
        // Hot-path sending benefits from fresher blockhashes than finalized commitment.
        "params": [{"commitment": "processed"}]
    });

    let response = RPC_HTTP_CLIENT.post(rpc_url).json(&request).send().await?;
    let body: serde_json::Value = response.json().await?;

    // Check for RPC errors
    if let Some(error) = body.get("error") {
        return Err(anyhow::anyhow!("RPC error: {:?}", error));
    }

    // Extract blockhash with proper error handling
    let blockhash_str = body
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.get("blockhash"))
        .and_then(|b| b.as_str())
        .context("Failed to parse blockhash from response")?;

    tracing::debug!("Received blockhash: {}", blockhash_str);

    let blockhash_bytes = bs58::decode(blockhash_str)
        .into_vec()
        .context("Failed to decode blockhash")?;

    // Ensure the blockhash is the correct size
    if blockhash_bytes.len() != 32 {
        return Err(anyhow::anyhow!(
            "Blockhash has wrong size: expected 32 bytes, got {}",
            blockhash_bytes.len()
        ));
    }

    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(&blockhash_bytes);
    let blockhash = Hash::from(hash_bytes);

    // Update cache
    {
        let mut cache = BLOCKHASH_CACHE.write().await;
        cache.blockhash = Some(blockhash);
        cache.timestamp = Some(Instant::now());
    }

    Ok(blockhash)
}

pub fn build_pump_swap_instruction(
    wallet: &Pubkey,
    token_mint: &Pubkey,
    amount_lamports: u64,
    minimum_out: u64,
    is_buy: bool,
) -> Result<Instruction> {
    let mut instruction_data = Vec::new();

    if is_buy {
        instruction_data.extend_from_slice(&[0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea]);
    } else {
        instruction_data.extend_from_slice(&[0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad]);
    }

    instruction_data.extend_from_slice(&amount_lamports.to_le_bytes());
    instruction_data.extend_from_slice(&minimum_out.to_le_bytes());

    let program_id = Pubkey::from_str(PUMP_PROGRAM_ID)?;
    let token_program = Pubkey::from_str(TOKEN_PROGRAM)?;
    let system_program = Pubkey::from_str(SYSTEM_PROGRAM)?;

    Ok(Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(*wallet, true),
            AccountMeta::new(*token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: instruction_data,
    })
}

pub fn build_raydium_swap_instruction(
    wallet: &Pubkey,
    pool_id: &Pubkey,
    amount_lamports: u64,
    minimum_out: u64,
) -> Result<Instruction> {
    let mut instruction_data = Vec::new();
    instruction_data.push(9);
    instruction_data.extend_from_slice(&amount_lamports.to_le_bytes());
    instruction_data.extend_from_slice(&minimum_out.to_le_bytes());

    let program_id = Pubkey::from_str(RAYDIUM_AMM_V4)?;
    let token_program = Pubkey::from_str(TOKEN_PROGRAM)?;

    Ok(Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(*wallet, true),
            AccountMeta::new(*pool_id, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: instruction_data,
    })
}

pub fn build_and_sign_transaction(
    instructions: Vec<Instruction>,
    payer: &Keypair,
    recent_blockhash: Hash,
    address_lookup_tables: &[Pubkey],
    rpc_url: &str,
    compute_unit_limit: u32,
    compute_unit_price_micro_lamports: u64,
    loaded_accounts_data_size_limit: u32,
) -> Result<String> {
    let wrapped_instructions = build_compute_budget_wrapped_instructions(
        &instructions,
        compute_unit_limit,
        compute_unit_price_micro_lamports,
        Some(loaded_accounts_data_size_limit),
        true,
    )?;
    match build_and_encode_signed_transaction(
        &wrapped_instructions,
        payer,
        recent_blockhash,
        address_lookup_tables,
        rpc_url,
    ) {
        Ok(transaction) => Ok(transaction),
        Err(error) if is_serialized_transaction_size_error(&error) => {
            let retry_instructions = build_compute_budget_wrapped_instructions(
                &instructions,
                compute_unit_limit,
                compute_unit_price_micro_lamports,
                None,
                true,
            )?;
            tracing::debug!("交易尺寸贴近上限，去掉账户数据上限后重试：原因={}", error);
            match build_and_encode_signed_transaction(
                &retry_instructions,
                payer,
                recent_blockhash,
                address_lookup_tables,
                rpc_url,
            ) {
                Ok(transaction) => Ok(transaction),
                Err(retry_error) if is_serialized_transaction_size_error(&retry_error) => {
                    let minimal_instructions = build_compute_budget_wrapped_instructions(
                        &instructions,
                        compute_unit_limit,
                        compute_unit_price_micro_lamports,
                        None,
                        false,
                    )?;
                    tracing::debug!(
                        "交易尺寸仍贴近上限，去掉计算预算后重试：原因={}",
                        retry_error
                    );
                    build_and_encode_signed_transaction(
                        &minimal_instructions,
                        payer,
                        recent_blockhash,
                        address_lookup_tables,
                        rpc_url,
                    )
                }
                Err(retry_error) => Err(retry_error),
            }
        }
        Err(error) => Err(error),
    }
}

fn build_compute_budget_wrapped_instructions(
    instructions: &[Instruction],
    compute_unit_limit: u32,
    compute_unit_price_micro_lamports: u64,
    loaded_accounts_data_size_limit: Option<u32>,
    include_compute_budget: bool,
) -> Result<Vec<Instruction>> {
    let loaded_accounts_instruction_count = if loaded_accounts_data_size_limit.is_some() {
        1
    } else {
        0
    };
    let compute_budget_instruction_count = if include_compute_budget { 2 } else { 0 };
    let mut wrapped_instructions = Vec::with_capacity(
        instructions.len() + compute_budget_instruction_count + loaded_accounts_instruction_count,
    );
    if include_compute_budget {
        wrapped_instructions.push(build_set_compute_unit_limit_instruction(
            compute_unit_limit,
        )?);
        if compute_unit_price_micro_lamports > 0 {
            wrapped_instructions.push(build_set_compute_unit_price_instruction(
                compute_unit_price_micro_lamports,
            )?);
        }
    }
    if let Some(limit) = loaded_accounts_data_size_limit {
        wrapped_instructions.push(build_set_loaded_accounts_data_size_limit_instruction(
            limit,
        )?);
    }
    wrapped_instructions.extend_from_slice(instructions);
    Ok(wrapped_instructions)
}

fn build_and_encode_signed_transaction(
    wrapped_instructions: &[Instruction],
    payer: &Keypair,
    recent_blockhash: Hash,
    address_lookup_tables: &[Pubkey],
    rpc_url: &str,
) -> Result<String> {
    let message = Message::new(wrapped_instructions, Some(&payer.pubkey()));
    let mut transaction = Transaction::new_unsigned(message);
    transaction.sign(&[payer], recent_blockhash);

    let serialized = bincode::serialize(&transaction).context("序列化交易失败")?;
    if serialized.len() > MAX_LEGACY_TRANSACTION_SIZE_BYTES {
        if address_lookup_tables.is_empty() {
            anyhow::bail!(
                "serialized transaction too large: {} bytes > {} bytes",
                serialized.len(),
                MAX_LEGACY_TRANSACTION_SIZE_BYTES
            );
        }

        let lookup_table_accounts =
            fetch_address_lookup_table_accounts_blocking(rpc_url, address_lookup_tables)?;
        let versioned_message = VersionedMessage::V0(v0::Message::try_compile(
            &payer.pubkey(),
            wrapped_instructions,
            &lookup_table_accounts,
            recent_blockhash,
        )?);
        let versioned_transaction = VersionedTransaction::try_new(versioned_message, &[payer])
            .context("签名 v0 交易失败")?;
        let versioned_serialized =
            bincode::serialize(&versioned_transaction).context("序列化 v0 交易失败")?;
        if versioned_serialized.len() > MAX_LEGACY_TRANSACTION_SIZE_BYTES {
            anyhow::bail!(
                "serialized transaction too large even with lookup tables: {} bytes > {} bytes",
                versioned_serialized.len(),
                MAX_LEGACY_TRANSACTION_SIZE_BYTES
            );
        }

        use base64::{engine::general_purpose, Engine as _};
        return Ok(general_purpose::STANDARD.encode(&versioned_serialized));
    }

    use base64::{engine::general_purpose, Engine as _};
    Ok(general_purpose::STANDARD.encode(&serialized))
}

fn is_serialized_transaction_size_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .to_string()
            .contains("serialized transaction too large")
    })
}

fn build_set_compute_unit_limit_instruction(units: u32) -> Result<Instruction> {
    let mut data = Vec::with_capacity(1 + std::mem::size_of::<u32>());
    data.push(2);
    data.extend_from_slice(&units.to_le_bytes());
    Ok(Instruction {
        program_id: Pubkey::from_str(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data,
    })
}

fn build_set_compute_unit_price_instruction(micro_lamports: u64) -> Result<Instruction> {
    let mut data = Vec::with_capacity(1 + std::mem::size_of::<u64>());
    data.push(3);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Ok(Instruction {
        program_id: Pubkey::from_str(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data,
    })
}

fn build_set_loaded_accounts_data_size_limit_instruction(bytes: u32) -> Result<Instruction> {
    let mut data = Vec::with_capacity(1 + std::mem::size_of::<u32>());
    data.push(4);
    data.extend_from_slice(&bytes.to_le_bytes());
    Ok(Instruction {
        program_id: Pubkey::from_str(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data,
    })
}

pub fn fetch_address_lookup_table_accounts_blocking(
    rpc_url: &str,
    table_addresses: &[Pubkey],
) -> Result<Vec<AddressLookupTableAccount>> {
    let now = Instant::now();
    let mut out = Vec::new();

    for table_key in table_addresses {
        {
            let cache = LOOKUP_TABLE_CACHE.read().unwrap();
            if let Some((addresses, timestamp)) = cache.entries.get(table_key) {
                if now.duration_since(*timestamp) < LOOKUP_TABLE_CACHE_DURATION {
                    out.push(AddressLookupTableAccount {
                        key: *table_key,
                        addresses: addresses.clone(),
                    });
                    continue;
                }
            }
        }

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAccountInfo",
            "params": [
                table_key.to_string(),
                {
                    "encoding": "jsonParsed",
                    "commitment": "confirmed"
                }
            ]
        });
        let response = BLOCKING_RPC_HTTP_CLIENT
            .post(rpc_url)
            .json(&request)
            .send()
            .with_context(|| format!("request getAccountInfo for lookup table {}", table_key))?
            .error_for_status()
            .with_context(|| {
                format!("getAccountInfo HTTP status for lookup table {}", table_key)
            })?;
        let body: serde_json::Value = response
            .json()
            .with_context(|| format!("parse getAccountInfo for lookup table {}", table_key))?;
        let Some(value) = body.get("result").and_then(|result| result.get("value")) else {
            anyhow::bail!("lookup table {} missing result.value", table_key);
        };
        if value.is_null() {
            tracing::warn!(
                "跳过不存在的地址表：{}，如果持续出现请从 ADDRESS_LOOKUP_TABLES 移除",
                table_key
            );
            invalidate_lookup_table_cache(table_key);
            continue;
        }
        let addresses = value
            .get("data")
            .and_then(|data| data.get("parsed"))
            .and_then(|parsed| parsed.get("info"))
            .and_then(|info| info.get("addresses"))
            .and_then(serde_json::Value::as_array)
            .with_context(|| format!("lookup table {} missing parsed addresses", table_key))?
            .iter()
            .map(|address| {
                address
                    .as_str()
                    .context("lookup table address missing string")
                    .and_then(|address| Pubkey::from_str(address).map_err(Into::into))
            })
            .collect::<Result<Vec<_>>>()?;

        {
            let mut cache = LOOKUP_TABLE_CACHE.write().unwrap();
            cache.entries.insert(*table_key, (addresses.clone(), now));
        }

        out.push(AddressLookupTableAccount {
            key: *table_key,
            addresses,
        });
    }

    Ok(out)
}

pub fn lookup_table_remaining_capacity(table: &AddressLookupTableAccount) -> usize {
    LOOKUP_TABLE_MAX_ADDRESSES.saturating_sub(table.addresses.len())
}

pub fn missing_lookup_table_addresses(
    rpc_url: &str,
    table_address: &Pubkey,
    candidate_addresses: &[Pubkey],
) -> Result<Vec<Pubkey>> {
    let table = fetch_address_lookup_table_accounts_blocking(rpc_url, &[*table_address])?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("lookup table {} not found", table_address))?;
    let existing = table
        .addresses
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    Ok(candidate_addresses
        .iter()
        .copied()
        .filter(|address| !existing.contains(address))
        .collect())
}

pub fn invalidate_lookup_table_cache(table_address: &Pubkey) {
    let mut cache = LOOKUP_TABLE_CACHE.write().unwrap();
    cache.entries.remove(table_address);
}

pub fn build_and_sign_plain_transaction(
    instructions: Vec<Instruction>,
    payer: &Keypair,
    recent_blockhash: Hash,
) -> Result<String> {
    let message = Message::new(&instructions, Some(&payer.pubkey()));
    let mut transaction = Transaction::new_unsigned(message);
    transaction.sign(&[payer], recent_blockhash);
    let serialized = bincode::serialize(&transaction).context("序列化交易失败")?;
    if serialized.len() > MAX_LEGACY_TRANSACTION_SIZE_BYTES {
        anyhow::bail!(
            "serialized transaction too large: {} bytes > {} bytes",
            serialized.len(),
            MAX_LEGACY_TRANSACTION_SIZE_BYTES
        );
    }
    use base64::{engine::general_purpose, Engine as _};
    Ok(general_purpose::STANDARD.encode(&serialized))
}

pub fn build_extend_lookup_table_instruction(
    lookup_table_address: Pubkey,
    authority_address: Pubkey,
    payer_address: Pubkey,
    new_addresses: Vec<Pubkey>,
) -> Instruction {
    Instruction {
        program_id: lookup_table_program::id(),
        accounts: vec![
            AccountMeta::new(lookup_table_address, false),
            AccountMeta::new_readonly(authority_address, true),
            AccountMeta::new(payer_address, true),
            AccountMeta::new_readonly(Pubkey::from_str(SYSTEM_PROGRAM).unwrap(), false),
        ],
        data: bincode::serialize(&LookupTableProgramInstruction::ExtendLookupTable {
            new_addresses,
        })
        .expect("serialize extend lookup table instruction"),
    }
}

pub fn extract_transaction_signature(transaction_base64: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(transaction_base64)
        .context("decode signed transaction base64")?;

    if let Ok(transaction) = bincode::deserialize::<VersionedTransaction>(&bytes) {
        return transaction
            .signatures
            .first()
            .map(ToString::to_string)
            .context("versioned transaction missing signature");
    }

    if let Ok(transaction) = bincode::deserialize::<Transaction>(&bytes) {
        return transaction
            .signatures
            .first()
            .map(ToString::to_string)
            .context("legacy transaction missing signature");
    }

    anyhow::bail!("unsupported signed transaction format")
}

#[derive(Debug, Clone)]
pub struct SimulationReport {
    pub passed: bool,
    pub error: Option<serde_json::Value>,
    pub logs: Vec<String>,
    pub units_consumed: Option<u64>,
    pub transaction_base64: Option<String>,
}

impl SimulationReport {
    pub fn require_passed(&self) -> Result<()> {
        if self.passed {
            return Ok(());
        }

        anyhow::bail!("交易模拟失败：错误={:?}，日志={:?}", self.error, self.logs)
    }
}

#[derive(Debug, Deserialize)]
struct SimulateTransactionResponse {
    result: Option<SimulateTransactionResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SendTransactionResponse {
    result: Option<String>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SignatureStatusesResponse {
    result: Option<SignatureStatusesResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SignatureStatusesResult {
    value: Vec<Option<SignatureStatusValue>>,
}

#[derive(Debug, Deserialize)]
struct SignatureStatusValue {
    err: Option<serde_json::Value>,
    #[serde(rename = "confirmationStatus")]
    confirmation_status: Option<String>,
    slot: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TransactionStatusResponse {
    result: Option<TransactionStatusResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct TransactionStatusResult {
    slot: u64,
    meta: Option<TransactionStatusMeta>,
}

#[derive(Debug, Deserialize)]
struct TransactionStatusMeta {
    err: Option<serde_json::Value>,
    #[serde(rename = "logMessages")]
    log_messages: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SimulateTransactionResult {
    value: SimulateTransactionValue,
}

#[derive(Debug, Deserialize)]
struct SimulateTransactionValue {
    err: Option<serde_json::Value>,
    logs: Option<Vec<String>>,
    #[serde(rename = "unitsConsumed")]
    units_consumed: Option<u64>,
    accounts: Option<Vec<Option<SimulatedAccount>>>,
}

#[derive(Debug, Deserialize)]
struct SimulatedAccount {
    data: (String, String),
}

pub async fn simulate_transaction(
    rpc_url: &str,
    transaction_base64: &str,
) -> Result<SimulationReport> {
    Ok(
        simulate_transaction_with_accounts(rpc_url, transaction_base64, &[])
            .await?
            .0,
    )
}

pub async fn simulate_transaction_with_accounts(
    rpc_url: &str,
    transaction_base64: &str,
    account_addresses: &[String],
) -> Result<(SimulationReport, std::collections::HashMap<String, Vec<u8>>)> {
    let client = reqwest::Client::new();
    let mut config = serde_json::json!({
        "encoding": "base64",
        "commitment": "processed",
        "sigVerify": true,
        "replaceRecentBlockhash": false
    });
    if !account_addresses.is_empty() {
        config["accounts"] = serde_json::json!({
            "encoding": "base64",
            "addresses": account_addresses,
        });
    }
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": [
            transaction_base64,
            config
        ]
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("request simulateTransaction")?
        .error_for_status()
        .context("simulateTransaction HTTP status")?
        .json::<SimulateTransactionResponse>()
        .await
        .context("parse simulateTransaction response")?;

    if let Some(error) = response.error {
        return Ok((
            SimulationReport {
                passed: false,
                error: Some(error),
                logs: Vec::new(),
                units_consumed: None,
                transaction_base64: Some(transaction_base64.to_string()),
            },
            std::collections::HashMap::new(),
        ));
    }

    let value = response
        .result
        .context("simulateTransaction missing result")?
        .value;
    let passed = value.err.is_none();
    let mut accounts = std::collections::HashMap::new();
    if let Some(simulated_accounts) = value.accounts {
        for (address, account) in account_addresses.iter().zip(simulated_accounts.into_iter()) {
            let Some(account) = account else {
                continue;
            };
            if account.data.1 != "base64" {
                continue;
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(account.data.0)
                .with_context(|| format!("decode simulated account {}", address))?;
            accounts.insert(address.clone(), data);
        }
    }

    Ok((
        SimulationReport {
            passed,
            error: value.err,
            logs: value.logs.unwrap_or_default(),
            units_consumed: value.units_consumed,
            transaction_base64: Some(transaction_base64.to_string()),
        },
        accounts,
    ))
}

pub async fn send_transaction(rpc_url: &str, transaction_base64: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build sendTransaction client")?;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendTransaction",
        "params": [
            transaction_base64,
            {
                "encoding": "base64",
                "skipPreflight": true,
                "maxRetries": 3
            }
        ]
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("request sendTransaction")?
        .error_for_status()
        .context("sendTransaction HTTP status")?
        .json::<SendTransactionResponse>()
        .await
        .context("parse sendTransaction response")?;

    if let Some(error) = response.error {
        anyhow::bail!("sendTransaction RPC error: {:?}", error);
    }

    response.result.context("sendTransaction missing signature")
}

#[derive(Debug, Clone)]
pub struct ConfirmedTransaction {
    pub signature: String,
    pub slot: Option<u64>,
    pub logs: Vec<String>,
}

pub async fn send_transaction_and_confirm(
    rpc_url: &str,
    transaction_base64: &str,
) -> Result<ConfirmedTransaction> {
    let signature = send_transaction(rpc_url, transaction_base64).await?;
    wait_for_transaction_confirmation(rpc_url, &signature).await
}

pub async fn wait_for_transaction_confirmation_by_signature(
    rpc_url: &str,
    signature: &str,
) -> Result<ConfirmedTransaction> {
    wait_for_transaction_confirmation(rpc_url, signature).await
}

async fn wait_for_transaction_confirmation(
    rpc_url: &str,
    signature: &str,
) -> Result<ConfirmedTransaction> {
    wait_for_transaction_confirmation_with_config(rpc_url, signature, 120, 250).await
}

pub async fn wait_for_transaction_confirmation_quick(
    rpc_url: &str,
    signature: &str,
) -> Result<ConfirmedTransaction> {
    wait_for_transaction_confirmation_with_config(rpc_url, signature, 8, 125).await
}

async fn wait_for_transaction_confirmation_with_config(
    rpc_url: &str,
    signature: &str,
    confirm_attempts: usize,
    confirm_poll_ms: u64,
) -> Result<ConfirmedTransaction> {
    let client = reqwest::Client::new();
    let mut last_confirmation_status = None;
    let mut last_slot = None;

    for attempt in 0..confirm_attempts {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [[signature], {"searchTransactionHistory": true}]
        });
        let response = client
            .post(rpc_url)
            .json(&request)
            .send()
            .await
            .context("request getSignatureStatuses")?
            .error_for_status()
            .context("getSignatureStatuses HTTP status")?
            .json::<SignatureStatusesResponse>()
            .await
            .context("parse getSignatureStatuses response")?;

        if let Some(error) = response.error {
            anyhow::bail!("getSignatureStatuses RPC error: {}", error);
        }

        let status = response
            .result
            .context("getSignatureStatuses missing result")?
            .value
            .into_iter()
            .next()
            .flatten();

        if let Some(status) = status {
            let details = fetch_transaction_status(rpc_url, signature)
                .await
                .ok()
                .flatten();
            let logs = details
                .as_ref()
                .and_then(|details| details.meta.as_ref())
                .and_then(|meta| meta.log_messages.clone())
                .unwrap_or_default();
            let slot = details.as_ref().map(|details| details.slot).or(status.slot);
            let err = details
                .as_ref()
                .and_then(|details| details.meta.as_ref())
                .and_then(|meta| meta.err.clone())
                .or(status.err.clone());
            last_confirmation_status = status.confirmation_status.clone();
            last_slot = slot;

            if let Some(err) = err {
                anyhow::bail!(
                    "transaction confirmed failed on-chain: signature={} slot={:?} error={:?} logs={:?}",
                    signature,
                    slot,
                    err,
                    logs
                );
            }

            // A successful `getTransaction` response at `confirmed` commitment is already
            // sufficient proof that the transaction landed, even if `confirmationStatus`
            // is temporarily absent from `getSignatureStatuses`.
            if details.is_some() || is_sufficient_confirmation_status(&status) {
                return Ok(ConfirmedTransaction {
                    signature: signature.to_string(),
                    slot,
                    logs,
                });
            }
        }

        if attempt + 1 < confirm_attempts {
            tokio::time::sleep(Duration::from_millis(confirm_poll_ms)).await;
        }
    }

    anyhow::bail!(
        "transaction confirmation timed out: signature={} after {} polls last_status={:?} last_slot={:?}",
        signature,
        confirm_attempts,
        last_confirmation_status,
        last_slot
    )
}

fn is_sufficient_confirmation_status(status: &SignatureStatusValue) -> bool {
    matches!(
        status.confirmation_status.as_deref(),
        Some("confirmed") | Some("finalized")
    )
}

async fn fetch_transaction_status(
    rpc_url: &str,
    signature: &str,
) -> Result<Option<TransactionStatusResult>> {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0
            }
        ]
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("request getTransaction")?
        .error_for_status()
        .context("getTransaction HTTP status")?
        .json::<TransactionStatusResponse>()
        .await
        .context("parse getTransaction response")?;

    if let Some(error) = response.error {
        anyhow::bail!("getTransaction RPC error: {}", error);
    }

    Ok(response.result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_successful_simulation_response() {
        let body = r#"{
            "jsonrpc": "2.0",
            "result": {
                "value": {
                    "err": null,
                    "logs": ["Program log: ok"],
                    "unitsConsumed": 12345
                }
            },
            "id": 1
        }"#;

        let response: SimulateTransactionResponse = serde_json::from_str(body).unwrap();
        let value = response.result.unwrap().value;

        assert!(value.err.is_none());
        assert_eq!(value.logs.unwrap(), vec!["Program log: ok"]);
        assert_eq!(value.units_consumed, Some(12345));
    }

    #[test]
    fn failed_simulation_report_is_rejected() {
        let report = SimulationReport {
            passed: false,
            error: Some(serde_json::json!({"InstructionError": [0, "Custom"]})),
            logs: vec!["failed".to_string()],
            units_consumed: Some(10),
            transaction_base64: Some("tx_base64".to_string()),
        };

        assert!(report.require_passed().is_err());
    }

    #[test]
    fn signs_small_transaction() {
        let payer = Keypair::new();
        let blockhash = Hash::new_unique();
        let instruction = solana_system_interface::instruction::transfer(
            &payer.pubkey(),
            &Pubkey::new_unique(),
            1,
        );

        let tx = build_and_sign_transaction(
            vec![instruction],
            &payer,
            blockhash,
            &[],
            "http://localhost:8899",
            600_000,
            1,
            131_072,
        )
        .unwrap();

        assert!(!tx.is_empty());
    }

    #[test]
    fn sufficient_confirmation_status_accepts_confirmed_and_finalized_only() {
        let confirmed = SignatureStatusValue {
            err: None,
            confirmation_status: Some("confirmed".to_string()),
            slot: Some(1),
        };
        let finalized = SignatureStatusValue {
            err: None,
            confirmation_status: Some("finalized".to_string()),
            slot: Some(2),
        };
        let processed = SignatureStatusValue {
            err: None,
            confirmation_status: Some("processed".to_string()),
            slot: Some(3),
        };
        let missing = SignatureStatusValue {
            err: None,
            confirmation_status: None,
            slot: Some(4),
        };

        assert!(is_sufficient_confirmation_status(&confirmed));
        assert!(is_sufficient_confirmation_status(&finalized));
        assert!(!is_sufficient_confirmation_status(&processed));
        assert!(!is_sufficient_confirmation_status(&missing));
    }
}
