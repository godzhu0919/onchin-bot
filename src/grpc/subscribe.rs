use crate::config::Config;
use crate::grpc::client;
use anyhow::Result;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use yellowstone_grpc_proto::prelude::*;

const OVERFLOW_LOG_INTERVAL: Duration = Duration::from_secs(2);
const OVERFLOW_FLUSH_BATCH_SIZE: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Pump,
    PumpSwapPool,
    PumpSwapVault,
    Raydium,
    RaydiumVault,
    RaydiumClmmTickArray,
    Meteora,
    MeteoraVault,
    MeteoraBinArray,
    Whirlpool,
    WhirlpoolTickArray,
}

#[derive(Debug)]
pub struct AccountUpdate {
    pub pubkey: String,
    pub kind: AccountKind,
    pub slot: u64,
    pub data: Vec<u8>,
    pub is_startup: bool,
}

#[derive(Debug, Clone)]
struct SubscriptionTarget {
    pubkey: String,
    kind: AccountKind,
}

#[derive(Debug, Default)]
struct StreamStats {
    received: u64,
    sent: u64,
    dropped: u64,
    buffered: u64,
    reconnects: u64,
}

pub fn subscribe_accounts(
    config: Config,
    pump_accounts: Vec<String>,
    pumpswap_pools: Vec<String>,
    pumpswap_vaults: Vec<String>,
    raydium_vaults: Vec<String>,
    raydium_clmm_tick_arrays: Vec<String>,
    meteora_vaults: Vec<String>,
    meteora_bin_arrays: Vec<String>,
    meteora_accounts: Vec<String>,
    raydium_accounts: Vec<String>,
    whirlpool_tick_arrays: Vec<String>,
    whirlpool_accounts: Vec<String>,
) -> mpsc::Receiver<AccountUpdate> {
    let requested_capacity = config.subscription.channel_capacity.max(1);
    let capacity = config.subscription.effective_channel_capacity();
    if capacity != requested_capacity {
        tracing::warn!(
            "监听队列容量已限制：配置={}，实际={}，用于控制内存",
            requested_capacity,
            capacity
        );
    }
    let (tx, rx) = mpsc::channel(capacity);
    let targets = build_targets(
        pump_accounts,
        pumpswap_pools,
        pumpswap_vaults,
        raydium_vaults,
        raydium_clmm_tick_arrays,
        meteora_vaults,
        meteora_bin_arrays,
        meteora_accounts,
        raydium_accounts,
        whirlpool_tick_arrays,
        whirlpool_accounts,
    );

    tokio::spawn(async move {
        run_subscription_loop(config, targets, tx).await;
    });

    rx
}

async fn run_subscription_loop(
    config: Config,
    targets: Vec<SubscriptionTarget>,
    tx: mpsc::Sender<AccountUpdate>,
) {
    let account_kind_by_pubkey: HashMap<String, AccountKind> = targets
        .iter()
        .map(|target| (target.pubkey.clone(), target.kind))
        .collect();
    let request = build_subscribe_request(&targets);
    let mut backoff = Duration::from_millis(config.subscription.reconnect_initial_ms.max(1));
    let max_backoff = Duration::from_millis(config.subscription.reconnect_max_ms.max(1));
    let idle_timeout = Duration::from_secs(config.subscription.stream_idle_timeout_secs.max(1));
    let mut stats = StreamStats::default();

    tracing::info!(
        "启动账户监听：账户数={}，队列容量={}，{}秒无更新会重连",
        targets.len(),
        config.subscription.effective_channel_capacity(),
        config.subscription.stream_idle_timeout_secs
    );

    loop {
        if tx.is_closed() {
            tracing::warn!("账户更新处理通道已关闭，停止 Yellowstone 订阅任务");
            break;
        }

        let received_before_attempt = stats.received;
        match subscribe_once(
            &config,
            request.clone(),
            &account_kind_by_pubkey,
            &tx,
            idle_timeout,
            &mut stats,
        )
        .await
        {
            Ok(()) => {
                tracing::warn!("Yellowstone 账户流已结束，准备重连");
            }
            Err(e) => {
                tracing::error!("Yellowstone 订阅失败：{}", e);
            }
        }

        stats.reconnects += 1;
        if stats.received > received_before_attempt {
            backoff = Duration::from_millis(config.subscription.reconnect_initial_ms.max(1));
        }
        tracing::warn!(
            "账户监听准备重连：等待={:?}，次数={}，收到={}，入队={}，合并={}",
            backoff,
            stats.reconnects,
            stats.received,
            stats.sent,
            stats.dropped
        );
        sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

async fn subscribe_once(
    config: &Config,
    request: SubscribeRequest,
    account_kind_by_pubkey: &HashMap<String, AccountKind>,
    tx: &mpsc::Sender<AccountUpdate>,
    idle_timeout: Duration,
    stats: &mut StreamStats,
) -> Result<()> {
    let mut grpc_client = client::create_client(config).await?;
    let mut overflow_updates: HashMap<String, AccountUpdate> = HashMap::new();
    let mut overflow_started_at: Option<Instant> = None;
    let mut overflow_dropped_at_start: Option<u64> = None;
    let mut last_overflow_log_at: Option<Instant> = None;
    let mut max_buffered_accounts = 0usize;
    tracing::info!("gRPC 已连接");

    let (_, mut stream) = grpc_client.subscribe_with_request(Some(request)).await?;
    tracing::info!("账户流已建立");

    loop {
        let had_overflow = !overflow_updates.is_empty();
        flush_overflow_updates(tx, &mut overflow_updates, stats)?;
        if had_overflow && overflow_updates.is_empty() {
            if let Some(started_at) = overflow_started_at.take() {
                let dropped_before_start =
                    overflow_dropped_at_start.take().unwrap_or(stats.dropped);
                tracing::warn!(
                    "监听队列已恢复：持续={:?}，合并更新={}，峰值账户={}",
                    started_at.elapsed(),
                    stats.dropped.saturating_sub(dropped_before_start),
                    max_buffered_accounts
                );
            }
            last_overflow_log_at = None;
            max_buffered_accounts = 0;
        }
        let message = match timeout(idle_timeout, stream.next()).await {
            Ok(Some(message)) => message?,
            Ok(None) => return Ok(()),
            Err(_) => {
                anyhow::bail!("{} 秒内没有收到 Yellowstone 消息", idle_timeout.as_secs());
            }
        };

        let Some(update) = message.update_oneof else {
            continue;
        };

        let subscribe_update::UpdateOneof::Account(account_update) = update else {
            continue;
        };

        let Some(account) = account_update.account else {
            continue;
        };

        stats.received += 1;
        let pubkey = bs58::encode(&account.pubkey).into_string();
        let Some(kind) = account_kind_by_pubkey.get(&pubkey).copied() else {
            tracing::debug!("收到未订阅账户更新：{}", pubkey);
            continue;
        };

        let update = AccountUpdate {
            pubkey,
            kind,
            slot: account_update.slot,
            data: account.data,
            is_startup: account_update.is_startup,
        };

        if overflow_updates.is_empty() {
            match tx.try_send(update) {
                Ok(()) => {
                    stats.sent += 1;
                    continue;
                }
                Err(mpsc::error::TrySendError::Full(update)) => {
                    stats.dropped += 1;
                    overflow_updates.insert(update.pubkey.clone(), update);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::warn!("账户更新接收端已关闭");
                    return Ok(());
                }
            }
        } else {
            stats.dropped += 1;
            overflow_updates.insert(update.pubkey.clone(), update);
        }

        stats.buffered = overflow_updates.len() as u64;
        max_buffered_accounts = max_buffered_accounts.max(overflow_updates.len());
        let now = Instant::now();
        overflow_started_at.get_or_insert(now);
        overflow_dropped_at_start.get_or_insert(stats.dropped.saturating_sub(1));
        let should_log = last_overflow_log_at
            .map(|last_logged| now.duration_since(last_logged) >= OVERFLOW_LOG_INTERVAL)
            .unwrap_or(true);
        if should_log {
            let dropped_before_start = overflow_dropped_at_start.unwrap_or(stats.dropped);
            tracing::warn!(
                "监听队列已满：合并更新={}，缓存账户={}，订阅账户={}，持续={:?}",
                stats.dropped.saturating_sub(dropped_before_start),
                stats.buffered,
                account_kind_by_pubkey.len(),
                overflow_started_at
                    .map(|started_at| now.duration_since(started_at))
                    .unwrap_or_default()
            );
            last_overflow_log_at = Some(now);
        }
    }
}

fn flush_overflow_updates(
    tx: &mpsc::Sender<AccountUpdate>,
    overflow_updates: &mut HashMap<String, AccountUpdate>,
    stats: &mut StreamStats,
) -> Result<()> {
    if overflow_updates.is_empty() {
        return Ok(());
    }

    let keys: Vec<String> = overflow_updates
        .keys()
        .take(OVERFLOW_FLUSH_BATCH_SIZE)
        .cloned()
        .collect();
    for key in keys {
        let Some(update) = overflow_updates.remove(&key) else {
            continue;
        };

        match tx.try_send(update) {
            Ok(()) => {
                stats.sent += 1;
            }
            Err(mpsc::error::TrySendError::Full(update)) => {
                overflow_updates.insert(key, update);
                stats.buffered = overflow_updates.len() as u64;
                return Ok(());
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                anyhow::bail!("account update receiver closed while flushing buffered updates");
            }
        }
    }

    stats.buffered = overflow_updates.len() as u64;
    Ok(())
}

fn build_targets(
    pump_accounts: Vec<String>,
    pumpswap_pools: Vec<String>,
    pumpswap_vaults: Vec<String>,
    raydium_vaults: Vec<String>,
    raydium_clmm_tick_arrays: Vec<String>,
    meteora_vaults: Vec<String>,
    meteora_bin_arrays: Vec<String>,
    meteora_accounts: Vec<String>,
    raydium_accounts: Vec<String>,
    whirlpool_tick_arrays: Vec<String>,
    whirlpool_accounts: Vec<String>,
) -> Vec<SubscriptionTarget> {
    let mut targets = Vec::new();

    for pubkey in pump_accounts {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::Pump,
        });
    }

    for pubkey in pumpswap_pools {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::PumpSwapPool,
        });
    }

    for pubkey in pumpswap_vaults {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::PumpSwapVault,
        });
    }

    for pubkey in raydium_accounts {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::Raydium,
        });
    }

    for pubkey in raydium_vaults {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::RaydiumVault,
        });
    }

    for pubkey in raydium_clmm_tick_arrays {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::RaydiumClmmTickArray,
        });
    }

    for pubkey in meteora_accounts {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::Meteora,
        });
    }

    for pubkey in meteora_vaults {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::MeteoraVault,
        });
    }

    for pubkey in meteora_bin_arrays {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::MeteoraBinArray,
        });
    }

    for pubkey in whirlpool_accounts {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::Whirlpool,
        });
    }

    for pubkey in whirlpool_tick_arrays {
        targets.push(SubscriptionTarget {
            pubkey,
            kind: AccountKind::WhirlpoolTickArray,
        });
    }

    targets.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
    targets.dedup_by(|a, b| a.pubkey == b.pubkey);
    targets
}

fn build_subscribe_request(targets: &[SubscriptionTarget]) -> SubscribeRequest {
    let mut accounts = HashMap::new();

    for target in targets {
        accounts.insert(
            format!(
                "{}_{}",
                kind_label(target.kind),
                short_pubkey(&target.pubkey)
            ),
            SubscribeRequestFilterAccounts {
                account: vec![target.pubkey.clone()],
                owner: vec![],
                filters: vec![],
                nonempty_txn_signature: None,
            },
        );
    }

    SubscribeRequest {
        accounts,
        slots: HashMap::new(),
        transactions: HashMap::new(),
        transactions_status: HashMap::new(),
        blocks: HashMap::new(),
        blocks_meta: HashMap::new(),
        entry: HashMap::new(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: vec![],
        ping: None,
        from_slot: None,
    }
}

fn kind_label(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Pump => "pump",
        AccountKind::PumpSwapPool => "pumpswap_pool",
        AccountKind::PumpSwapVault => "pumpswap_vault",
        AccountKind::Raydium => "raydium",
        AccountKind::RaydiumVault => "raydium_vault",
        AccountKind::RaydiumClmmTickArray => "raydium_clmm_tick_array",
        AccountKind::Meteora => "meteora",
        AccountKind::MeteoraVault => "meteora_vault",
        AccountKind::MeteoraBinArray => "meteora_bin_array",
        AccountKind::Whirlpool => "whirlpool",
        AccountKind::WhirlpoolTickArray => "whirlpool_tick_array",
    }
}

fn short_pubkey(pubkey: &str) -> &str {
    let end = pubkey.len().min(8);
    &pubkey[..end]
}
