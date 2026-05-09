use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

fn main() {
    let client = RpcClient::new("https://api.mainnet-beta.solana.com");
    let pool = Pubkey::from_str("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2").unwrap();

    println!("Fetching Raydium SOL/USDC pool data...\n");

    match client.get_account(&pool) {
        Ok(account) => {
            let data = &account.data;
            println!("Pool data length: {}\n", data.len());

            // Expected values
            let sol_mint = "So11111111111111111111111111111111111111112";
            let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

            println!("Expected mints:");
            println!("  SOL:  {}", sol_mint);
            println!("  USDC: {}\n", usdc_mint);

            // Search for SOL mint in the data
            let sol_bytes = bs58::decode(sol_mint).into_vec().unwrap();
            let usdc_bytes = bs58::decode(usdc_mint).into_vec().unwrap();

            println!("Searching for mint locations in pool data...\n");

            for i in 0..data.len().saturating_sub(32) {
                if &data[i..i + 32] == sol_bytes.as_slice() {
                    println!("Found SOL mint at offset: {}", i);
                }
                if &data[i..i + 32] == usdc_bytes.as_slice() {
                    println!("Found USDC mint at offset: {}", i);
                }
            }

            println!("\nTrying different offset combinations:\n");

            let offsets = vec![
                (400, 432, "Current parser (src/parser/raydium.rs)"),
                (73, 105, "Alternative parser (src/model/raydium.rs)"),
                (464, 496, "Alternative 1"),
                (368, 400, "Alternative 2"),
            ];

            for (offset1, offset2, label) in offsets {
                if data.len() >= offset2 + 32 {
                    let mint1 = bs58::encode(&data[offset1..offset1 + 32]).into_string();
                    let mint2 = bs58::encode(&data[offset2..offset2 + 32]).into_string();
                    println!("{} (offsets {}, {}):", label, offset1, offset2);
                    println!("  Mint 1: {}", mint1);
                    println!("  Mint 2: {}", mint2);

                    if mint1 == sol_mint || mint1 == usdc_mint {
                        println!("  ✓ Mint 1 matches!");
                    }
                    if mint2 == sol_mint || mint2 == usdc_mint {
                        println!("  ✓ Mint 2 matches!");
                    }
                    println!();
                }
            }
        }
        Err(e) => eprintln!("Error fetching account: {}", e),
    }
}
