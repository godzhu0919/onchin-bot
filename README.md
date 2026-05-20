# Solana 套利机器人

这是一个索引快照驱动的套利机器人。索引器负责发现并链上校验池子，交易端只读取已验证快照，不再把静态 txt 当成交易真源。

## 配置

所有非敏感配置统一在 `config.toml`。

- `[rpc]`：RPC 地址。
- `[grpc]`：Yellowstone gRPC 地址。
- `[subscription]`：监听队列和重连参数。
- `[strategy]`：扫描金额、利润阈值、滑点和报价参数。
- `[execution]`：发送开关、Jito、模拟和交易构建参数。
- `[discovery].validated_pools_snapshot_path` / `validated_pools_snapshot_url`：索引器输出的已验证池子快照。
- `[tokens]`：USDC 和 WSOL mint。

`.env` 只放钱包私钥、Jito UUID 这类敏感信息，不放市场配置。

## 市场和代币

交易端不再自己找池子，也不再读取 `dynamic_market_addresses.txt`。

索引器默认使用链上 program account 扫描 PumpSwap + Meteora DLMM，输出 `validated_pools.snapshot` 或 `validated_pools.jsonl`。交易端启动时读取一次，之后热加载快照变化。只有快照里的已验证池子会被 hydrate、订阅并进入路径图。

选币优先级：SOL 报价、两个以上 DEX、有最近 5 到 15 分钟真实交易、流动性足够，且 PumpSwap + Raydium CLMM 优先于 PumpSwap + Meteora DLMM。

## 运行

```bash
cargo run --release
```

日志默认使用中文，重点只看启动、市场加载、监听状态、报价结果和发送失败原因。

## 常用调整

- 改池子：让索引器更新 `validated_pools.snapshot`。
- 改扫描金额：编辑 `[strategy].trade_sizes` 和 `[strategy].program_pair_trade_sizes_sol`。
- 改发送门槛：编辑 `[execution].live_send_min_profit_pct`。
- 暂停实盘：设置 `[execution].enabled = false` 或 `[execution].dry_run_only = true`。


  POOL_SCAN_SOURCE=onchain cargo run --release --bin pool_scanner

 cargo run --release --bin solana_arb_bot

   cargo build --release --bin solana_arb_bot
  ./target/release/solana_arb_bot

    POOL_SCAN_MIN_EDGE_PCT=1 POOL_SCAN_SOURCE=grpc cargo run --release --bin pool_scanner
