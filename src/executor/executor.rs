use crate::executor::{
    config::{ExecutorConfig, SendTransport},
    jito::JitoClient,
    transaction,
};
use crate::model::state::{PumpState, RaydiumState, RaydiumVenue};
use crate::rpc;
use crate::strategy::execution::ValidatedArbitrage;
use anyhow::{Context, Result};
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signer::Signer,
};
use solana_system_interface::instruction::transfer;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tokio::time::{sleep, Duration};

const DEFAULT_JITO_MIN_TIP_LAMPORTS: u64 = 1_000;

pub struct ArbitrageExecutor {
    config: ExecutorConfig,
    jito_client: Option<JitoClient>,
    lookup_table_capacity_exhausted: AtomicBool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SendReadiness {
    Ready,
    Blocked(String),
}

fn log_send_instruction_dump(
    transport_label: &str,
    instructions: &[Instruction],
    compute_unit_limit: u32,
    compute_unit_price_micro_lamports: u64,
    loaded_accounts_data_size_limit: u32,
) {
    tracing::debug!(
        "交易指令明细：发送方式={}，用户指令={}，CU上限={}，CU价格={}，账户数据上限={}",
        transport_label,
        instructions.len(),
        compute_unit_limit,
        compute_unit_price_micro_lamports,
        loaded_accounts_data_size_limit
    );
    for (index, instruction) in instructions.iter().enumerate() {
        tracing::debug!(
            "交易指令：发送方式={}，序号={}，程序={}，账户数={}，数据长度={}",
            transport_label,
            index,
            instruction.program_id,
            instruction.accounts.len(),
            instruction.data.len(),
        );
    }
}

fn minimum_jito_tip_lamports() -> u64 {
    std::env::var("JITO_MIN_TIP_LAMPORTS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_JITO_MIN_TIP_LAMPORTS)
}

fn log_verbose_enabled() -> bool {
    std::env::var("LOG_VERBOSE")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::state::{PriceSnapshot, RaydiumVenue};
    use solana_sdk::signature::Keypair;

    fn executor(enabled: bool) -> ArbitrageExecutor {
        ArbitrageExecutor::new(ExecutorConfig {
            keypair: Keypair::new(),
            max_slippage_bps: 100,
            compute_unit_limit: 1_400_000,
            compute_unit_limit_margin_bps: 1_000,
            compute_unit_limit_min_buffer: 25_000,
            compute_unit_price_micro_lamports: 1_000,
            loaded_accounts_data_size_limit: 64 * 1024 * 1024,
            enabled,
            dry_run_only: true,
            rpc_url: "http://localhost:8899".to_string(),
            address_lookup_tables: Vec::new(),
            live_send_min_profit_pct: 0.0,
            require_pre_send_simulation: true,
            send_transport: SendTransport::Rpc,
            jito_uuid: None,
            jito_tip_lamports: 0,
            two_hop_executor_program_id: None,
        })
        .unwrap()
    }

    fn executor_with_flags(enabled: bool, dry_run_only: bool) -> ArbitrageExecutor {
        ArbitrageExecutor::new(ExecutorConfig {
            keypair: Keypair::new(),
            max_slippage_bps: 100,
            compute_unit_limit: 1_400_000,
            compute_unit_limit_margin_bps: 1_000,
            compute_unit_limit_min_buffer: 25_000,
            compute_unit_price_micro_lamports: 1_000,
            loaded_accounts_data_size_limit: 64 * 1024 * 1024,
            enabled,
            dry_run_only,
            rpc_url: "http://localhost:8899".to_string(),
            address_lookup_tables: Vec::new(),
            live_send_min_profit_pct: 0.0,
            require_pre_send_simulation: true,
            send_transport: SendTransport::Rpc,
            jito_uuid: None,
            jito_tip_lamports: 0,
            two_hop_executor_program_id: None,
        })
        .unwrap()
    }

    #[test]
    fn jito_transport_initializes_without_uuid() {
        let executor = ArbitrageExecutor::new(ExecutorConfig {
            keypair: Keypair::new(),
            max_slippage_bps: 100,
            compute_unit_limit: 1_400_000,
            compute_unit_limit_margin_bps: 1_000,
            compute_unit_limit_min_buffer: 25_000,
            compute_unit_price_micro_lamports: 1_000,
            loaded_accounts_data_size_limit: 64 * 1024 * 1024,
            enabled: true,
            dry_run_only: false,
            rpc_url: "http://localhost:8899".to_string(),
            address_lookup_tables: Vec::new(),
            live_send_min_profit_pct: 0.0,
            require_pre_send_simulation: true,
            send_transport: SendTransport::JitoBundle,
            jito_uuid: None,
            jito_tip_lamports: 1_000,
            two_hop_executor_program_id: None,
        })
        .unwrap();

        assert!(executor.jito_client.is_some());
    }

    fn pump_state() -> PumpState {
        PumpState {
            sol_reserve: 1_000_000_000,
            token_reserve: 1_000_000_000,
            token_mint: "Token111111111111111111111111111111111111111".to_string(),
            price_history: Vec::<PriceSnapshot>::new(),
        }
    }

    fn raydium_state(venue: RaydiumVenue) -> RaydiumState {
        RaydiumState {
            pool_address: "pool".to_string(),
            venue,
            amm_config: None,
            base_mint: "Token111111111111111111111111111111111111111".to_string(),
            quote_mint: "So11111111111111111111111111111111111111112".to_string(),
            base_vault: None,
            quote_vault: None,
            observation_key: None,
            base_reserve: 1_000_000_000,
            quote_reserve: 1_000_000_000,
            base_decimals: 6,
            quote_decimals: 9,
            sqrt_price_x64: None,
            liquidity: 0,
            tick_current: None,
            tick_spacing: None,
            base_fee_owed: 0,
            quote_fee_owed: 0,
            fee_bps: 25.0,
            price_history: Vec::<PriceSnapshot>::new(),
        }
    }

    fn opportunity() -> ValidatedArbitrage {
        ValidatedArbitrage {
            buy_venue: "Raydium CLMM".to_string(),
            sell_venue: "PumpSwap".to_string(),
            buy_price: 0.1,
            sell_price: 0.12,
            profit_usdc: 1.0,
            profit_pct: 2.0,
            price_impact: 0.1,
            trade_size_usdc: 10.0,
        }
    }

    #[tokio::test]
    async fn blocks_clmm_execution_before_network_or_signing() {
        let err = executor(true)
            .execute_arbitrage(
                &opportunity(),
                &pump_state(),
                &raydium_state(RaydiumVenue::Clmm),
                100.0,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("暂不发送"));
        assert!(err.to_string().contains("Raydium CLMM"));
    }

    #[tokio::test]
    async fn blocks_dry_run_before_simulation_or_sending() {
        let err = executor_with_flags(true, true)
            .execute_arbitrage(
                &ValidatedArbitrage {
                    buy_venue: "Pump".to_string(),
                    sell_venue: "Raydium".to_string(),
                    ..opportunity()
                },
                &pump_state(),
                &raydium_state(RaydiumVenue::AmmV4),
                100.0,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("DRY_RUN_ONLY=true"));
    }

    #[tokio::test]
    async fn blocks_unimplemented_executable_accounts_before_simulation() {
        let err = executor_with_flags(true, false)
            .execute_arbitrage(
                &ValidatedArbitrage {
                    buy_venue: "Pump".to_string(),
                    sell_venue: "Raydium".to_string(),
                    ..opportunity()
                },
                &pump_state(),
                &raydium_state(RaydiumVenue::AmmV4),
                100.0,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("account list"));
        assert!(err.to_string().contains("simulation"));
    }

    #[tokio::test]
    async fn refuses_to_simulate_empty_instruction_list() {
        let err = executor_with_flags(true, true)
            .build_sign_and_simulate(Vec::new())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("empty instruction"));
    }

    #[test]
    fn compute_unit_limit_for_send_uses_simulated_units_plus_buffer() {
        let executor = executor(true);

        assert_eq!(
            executor.config().compute_unit_limit_for_send(Some(500_000)),
            550_000
        );
    }

    #[test]
    fn compute_unit_limit_for_send_is_capped_by_configured_ceiling() {
        let executor = ArbitrageExecutor::new(ExecutorConfig {
            keypair: Keypair::new(),
            max_slippage_bps: 100,
            compute_unit_limit: 600_000,
            compute_unit_limit_margin_bps: 1_000,
            compute_unit_limit_min_buffer: 25_000,
            compute_unit_price_micro_lamports: 1_000,
            loaded_accounts_data_size_limit: 64 * 1024 * 1024,
            enabled: true,
            dry_run_only: false,
            rpc_url: "http://localhost:8899".to_string(),
            address_lookup_tables: Vec::new(),
            live_send_min_profit_pct: 0.0,
            require_pre_send_simulation: true,
            send_transport: SendTransport::Rpc,
            jito_uuid: None,
            jito_tip_lamports: 0,
            two_hop_executor_program_id: None,
        })
        .unwrap();

        assert_eq!(
            executor.config().compute_unit_limit_for_send(Some(590_000)),
            600_000
        );
    }

    #[test]
    fn lookup_table_extension_plan_respects_remaining_capacity() {
        let existing_addresses = (0..250).map(|_| Pubkey::new_unique()).collect::<Vec<_>>();
        let missing_addresses = (0..8).map(|_| Pubkey::new_unique()).collect::<Vec<_>>();
        let table = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: existing_addresses,
        };

        let (plan, remaining_missing) =
            plan_lookup_table_extensions(&missing_addresses, &[table.clone()]);

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0, table.key);
        assert_eq!(plan[0].1.len(), 6);
        assert_eq!(remaining_missing, missing_addresses[6..].to_vec());
    }

    #[test]
    fn lookup_table_extension_plan_spans_multiple_tables() {
        let first_table = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: (0..255).map(|_| Pubkey::new_unique()).collect(),
        };
        let second_table = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: vec![Pubkey::new_unique()],
        };
        let missing_addresses = (0..3).map(|_| Pubkey::new_unique()).collect::<Vec<_>>();

        let (plan, remaining_missing) =
            plan_lookup_table_extensions(&missing_addresses, &[first_table.clone(), second_table]);

        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].0, first_table.key);
        assert_eq!(plan[0].1, vec![missing_addresses[0]]);
        assert_eq!(plan[1].1, missing_addresses[1..].to_vec());
        assert!(remaining_missing.is_empty());
    }
}

const ALT_EXTEND_CHUNK_SIZE: usize = 20;

fn plan_lookup_table_extensions(
    candidate_addresses: &[Pubkey],
    lookup_tables: &[AddressLookupTableAccount],
) -> (Vec<(Pubkey, Vec<Pubkey>)>, Vec<Pubkey>) {
    let mut existing_addresses = HashSet::new();
    for table in lookup_tables {
        existing_addresses.extend(table.addresses.iter().copied());
    }

    let mut missing_addresses = Vec::new();
    let mut seen_missing = HashSet::new();
    for &address in candidate_addresses {
        if existing_addresses.contains(&address) || !seen_missing.insert(address) {
            continue;
        }
        missing_addresses.push(address);
    }

    let mut plan = Vec::new();
    let mut cursor = 0usize;
    for table in lookup_tables {
        if cursor >= missing_addresses.len() {
            break;
        }
        let remaining_capacity = transaction::lookup_table_remaining_capacity(table);
        if remaining_capacity == 0 {
            continue;
        }
        let end = (cursor + remaining_capacity).min(missing_addresses.len());
        plan.push((table.key, missing_addresses[cursor..end].to_vec()));
        cursor = end;
    }

    (plan, missing_addresses[cursor..].to_vec())
}

impl ArbitrageExecutor {
    pub fn new(config: ExecutorConfig) -> Result<Self> {
        tracing::info!(
            "执行器已初始化：钱包={}，发送={}，模拟={}，Jito小费={} lamports，两跳程序={}",
            config.get_pubkey_base58(),
            match config.send_transport {
                SendTransport::JitoBundle => "Jito",
                SendTransport::Rpc => "RPC",
            },
            if config.require_pre_send_simulation {
                "开启"
            } else {
                "关闭"
            },
            config.jito_tip_lamports,
            config
                .two_hop_executor_program_id
                .map(|pubkey| pubkey.to_string())
                .unwrap_or_else(|| "未配置".to_string())
        );

        let jito_client = if config.send_transport == SendTransport::JitoBundle {
            tracing::info!(
                "Jito 认证：{}",
                if config.jito_uuid.is_some() {
                    "已配置"
                } else {
                    "未配置"
                }
            );
            Some(JitoClient::new(config.jito_uuid.clone())?)
        } else {
            None
        };

        if !config.address_lookup_tables.is_empty() {
            let started = Instant::now();
            match transaction::fetch_address_lookup_table_accounts_blocking(
                &config.rpc_url,
                &config.address_lookup_tables,
            ) {
                Ok(tables) => tracing::info!(
                    "地址表缓存预热完成：数量={}，耗时={}毫秒",
                    tables.len(),
                    started.elapsed().as_millis()
                ),
                Err(error) => tracing::warn!(
                    "地址表缓存预热失败，首次大交易可能变慢：数量={}，原因={}",
                    config.address_lookup_tables.len(),
                    error
                ),
            }
        }

        Ok(Self {
            config,
            jito_client,
            lookup_table_capacity_exhausted: AtomicBool::new(false),
        })
    }

    pub fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    pub fn live_send_enabled(&self) -> bool {
        self.config.enabled && !self.config.dry_run_only
    }

    pub fn has_jito_bundle_transport(&self) -> bool {
        self.jito_client.is_some()
    }

    pub fn live_send_min_profit_pct(&self) -> f64 {
        self.config.live_send_min_profit_pct
    }

    pub fn require_pre_send_simulation(&self) -> bool {
        self.config.require_pre_send_simulation
    }

    pub fn slippage_floor(&self, amount: u64) -> u64 {
        self.config.calculate_slippage(amount)
    }

    pub fn two_hop_executor_program_id(&self) -> Option<Pubkey> {
        self.config.two_hop_executor_program_id
    }

    pub fn jito_tip_lamports(&self) -> u64 {
        self.config.jito_tip_lamports
    }

    pub async fn extend_lookup_tables(&self, candidate_addresses: &[Pubkey]) -> Result<usize> {
        if self.config.address_lookup_tables.is_empty() {
            return Ok(0);
        }
        if self
            .lookup_table_capacity_exhausted
            .load(Ordering::Relaxed)
        {
            return Ok(0);
        }

        let lookup_tables = transaction::fetch_address_lookup_table_accounts_blocking(
            &self.config.rpc_url,
            &self.config.address_lookup_tables,
        )?;
        let (extension_plan, remaining_missing) =
            plan_lookup_table_extensions(candidate_addresses, &lookup_tables);
        if extension_plan.is_empty() {
            if remaining_missing.is_empty() {
                return Ok(0);
            }
            self.lookup_table_capacity_exhausted
                .store(true, Ordering::Relaxed);
            tracing::warn!(
                "地址表容量已满：已配置={}，还缺={}，本进程后续不再继续自动补充",
                lookup_tables.len(),
                remaining_missing.len()
            );
            return Ok(0);
        }

        let authority = self.config.keypair.pubkey();
        let mut added_total = 0usize;
        let mut highest_extension_slot = None;
        for (lookup_table_address, addresses_for_table) in extension_plan {
            if addresses_for_table.is_empty() {
                continue;
            }
            for chunk in addresses_for_table.chunks(ALT_EXTEND_CHUNK_SIZE) {
                let recent_blockhash =
                    transaction::get_recent_blockhash(&self.config.rpc_url).await?;
                let extend_instruction = transaction::build_extend_lookup_table_instruction(
                    lookup_table_address,
                    authority,
                    authority,
                    chunk.to_vec(),
                );
                let transaction_base64 = transaction::build_and_sign_plain_transaction(
                    vec![extend_instruction],
                    &self.config.keypair,
                    recent_blockhash,
                )?;
                let confirmed = transaction::send_transaction_and_confirm(
                    &self.config.rpc_url,
                    &transaction_base64,
                )
                .await?;
                tracing::warn!(
                    "地址表已自动补充：表={}，新增={}，签名={}",
                    lookup_table_address,
                    chunk.len(),
                    confirmed.signature
                );
                if let Some(slot) = confirmed.slot {
                    highest_extension_slot =
                        Some(highest_extension_slot.map_or(slot, |current: u64| current.max(slot)));
                }
            }
            transaction::invalidate_lookup_table_cache(&lookup_table_address);
            added_total += addresses_for_table.len();
        }

        if let Some(extension_slot) = highest_extension_slot {
            wait_for_lookup_table_warmup(&self.config.rpc_url, extension_slot).await?;
        }

        if !remaining_missing.is_empty() {
            self.lookup_table_capacity_exhausted
                .store(true, Ordering::Relaxed);
            tracing::warn!(
                "地址表容量不足：表数={}，已新增={}，还缺={}，本进程后续不再继续自动补充",
                lookup_tables.len(),
                added_total,
                remaining_missing.len()
            );
        }

        if added_total == 0 {
            return Ok(0);
        }

        Ok(added_total)
    }

    pub async fn execute_arbitrage(
        &self,
        opportunity: &ValidatedArbitrage,
        _pump_state: &PumpState,
        raydium_state: &RaydiumState,
        _sol_price: f64,
    ) -> Result<String> {
        if raydium_state.venue != RaydiumVenue::AmmV4 {
            anyhow::bail!(
                "暂不发送：{} 路线还只有监控和预检，缺少完整换币指令",
                raydium_state.venue.label()
            );
        }

        if opportunity.buy_venue != "Pump" && opportunity.sell_venue != "Pump" {
            anyhow::bail!(
                "暂不发送：暂不支持这条执行路线 {} 到 {}",
                opportunity.buy_venue,
                opportunity.sell_venue
            );
        }

        match self.send_readiness(opportunity, raydium_state) {
            SendReadiness::Ready => {}
            SendReadiness::Blocked(reason) => {
                anyhow::bail!("模拟前暂不发送：{}", reason);
            }
        }

        anyhow::bail!("实盘发送未开启：换币指令和模拟流程还需要确认")
    }

    fn send_readiness(
        &self,
        opportunity: &ValidatedArbitrage,
        raydium_state: &RaydiumState,
    ) -> SendReadiness {
        let disable_local_profit_checks = std::env::var("DISABLE_LOCAL_PROFIT_CHECKS")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true);
        if !self.config.enabled {
            return SendReadiness::Blocked("ENABLE_EXECUTION=false".to_string());
        }
        if self.config.dry_run_only {
            return SendReadiness::Blocked("DRY_RUN_ONLY=true".to_string());
        }
        if raydium_state.venue != RaydiumVenue::AmmV4 {
            return SendReadiness::Blocked(format!(
                "{} 换币指令还不完整",
                raydium_state.venue.label()
            ));
        }
        if !disable_local_profit_checks
            && (opportunity.profit_usdc <= 0.0 || opportunity.profit_pct <= 0.0)
        {
            return SendReadiness::Blocked(format!(
                "预计利润不为正：${:.4} ({:.4}%)",
                opportunity.profit_usdc, opportunity.profit_pct
            ));
        }

        SendReadiness::Blocked("Pump/Raydium account list 和 simulation 流程还不完整".to_string())
    }

    pub async fn require_simulation_passed(
        &self,
        transaction_base64: &str,
    ) -> Result<transaction::SimulationReport> {
        let report =
            transaction::simulate_transaction(&self.config.rpc_url, transaction_base64).await?;
        report.require_passed()?;
        Ok(report)
    }

    pub async fn require_simulation_passed_with_accounts(
        &self,
        transaction_base64: &str,
        account_addresses: &[String],
    ) -> Result<(
        transaction::SimulationReport,
        std::collections::HashMap<String, Vec<u8>>,
    )> {
        let (report, accounts) = transaction::simulate_transaction_with_accounts(
            &self.config.rpc_url,
            transaction_base64,
            account_addresses,
        )
        .await?;
        report.require_passed()?;
        Ok((report, accounts))
    }

    pub async fn build_sign_and_simulate(
        &self,
        instructions: Vec<Instruction>,
    ) -> Result<transaction::SimulationReport> {
        if instructions.is_empty() {
            anyhow::bail!("cannot simulate an empty instruction list");
        }

        let recent_blockhash = transaction::get_recent_blockhash(&self.config.rpc_url).await?;
        let transaction_base64 = transaction::build_and_sign_transaction(
            instructions,
            &self.config.keypair,
            recent_blockhash,
            &self.config.address_lookup_tables,
            &self.config.rpc_url,
            self.config.compute_unit_limit,
            self.config.compute_unit_price_micro_lamports,
            self.config.loaded_accounts_data_size_limit,
        )?;
        self.require_simulation_passed(&transaction_base64).await
    }

    pub async fn simulate_instructions(
        &self,
        instructions: Vec<Instruction>,
    ) -> Result<transaction::SimulationReport> {
        self.build_sign_and_simulate(instructions).await
    }

    pub async fn simulate_instructions_with_accounts(
        &self,
        instructions: Vec<Instruction>,
        account_addresses: &[String],
    ) -> Result<(
        transaction::SimulationReport,
        std::collections::HashMap<String, Vec<u8>>,
    )> {
        if instructions.is_empty() {
            anyhow::bail!("cannot simulate an empty instruction list");
        }

        let recent_blockhash = transaction::get_recent_blockhash(&self.config.rpc_url).await?;
        let transaction_base64 = transaction::build_and_sign_transaction(
            instructions,
            &self.config.keypair,
            recent_blockhash,
            &self.config.address_lookup_tables,
            &self.config.rpc_url,
            self.config.compute_unit_limit,
            self.config.compute_unit_price_micro_lamports,
            self.config.loaded_accounts_data_size_limit,
        )?;
        self.require_simulation_passed_with_accounts(&transaction_base64, account_addresses)
            .await
    }

    pub async fn send_instructions(&self, instructions: Vec<Instruction>) -> Result<String> {
        self.send_instructions_with_simulated_units(instructions, None)
            .await
    }

    async fn units_consumed_for_send_limit(
        &self,
        instructions: &[Instruction],
        recent_blockhash: Hash,
        simulated_units_consumed: Option<u64>,
        allow_estimation_failure: bool,
        context_label: &str,
    ) -> Result<Option<u64>> {
        if let Some(units) = simulated_units_consumed {
            tracing::debug!("复用模拟 CU：场景={}，CU={}", context_label, units);
            return Ok(Some(units));
        }

        if allow_estimation_failure {
            tracing::debug!(
                "发送前模拟已关闭，直接使用固定 CU：场景={}，CU={}",
                context_label,
                self.config.compute_unit_limit
            );
            return Ok(None);
        }

        let simulation_limit = self.config.compute_unit_limit.max(1);
        let simulation_base64 = match transaction::build_and_sign_transaction(
            instructions.to_vec(),
            &self.config.keypair,
            recent_blockhash,
            &self.config.address_lookup_tables,
            &self.config.rpc_url,
            simulation_limit,
            self.config.compute_unit_price_micro_lamports,
            self.config.loaded_accounts_data_size_limit,
        ) {
            Ok(transaction) => transaction,
            Err(error) if allow_estimation_failure => {
                tracing::warn!(
                    "发送前 CU 估算构建失败，使用配置上限：场景={}，原因={}",
                    context_label,
                    error
                );
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let report =
            match transaction::simulate_transaction(&self.config.rpc_url, &simulation_base64).await
            {
                Ok(report) => report,
                Err(error) if allow_estimation_failure => {
                    tracing::warn!(
                        "发送前 CU 估算 RPC 失败，使用配置上限：场景={}，原因={}",
                        context_label,
                        error
                    );
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
        if let Err(error) = report.require_passed() {
            anyhow::bail!("发送前 CU 估算模拟失败：{}", error);
        }
        let Some(units) = report.units_consumed else {
            if allow_estimation_failure {
                tracing::warn!(
                    "发送前 CU 估算缺少结果，使用配置上限：场景={}",
                    context_label
                );
                return Ok(None);
            }
            anyhow::bail!("发送前 CU 估算没有返回结果");
        };
        tracing::debug!(
            "发送前 CU 估算完成：场景={}，CU={}，模拟上限={}，日志数={}",
            context_label,
            units,
            simulation_limit,
            report.logs.len()
        );
        Ok(Some(units))
    }

    async fn build_signed_transaction_for_send(
        &self,
        instructions: Vec<Instruction>,
        recent_blockhash: Hash,
        simulated_units_consumed: Option<u64>,
        allow_estimation_failure: bool,
        context_label: &str,
        transport_label: &str,
    ) -> Result<(String, String)> {
        let units_consumed_for_limit = self
            .units_consumed_for_send_limit(
                &instructions,
                recent_blockhash,
                simulated_units_consumed,
                allow_estimation_failure,
                context_label,
            )
            .await?;
        let compute_unit_limit = self
            .config
            .compute_unit_limit_for_send(units_consumed_for_limit);
        log_send_instruction_dump(
            transport_label,
            &instructions,
            compute_unit_limit,
            self.config.compute_unit_price_micro_lamports,
            self.config.loaded_accounts_data_size_limit,
        );
        let transaction_base64 = transaction::build_and_sign_transaction(
            instructions,
            &self.config.keypair,
            recent_blockhash,
            &self.config.address_lookup_tables,
            &self.config.rpc_url,
            compute_unit_limit,
            self.config.compute_unit_price_micro_lamports,
            self.config.loaded_accounts_data_size_limit,
        )?;
        let transaction_signature =
            transaction::extract_transaction_signature(&transaction_base64)?;
        tracing::debug!(
            "发送 CU 已确定：场景={}，估算={:?}，配置上限={}，最终={}",
            context_label,
            units_consumed_for_limit,
            self.config.compute_unit_limit,
            compute_unit_limit
        );
        Ok((transaction_base64, transaction_signature))
    }

    fn build_jito_tip_instruction(&self, tip_lamports: u64) -> Result<Option<(Instruction, u64)>> {
        let tip_lamports = if tip_lamports == 0 {
            minimum_jito_tip_lamports()
        } else {
            tip_lamports
        };
        if tip_lamports == 0 {
            return Ok(None);
        }

        let tip_account = JitoClient::random_tip_account()?;
        let tip_instruction = transfer(&self.config.keypair.pubkey(), &tip_account, tip_lamports);
        Ok(Some((tip_instruction, tip_lamports)))
    }

    pub async fn send_instruction_bundle_with_simulated_units(
        &self,
        instruction_groups: Vec<Vec<Instruction>>,
        simulated_units_consumed: Vec<Option<u64>>,
    ) -> Result<String> {
        self.send_instruction_bundle_with_simulated_units_and_tip(
            instruction_groups,
            simulated_units_consumed,
            self.config.jito_tip_lamports,
        )
        .await
    }

    pub async fn send_instruction_bundle_with_simulated_units_and_tip(
        &self,
        instruction_groups: Vec<Vec<Instruction>>,
        simulated_units_consumed: Vec<Option<u64>>,
        jito_tip_lamports: u64,
    ) -> Result<String> {
        self.send_instruction_bundle_with_simulated_units_and_tip_context(
            instruction_groups,
            simulated_units_consumed,
            jito_tip_lamports,
            None,
        )
        .await
    }

    pub async fn send_instruction_bundle_with_simulated_units_and_tip_context(
        &self,
        instruction_groups: Vec<Vec<Instruction>>,
        simulated_units_consumed: Vec<Option<u64>>,
        jito_tip_lamports: u64,
        log_context: Option<&str>,
    ) -> Result<String> {
        if instruction_groups.is_empty() {
            anyhow::bail!("cannot send an empty instruction bundle");
        }
        if instruction_groups.len() > 5 {
            anyhow::bail!(
                "Jito bundle supports at most 5 transactions; got {} instruction groups",
                instruction_groups.len()
            );
        }
        if !self.live_send_enabled() {
            anyhow::bail!("live sending disabled by executor config");
        }
        let Some(jito_client) = &self.jito_client else {
            anyhow::bail!("multi-transaction setup split requires Jito bundle transport");
        };

        let total_started = Instant::now();
        let blockhash_started = Instant::now();
        let recent_blockhash = transaction::get_recent_blockhash(&self.config.rpc_url).await?;
        let blockhash_elapsed_ms = blockhash_started.elapsed().as_millis();
        let last_group_index = instruction_groups.len() - 1;
        let tip_instruction = self.build_jito_tip_instruction(jito_tip_lamports)?;
        let log_context = log_context.unwrap_or("-");
        let mut transaction_base64s = Vec::with_capacity(instruction_groups.len());
        let mut transaction_signatures = Vec::with_capacity(instruction_groups.len());

        let build_started = Instant::now();
        for (index, instructions) in instruction_groups.into_iter().enumerate() {
            if instructions.is_empty() {
                anyhow::bail!("cannot send empty instruction group {}", index);
            }
            let mut instructions = instructions;
            if index == last_group_index {
                if let Some((tip_instruction, tip_lamports)) = &tip_instruction {
                    instructions.push(tip_instruction.clone());
                    tracing::debug!(
                        "Jito 小费已并入最后一笔交易：场景=Jito bundle split send，上下文={}，lamports={}",
                        log_context,
                        tip_lamports
                    );
                }
            }
            let units_hint = simulated_units_consumed.get(index).copied().flatten();
            let allow_estimation_failure =
                units_hint.is_none() && (!self.config.require_pre_send_simulation || index > 0);
            let context_label = if log_context == "-" {
                format!("Jito bundle group {}/{}", index + 1, last_group_index + 1)
            } else {
                format!(
                    "Jito bundle group {}/{} {}",
                    index + 1,
                    last_group_index + 1,
                    log_context
                )
            };
            let (transaction_base64, transaction_signature) = self
                .build_signed_transaction_for_send(
                    instructions,
                    recent_blockhash,
                    units_hint,
                    allow_estimation_failure,
                    &context_label,
                    "Jito bundle split send",
                )
                .await?;
            transaction_base64s.push(transaction_base64);
            transaction_signatures.push(transaction_signature);
        }
        let build_elapsed_ms = build_started.elapsed().as_millis();

        let primary_signature = transaction_signatures
            .last()
            .cloned()
            .context("Jito bundle missing primary signature")?;

        let submitted_base64 = transaction_base64s.join(",");
        let submit_started = Instant::now();
        let bundle_id = jito_client.send_bundle(transaction_base64s).await?;
        let submit_elapsed_ms = submit_started.elapsed().as_millis();
        tracing::debug!(
            "Jito 拆分包已提交：bundle={}，主交易={}",
            bundle_id,
            primary_signature
        );
        for signature in &transaction_signatures {
            self.spawn_async_confirmation(signature.clone(), "Jito bundle split");
        }
        tracing::info!(
            "发送耗时明细：方式=Jito bundle split send，上下文={}，blockhash={}毫秒，构建签名={}毫秒，提交={}毫秒，总计={}毫秒，交易base64s={}",
            log_context,
            blockhash_elapsed_ms,
            build_elapsed_ms,
            submit_elapsed_ms,
            total_started.elapsed().as_millis(),
            submitted_base64
        );
        Ok(format!("{bundle_id}，base64={submitted_base64}"))
    }

    pub async fn send_instructions_with_simulated_units(
        &self,
        instructions: Vec<Instruction>,
        simulated_units_consumed: Option<u64>,
    ) -> Result<String> {
        self.send_instructions_with_simulated_units_and_tip(
            instructions,
            simulated_units_consumed,
            self.config.jito_tip_lamports,
        )
        .await
    }

    pub async fn send_instructions_with_simulated_units_and_tip(
        &self,
        instructions: Vec<Instruction>,
        simulated_units_consumed: Option<u64>,
        jito_tip_lamports: u64,
    ) -> Result<String> {
        self.send_instructions_with_simulated_units_and_tip_context(
            instructions,
            simulated_units_consumed,
            jito_tip_lamports,
            None,
        )
        .await
    }

    pub async fn send_instructions_with_simulated_units_and_tip_context(
        &self,
        instructions: Vec<Instruction>,
        simulated_units_consumed: Option<u64>,
        jito_tip_lamports: u64,
        log_context: Option<&str>,
    ) -> Result<String> {
        if instructions.is_empty() {
            anyhow::bail!("cannot send an empty instruction list");
        }
        if !self.live_send_enabled() {
            anyhow::bail!("live sending disabled by executor config");
        }

        let total_started = Instant::now();
        let blockhash_started = Instant::now();
        let recent_blockhash = transaction::get_recent_blockhash(&self.config.rpc_url).await?;
        let blockhash_elapsed_ms = blockhash_started.elapsed().as_millis();
        let transport_label = if self.jito_client.is_some() {
            "Jito bundle send"
        } else {
            "RPC send"
        };
        let log_context = log_context.unwrap_or("-");
        let mut instructions = instructions;
        let tip_instruction = if self.jito_client.is_some() {
            self.build_jito_tip_instruction(jito_tip_lamports)?
        } else {
            None
        };
        if let Some((tip_instruction, tip_lamports)) = &tip_instruction {
            instructions.push(tip_instruction.clone());
            tracing::debug!(
                "Jito 小费已并入主交易：场景={}，上下文={}，lamports={}",
                transport_label,
                log_context,
                tip_lamports
            );
        }
        let context_label = if log_context == "-" {
            transport_label.to_string()
        } else {
            format!("{} {}", transport_label, log_context)
        };
        let (transaction_base64, transaction_signature) = self
            .build_signed_transaction_for_send(
                instructions,
                recent_blockhash,
                simulated_units_consumed,
                !self.config.require_pre_send_simulation,
                &context_label,
                transport_label,
            )
            .await?;
        let build_elapsed_ms = total_started
            .elapsed()
            .as_millis()
            .saturating_sub(blockhash_elapsed_ms);
        if let Some(jito_client) = &self.jito_client {
            tracing::debug!("Jito 交易已签名：{}", transaction_signature);
            let submitted_base64 = transaction_base64.clone();
            let submit_started = Instant::now();
            let bundle_id = jito_client.send_bundle(vec![transaction_base64]).await?;
            let submit_elapsed_ms = submit_started.elapsed().as_millis();
            tracing::debug!(
                "Jito 交易包已提交：bundle={}，交易={}",
                bundle_id,
                transaction_signature
            );
            self.spawn_async_confirmation(transaction_signature.clone(), "Jito bundle");
            tracing::info!(
                "发送耗时明细：方式=Jito bundle send，上下文={}，blockhash={}毫秒，构建签名={}毫秒，提交={}毫秒，总计={}毫秒，交易base64={}",
                log_context,
                blockhash_elapsed_ms,
                build_elapsed_ms,
                submit_elapsed_ms,
                total_started.elapsed().as_millis(),
                submitted_base64
            );
            Ok(format!("{bundle_id}，base64={submitted_base64}"))
        } else {
            tracing::debug!("RPC 交易已签名：{}", transaction_signature);
            let submit_started = Instant::now();
            let signature =
                transaction::send_transaction(&self.config.rpc_url, &transaction_base64).await?;
            let submit_elapsed_ms = submit_started.elapsed().as_millis();
            tracing::debug!("交易已提交：{}", signature);
            tracing::info!(
                "发送耗时明细：方式=RPC send，上下文={}，blockhash={}毫秒，构建签名={}毫秒，提交={}毫秒，总计={}毫秒，交易={}",
                log_context,
                blockhash_elapsed_ms,
                build_elapsed_ms,
                submit_elapsed_ms,
                total_started.elapsed().as_millis(),
                transaction_signature
            );
            self.confirm_submitted_signature(signature, "RPC transaction")
                .await
        }
    }

    async fn confirm_submitted_signature(
        &self,
        signature: String,
        transport_label: &'static str,
    ) -> Result<String> {
        match transaction::wait_for_transaction_confirmation_quick(&self.config.rpc_url, &signature)
            .await
        {
            Ok(confirmed) => {
                tracing::debug!(
                    "交易已确认：签名={}，slot={:?}，日志数={}",
                    confirmed.signature,
                    confirmed.slot,
                    confirmed.logs.len()
                );
                Ok(signature)
            }
            Err(error)
                if error
                    .to_string()
                    .contains("transaction confirmation timed out") =>
            {
                tracing::warn!("交易已提交，短时间内还没确认：签名={}", signature);
                self.spawn_async_confirmation(signature.clone(), transport_label);
                Ok(signature)
            }
            Err(error) => anyhow::bail!(error),
        }
    }

    fn spawn_async_confirmation(&self, signature: String, transport_label: &'static str) {
        let rpc_url = self.config.rpc_url.clone();
        tokio::spawn(async move {
            match transaction::wait_for_transaction_confirmation_by_signature(&rpc_url, &signature)
                .await
            {
                Ok(confirmed) => {
                    tracing::debug!(
                        "交易异步确认：方式={}，签名={}，slot={:?}，日志数={}",
                        transport_label,
                        confirmed.signature,
                        confirmed.slot,
                        confirmed.logs.len()
                    );
                }
                Err(error) if is_confirmation_timeout_without_status(&error) => {
                    tracing::warn!(
                        "交易未上链或本地RPC查不到确认：方式={}，签名={}，原因={}",
                        transport_label,
                        signature,
                        error
                    );
                }
                Err(error)
                    if error
                        .to_string()
                        .contains("transaction confirmation timed out") =>
                {
                    tracing::warn!(
                        "交易确认超时：方式={}，签名={}，原因={}",
                        transport_label,
                        signature,
                        error
                    );
                }
                Err(error) => {
                    let message = format!(
                        "交易链上执行失败：方式={}，签名={}，原因={}",
                        transport_label, signature, error
                    );
                    if !log_verbose_enabled() {
                        println!("{message}");
                    }
                    tracing::warn!("{message}");
                }
            }
        });
    }
}

fn is_confirmation_timeout_without_status(error: &anyhow::Error) -> bool {
    let error = error.to_string();
    error.contains("transaction confirmation timed out") && error.contains("last_status=None")
}

async fn wait_for_lookup_table_warmup(rpc_url: &str, extension_slot: u64) -> Result<()> {
    const LOOKUP_TABLE_WARMUP_ATTEMPTS: usize = 20;
    const LOOKUP_TABLE_WARMUP_POLL_MS: u64 = 200;

    for _ in 0..LOOKUP_TABLE_WARMUP_ATTEMPTS {
        let current_slot = rpc::get_slot(rpc_url).await?;
        if current_slot > extension_slot {
            return Ok(());
        }
        sleep(Duration::from_millis(LOOKUP_TABLE_WARMUP_POLL_MS)).await;
    }

    anyhow::bail!(
        "地址表预热超时：扩展slot={}，重试前仍不可用",
        extension_slot
    );
}
