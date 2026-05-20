use crate::executor::{config::ExecutorConfig, executor::ArbitrageExecutor, transaction};
use crate::model::state::{PumpState, RaydiumState};
use crate::strategy::execution::ValidatedArbitrage;
use anyhow::Result;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio::time::sleep;

pub struct ExecutionManager {
    executor: Option<Arc<ArbitrageExecutor>>,
    last_execution: Arc<Mutex<std::time::Instant>>,
    min_execution_interval: std::time::Duration,
}

impl ExecutionManager {
    pub fn new(default_rpc_url: &str) -> Result<Self> {
        let configured_min_execution_interval_ms = std::env::var("EXECUTION_MIN_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(250);
        let executor = match ExecutorConfig::from_env_with_rpc_default(default_rpc_url) {
            Ok(config) => {
                tracing::info!(
                    "执行配置：发送={}，只演练={}，最小间隔={}毫秒，发送前模拟={}",
                    if config.enabled { "开" } else { "关" },
                    if config.dry_run_only { "是" } else { "否" },
                    configured_min_execution_interval_ms,
                    if config.require_pre_send_simulation {
                        "开"
                    } else {
                        "关"
                    }
                );

                if !config.enabled || config.dry_run_only {
                    tracing::warn!("实盘发送未开启，只监控不广播交易");
                } else {
                    if !config.require_pre_send_simulation {
                        tracing::warn!("发送前模拟已关闭，实盘风险较高");
                    }
                }

                Some((
                    Arc::new(ArbitrageExecutor::new(config)?),
                    configured_min_execution_interval_ms,
                ))
            }
            Err(e) => {
                tracing::warn!("执行器未配置，只运行监控模式：{}", e);
                None
            }
        };

        let (executor, min_execution_interval_ms) = executor
            .map(|(executor, min_interval)| (Some(executor), min_interval))
            .unwrap_or((None, configured_min_execution_interval_ms));

        let min_execution_interval = Duration::from_millis(min_execution_interval_ms);
        let now = std::time::Instant::now();
        let last_execution = now.checked_sub(min_execution_interval).unwrap_or(now);

        Ok(Self {
            executor,
            last_execution: Arc::new(Mutex::new(last_execution)),
            min_execution_interval,
        })
    }

    async fn reserve_send_slot(&self) {
        if self.min_execution_interval.is_zero() {
            return;
        }
        loop {
            let wait_for = {
                let mut last_exec = self.last_execution.lock().await;
                let elapsed = last_exec.elapsed();
                if elapsed >= self.min_execution_interval {
                    *last_exec = std::time::Instant::now();
                    return;
                }
                self.min_execution_interval - elapsed
            };
            tracing::debug!(
                "Live send throttle: waiting {}ms before next submission",
                wait_for.as_millis()
            );
            sleep(wait_for).await;
        }
    }

    pub async fn execute_arbitrage(
        &self,
        opportunity: &ValidatedArbitrage,
        pump_state: &PumpState,
        raydium_state: &RaydiumState,
        sol_price: f64,
    ) -> Result<String> {
        let executor = match &self.executor {
            Some(e) => e,
            None => {
                return Ok("MONITORING_MODE".to_string());
            }
        };

        self.reserve_send_slot().await;

        let result = executor
            .execute_arbitrage(opportunity, pump_state, raydium_state, sol_price)
            .await?;
        Ok(result)
    }

    pub fn wallet_pubkey(&self) -> Option<Pubkey> {
        self.executor
            .as_ref()
            .map(|executor| executor.config().get_pubkey())
    }

    pub fn live_send_enabled(&self) -> bool {
        self.executor
            .as_ref()
            .map(|executor| executor.live_send_enabled())
            .unwrap_or(false)
    }

    pub fn has_jito_bundle_transport(&self) -> bool {
        self.executor
            .as_ref()
            .map(|executor| executor.has_jito_bundle_transport())
            .unwrap_or(false)
    }

    pub fn live_send_min_profit_pct(&self) -> f64 {
        self.executor
            .as_ref()
            .map(|executor| executor.live_send_min_profit_pct())
            .unwrap_or(0.0)
    }

    pub fn require_pre_send_simulation(&self) -> bool {
        self.executor
            .as_ref()
            .map(|executor| executor.require_pre_send_simulation())
            .unwrap_or(true)
    }

    pub fn estimated_send_gas_cost_sol(&self) -> Option<f64> {
        self.executor
            .as_ref()
            .map(|executor| executor.config().estimated_send_gas_cost_sol())
    }

    pub fn jito_tip_lamports(&self) -> u64 {
        self.executor
            .as_ref()
            .map(|executor| executor.jito_tip_lamports())
            .unwrap_or(0)
    }

    pub fn slippage_floor(&self, amount: u64) -> u64 {
        self.executor
            .as_ref()
            .map(|executor| executor.slippage_floor(amount))
            .unwrap_or(amount)
    }

    pub fn two_hop_executor_program_id(&self) -> Option<Pubkey> {
        self.executor
            .as_ref()
            .and_then(|executor| executor.two_hop_executor_program_id())
    }

    pub async fn simulate_instructions(
        &self,
        instructions: Vec<Instruction>,
    ) -> Result<transaction::SimulationReport> {
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;
        executor.simulate_instructions(instructions).await
    }

    pub async fn simulate_instructions_with_accounts(
        &self,
        instructions: Vec<Instruction>,
        account_addresses: &[String],
    ) -> Result<(
        transaction::SimulationReport,
        std::collections::HashMap<String, Vec<u8>>,
    )> {
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;
        executor
            .simulate_instructions_with_accounts(instructions, account_addresses)
            .await
    }

    pub async fn send_instructions(&self, instructions: Vec<Instruction>) -> Result<String> {
        self.send_instructions_with_simulated_units(instructions, None)
            .await
    }

    pub async fn extend_lookup_tables(&self, candidate_addresses: &[Pubkey]) -> Result<usize> {
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;
        executor.extend_lookup_tables(candidate_addresses).await
    }

    pub async fn send_instructions_unmetered(
        &self,
        instructions: Vec<Instruction>,
    ) -> Result<String> {
        self.send_instructions_with_simulated_units_and_tip_context_unmetered(
            instructions,
            None,
            self.jito_tip_lamports(),
            None,
        )
        .await
    }

    pub async fn send_instructions_with_simulated_units(
        &self,
        instructions: Vec<Instruction>,
        simulated_units_consumed: Option<u64>,
    ) -> Result<String> {
        self.send_instructions_with_simulated_units_and_tip(
            instructions,
            simulated_units_consumed,
            self.jito_tip_lamports(),
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
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;

        self.reserve_send_slot().await;

        let bundle_id = executor
            .send_instructions_with_simulated_units_and_tip_context(
                instructions,
                simulated_units_consumed,
                jito_tip_lamports,
                log_context,
            )
            .await?;
        Ok(bundle_id)
    }

    pub async fn send_instructions_with_simulated_units_and_tip_context_unmetered(
        &self,
        instructions: Vec<Instruction>,
        simulated_units_consumed: Option<u64>,
        jito_tip_lamports: u64,
        log_context: Option<&str>,
    ) -> Result<String> {
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;
        self.reserve_send_slot().await;
        executor
            .send_instructions_with_simulated_units_and_tip_context(
                instructions,
                simulated_units_consumed,
                jito_tip_lamports,
                log_context,
            )
            .await
    }

    pub async fn send_instruction_bundle_with_simulated_units_and_tip_context_unmetered(
        &self,
        instruction_groups: Vec<Vec<Instruction>>,
        simulated_units_consumed: Vec<Option<u64>>,
        jito_tip_lamports: u64,
        log_context: Option<&str>,
    ) -> Result<String> {
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;
        self.reserve_send_slot().await;
        executor
            .send_instruction_bundle_with_simulated_units_and_tip_context(
                instruction_groups,
                simulated_units_consumed,
                jito_tip_lamports,
                log_context,
            )
            .await
    }

    pub async fn send_instruction_bundle_with_simulated_units(
        &self,
        instruction_groups: Vec<Vec<Instruction>>,
        simulated_units_consumed: Vec<Option<u64>>,
    ) -> Result<String> {
        self.send_instruction_bundle_with_simulated_units_and_tip(
            instruction_groups,
            simulated_units_consumed,
            self.jito_tip_lamports(),
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
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("executor not configured"))?;

        self.reserve_send_slot().await;

        let signature = executor
            .send_instruction_bundle_with_simulated_units_and_tip_context(
                instruction_groups,
                simulated_units_consumed,
                jito_tip_lamports,
                log_context,
            )
            .await?;
        Ok(signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::config::SendTransport;
    use solana_sdk::signature::Keypair;

    fn manager(require_pre_send_simulation: bool) -> ExecutionManager {
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
            require_pre_send_simulation,
            send_transport: SendTransport::Rpc,
            jito_uuid: None,
            jito_tip_lamports: 0,
            two_hop_executor_program_id: None,
        })
        .unwrap();

        ExecutionManager {
            executor: Some(Arc::new(executor)),
            last_execution: Arc::new(Mutex::new(std::time::Instant::now())),
            min_execution_interval: std::time::Duration::from_secs(10),
        }
    }

    #[test]
    fn require_pre_send_simulation_defaults_to_true() {
        let manager = manager(true);

        assert!(manager.require_pre_send_simulation());
    }

    #[test]
    fn require_pre_send_simulation_can_be_disabled() {
        let manager = manager(false);

        assert!(!manager.require_pre_send_simulation());
    }
}
