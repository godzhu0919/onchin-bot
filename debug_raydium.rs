use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

fn main() {
    let client = RpcClient::new("https://api.mainnet-beta.solana.com");
    let pool = Pubkey::from_str("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2").unwrap();
    
    match client.get_account(&pool) {
        Ok(account) => {
            let data = &account.data;
            println!("Pool data length: {}", data.len());
            println!("\nFirst 8 bytes (discriminator): {:?}", &data[..8]);
            
            // Try different offsets for mints
            let offsets = vec![
                (400, 432, "Current parser offsets"),
                (72, 104, "CPMM offsets"),
                (464, 496, "Alternative 1"),
                (368, 400, "Alternative 2"),
            ];
            
            for (quote_offset, base_offset, label) in offsets {
                if data.len() >= base_offset + 32 {
                    let quote_mint = bs58::encode(&data[quote_offset..quote_offset+32]).into_string();
                    let base_mint = bs58::encode(&data[base_offset..base_offset+32]).into_string();
                    println!("\n{} (quote@{}, base@{}):", label, quote_offset, base_offset);
                    println!("  Quote mint: {}", quote_mint);
                    println!("  Base mint:  {}", base_mint);
                }
            }
            
            // Expected values
            println!("\n\nExpected:");
            println!("  SOL:  So11111111111111111111111111111111111111112");
            println!("  USDC: EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
