# Solana 套利机器人

这是一个静态市场套利机器人。现在不做新池发现，也不再从多个文件读取市场列表。

## 配置

所有非敏感配置统一在 `config.toml`。

- `[rpc]`：RPC 地址。
- `[grpc]`：Yellowstone gRPC 地址。
- `[subscription]`：监听队列和重连参数。
- `[strategy]`：扫描金额、利润阈值、滑点和报价参数。
- `[execution]`：发送开关、Jito、模拟和交易构建参数。
- `[discovery].manual_market_addresses`：手工维护的池子地址。
- `[tokens]`：USDC 和 WSOL mint。

`.env` 只放钱包私钥、Jito UUID 这类敏感信息，不放市场配置。

## 市场和代币

要加一个代币，不是配置 token mint，而是在 `config.toml` 的 `[discovery].manual_market_addresses` 里加入这个代币在不同 DEX 上的池子地址。

程序启动时会用 RPC 读取这些池子账户，自动识别 PumpSwap、Raydium、Meteora 或 Whirlpool，并从池子里反推出 token mint。只有同一个 token 至少有两个可路由市场时，才会进入套利扫描。

旧的外部市场文件已经移除，避免配置分散。

## 运行

```bash
cargo run --release
```

日志默认使用中文，重点只看启动、市场加载、监听状态、报价结果和发送失败原因。

## 常用调整

- 改池子：编辑 `[discovery].manual_market_addresses`。
- 改扫描金额：编辑 `[strategy].trade_sizes` 和 `[strategy].program_pair_trade_sizes_sol`。
- 改发送门槛：编辑 `[execution].live_send_min_profit_pct`。
- 暂停实盘：设置 `[execution].enabled = false` 或 `[execution].dry_run_only = true`。


  cargo run --release --bin pool_scanner

 cargo run --release --bin solana_arb_bot

   cargo build --release --bin solana_arb_bot
  ./target/release/solana_arb_bot