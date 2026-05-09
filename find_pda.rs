use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::str::FromStr;

fn main() {
    // Pump.fun 程序 ID
    let pump_program = Pubkey::from_str("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P").unwrap();

    // 代币 mint 地址
    let mint = Pubkey::from_str("FybT3QBqy5GdWPM3X3UoxggQQgMMFzcduk74YuVQpump").unwrap();

    // 计算 bonding curve PDA
    let (bonding_curve, bump) = Pubkey::find_program_address(
        &[b"bonding-curve", mint.as_ref()],
        &pump_program,
    );

    println!("Bonding Curve Address: {}", bonding_curve);
    println!("Bump: {}", bump);
}
