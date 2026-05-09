#[derive(Debug, Clone)]
pub struct PumpState {
    pub sol_reserve: u64,
    pub token_reserve: u64,
    pub token_mint: String,
    pub price_history: Vec<PriceSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PriceSnapshot {
    pub price: f64,
    pub timestamp: std::time::Instant,
}

impl PumpState {
    /// Calculate current price in SOL per token
    pub fn calculate_price(&self) -> f64 {
        if self.token_reserve == 0 {
            return 0.0;
        }
        (self.sol_reserve as f64 / 1e9) / (self.token_reserve as f64 / 1e6)
    }

    /// Add a new price snapshot to history (keep last 100)
    pub fn update_price_history(&mut self) {
        let price = self.calculate_price();
        let snapshot = PriceSnapshot {
            price,
            timestamp: std::time::Instant::now(),
        };

        self.price_history.push(snapshot);

        // Keep only last 100 snapshots
        if self.price_history.len() > 100 {
            self.price_history.remove(0);
        }
    }

    /// Get price change percentage over a time window (in seconds)
    pub fn get_price_change(&self, window_secs: u64) -> Option<f64> {
        if self.price_history.len() < 2 {
            return None;
        }

        let current = self.price_history.last()?;
        let cutoff = current.timestamp - std::time::Duration::from_secs(window_secs);

        // Find the oldest price within the window
        // We want the last price that is still >= cutoff (closest to cutoff time)
        let old_price = self
            .price_history
            .iter()
            .rev()
            .skip(1) // Skip current price
            .find(|s| s.timestamp >= cutoff)?;

        if old_price.price == 0.0 {
            return None;
        }

        let change_pct = ((current.price - old_price.price) / old_price.price) * 100.0;
        Some(change_pct)
    }
}

#[derive(Debug, Clone)]
pub struct MeteoraState {
    pub pool_address: String,
    pub active_id: i32,
    pub bin_step: u16,
    pub base_factor: u16,
    pub variable_fee_control: u32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    pub volatility_accumulator: u32,
    pub token_x_mint: String,
    pub token_y_mint: String,
    pub reserve_x: String,
    pub reserve_y: String,
    pub bin_array_bitmap: [u64; 16],
    pub token_x_amount: u64,
    pub token_y_amount: u64,
    pub fee_bps: f64,
    pub damm_v2_pool_data: Option<Vec<u8>>,
    pub price_history: Vec<PriceSnapshot>,
}

impl MeteoraState {
    pub fn is_damm_v2(&self) -> bool {
        self.bin_step == 0
    }

    pub fn current_total_fee_rate(&self) -> u64 {
        const FEE_PRECISION: u128 = 1_000_000_000;
        const MAX_FEE_RATE: u128 = 100_000_000;
        let base_fee = (self.base_factor as u128)
            .saturating_mul(self.bin_step as u128)
            .saturating_mul(10)
            .saturating_mul(10u128.saturating_pow(self.base_fee_power_factor as u32));
        let variable_fee = if self.variable_fee_control > 0 {
            let vfa_bin =
                (self.volatility_accumulator as u128).saturating_mul(self.bin_step as u128);
            (self.variable_fee_control as u128)
                .saturating_mul(vfa_bin.saturating_mul(vfa_bin))
                .saturating_add(99_999_999_999)
                / 100_000_000_000
        } else {
            0
        };
        base_fee
            .saturating_add(variable_fee)
            .min(MAX_FEE_RATE)
            .min(FEE_PRECISION) as u64
    }

    /// Calculate price from DLMM active_id and bin_step
    /// Price = (1 + bin_step/10000)^active_id
    pub fn calculate_price(&self) -> f64 {
        if self.bin_step == 0 {
            return 1.0;
        }

        // Prevent overflow for unreasonable values
        if self.bin_step > 1000 || self.active_id.abs() > 100000 {
            tracing::warn!(
                "Suspicious DLMM parameters: bin_step={}, active_id={}",
                self.bin_step,
                self.active_id
            );
            return f64::NAN;
        }

        let base = 1.0 + (self.bin_step as f64 / 10000.0);
        let price = base.powf(self.active_id as f64);

        if !price.is_finite() {
            tracing::warn!(
                "Price overflow: bin_step={}, active_id={}, price={}",
                self.bin_step,
                self.active_id,
                price
            );
            return f64::NAN;
        }

        price
    }

    pub fn traded_token(&self, usdc_mint: &str, sol_mint: &str) -> Option<String> {
        if self.token_x_mint == usdc_mint || self.token_x_mint == sol_mint {
            Some(self.token_y_mint.clone())
        } else if self.token_y_mint == usdc_mint || self.token_y_mint == sol_mint {
            Some(self.token_x_mint.clone())
        } else {
            None
        }
    }

    pub fn quote_mint(&self, usdc_mint: &str, sol_mint: &str) -> Option<String> {
        if self.token_x_mint == usdc_mint || self.token_x_mint == sol_mint {
            Some(self.token_x_mint.clone())
        } else if self.token_y_mint == usdc_mint || self.token_y_mint == sol_mint {
            Some(self.token_y_mint.clone())
        } else {
            None
        }
    }

    pub fn token_price_in_quote(&self, usdc_mint: &str, sol_mint: &str) -> Option<f64> {
        let price_y_per_x = self.calculate_price();
        if !price_y_per_x.is_finite() || price_y_per_x <= 0.0 {
            return None;
        }

        if self.token_x_mint == usdc_mint || self.token_x_mint == sol_mint {
            Some(1.0 / price_y_per_x)
        } else if self.token_y_mint == usdc_mint || self.token_y_mint == sol_mint {
            Some(price_y_per_x)
        } else {
            None
        }
    }

    pub fn update_price_history(&mut self) {
        let price = self.calculate_price();
        let now = std::time::Instant::now();
        self.price_history.push(PriceSnapshot {
            price,
            timestamp: now,
        });
        let cutoff = now - std::time::Duration::from_secs(60);
        self.price_history.retain(|s| s.timestamp >= cutoff);
    }
}

#[derive(Debug, Clone)]
pub struct RaydiumState {
    pub pool_address: String,
    pub venue: RaydiumVenue,
    pub amm_config: Option<String>,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_vault: Option<String>,
    pub quote_vault: Option<String>,
    pub observation_key: Option<String>,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub sqrt_price_x64: Option<u128>,
    pub liquidity: u128,
    pub tick_current: Option<i32>,
    pub tick_spacing: Option<u16>,
    pub base_fee_owed: u64,
    pub quote_fee_owed: u64,
    pub fee_bps: f64,
    pub price_history: Vec<PriceSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaydiumVenue {
    AmmV4,
    Cpmm,
    Clmm,
}

#[derive(Debug, Clone)]
pub struct WhirlpoolState {
    pub pool_address: String,
    pub whirlpools_config: String,
    pub tick_spacing: u16,
    pub fee_rate: u16,
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub tick_current_index: i32,
    pub token_mint_a: String,
    pub token_vault_a: String,
    pub token_mint_b: String,
    pub token_vault_b: String,
    pub fee_bps: f64,
    pub price_history: Vec<PriceSnapshot>,
}

impl WhirlpoolState {
    pub fn calculate_price(&self, decimals_a: u8, decimals_b: u8) -> f64 {
        orca_whirlpools_core::sqrt_price_to_price(self.sqrt_price, decimals_a, decimals_b)
    }

    pub fn traded_token(&self, usdc_mint: &str, sol_mint: &str) -> Option<String> {
        if self.token_mint_a == usdc_mint || self.token_mint_a == sol_mint {
            Some(self.token_mint_b.clone())
        } else if self.token_mint_b == usdc_mint || self.token_mint_b == sol_mint {
            Some(self.token_mint_a.clone())
        } else {
            None
        }
    }

    pub fn quote_mint(&self, usdc_mint: &str, sol_mint: &str) -> Option<String> {
        if self.token_mint_a == usdc_mint || self.token_mint_a == sol_mint {
            Some(self.token_mint_a.clone())
        } else if self.token_mint_b == usdc_mint || self.token_mint_b == sol_mint {
            Some(self.token_mint_b.clone())
        } else {
            None
        }
    }

    pub fn update_price_history(&mut self, decimals_a: u8, decimals_b: u8) {
        let price = self.calculate_price(decimals_a, decimals_b);
        let now = std::time::Instant::now();
        self.price_history.push(PriceSnapshot {
            price,
            timestamp: now,
        });
        let cutoff = now - std::time::Duration::from_secs(60);
        self.price_history.retain(|s| s.timestamp >= cutoff);
    }
}

#[derive(Debug, Clone)]
pub struct WhirlpoolTickArrayState {
    pub whirlpool: String,
    pub start_tick_index: i32,
    pub initialized_tick_count: usize,
    pub tick_array: orca_whirlpools_core::TickArrayFacade,
}

impl RaydiumVenue {
    pub fn label(self) -> &'static str {
        match self {
            Self::AmmV4 => "Raydium AMM V4",
            Self::Cpmm => "Raydium CPMM",
            Self::Clmm => "Raydium CLMM",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PumpSwapState {
    pub pool_address: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_vault: String,
    pub quote_vault: String,
    pub coin_creator: Option<String>,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub price_history: Vec<PriceSnapshot>,
}

impl PumpSwapState {
    pub fn traded_token(&self, usdc_mint: &str, sol_mint: &str) -> Option<String> {
        if self.base_mint == usdc_mint || self.base_mint == sol_mint {
            Some(self.quote_mint.clone())
        } else if self.quote_mint == usdc_mint || self.quote_mint == sol_mint {
            Some(self.base_mint.clone())
        } else {
            None
        }
    }

    pub fn update_price_history(&mut self) {
        let price = if self.base_reserve == 0 {
            0.0
        } else {
            self.quote_reserve as f64 / self.base_reserve as f64
        };
        let now = std::time::Instant::now();
        self.price_history.push(PriceSnapshot {
            price,
            timestamp: now,
        });
        let cutoff = now - std::time::Duration::from_secs(60);
        self.price_history.retain(|s| s.timestamp >= cutoff);
    }
}

impl RaydiumState {
    pub fn effective_base_reserve(&self) -> u64 {
        self.base_reserve.saturating_sub(self.base_fee_owed)
    }

    pub fn effective_quote_reserve(&self) -> u64 {
        self.quote_reserve.saturating_sub(self.quote_fee_owed)
    }

    /// Calculate price with proper decimal handling
    pub fn calculate_price(&self) -> f64 {
        if let Some(sqrt_price_x64) = self.sqrt_price_x64 {
            let sqrt_price = sqrt_price_x64 as f64 / 2_f64.powi(64);
            return sqrt_price
                * sqrt_price
                * 10_f64.powi(self.base_decimals as i32 - self.quote_decimals as i32);
        }

        let base_reserve = self.effective_base_reserve();
        let quote_reserve = self.effective_quote_reserve();
        if base_reserve == 0 {
            return 0.0;
        }

        let sol_mint = "So11111111111111111111111111111111111111112";
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

        // Determine decimals based on token type
        let (base_decimals, quote_decimals) = if self.base_mint == sol_mint {
            (9, if self.quote_mint == usdc_mint { 6 } else { 6 })
        } else if self.quote_mint == sol_mint {
            (if self.base_mint == usdc_mint { 6 } else { 6 }, 9)
        } else if self.base_mint == usdc_mint {
            (6, 6)
        } else if self.quote_mint == usdc_mint {
            (6, 6)
        } else {
            (6, 6) // Default for unknown tokens
        };

        let base_adjusted = base_reserve as f64 / 10_f64.powi(base_decimals);
        let quote_adjusted = quote_reserve as f64 / 10_f64.powi(quote_decimals);

        if base_adjusted == 0.0 {
            return 0.0;
        }

        quote_adjusted / base_adjusted
    }

    /// Get liquidity in USDC terms
    pub fn get_liquidity_usdc(&self, sol_price: f64) -> f64 {
        let sol_mint = "So11111111111111111111111111111111111111112";
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

        if self.base_mint == usdc_mint {
            self.base_reserve as f64 / 1e6
        } else if self.quote_mint == usdc_mint {
            self.quote_reserve as f64 / 1e6
        } else if self.base_mint == sol_mint {
            (self.base_reserve as f64 / 1e9) * sol_price
        } else if self.quote_mint == sol_mint {
            (self.quote_reserve as f64 / 1e9) * sol_price
        } else {
            0.0
        }
    }

    pub fn update_price_history(&mut self) {
        let price = self.calculate_price();
        let now = std::time::Instant::now();

        self.price_history.push(PriceSnapshot {
            price,
            timestamp: now,
        });

        // Keep only last 60 seconds
        let cutoff = now - std::time::Duration::from_secs(60);
        self.price_history.retain(|s| s.timestamp >= cutoff);
    }

    pub fn get_price_change(&self, window_secs: u64) -> Option<f64> {
        if self.price_history.len() < 2 {
            return None;
        }

        let current = self.price_history.last()?;
        let cutoff = current.timestamp - std::time::Duration::from_secs(window_secs);

        let old_price = self
            .price_history
            .iter()
            .rev()
            .skip(1)
            .find(|s| s.timestamp >= cutoff)?;

        if old_price.price == 0.0 {
            return None;
        }

        let change_pct = ((current.price - old_price.price) / old_price.price) * 100.0;
        Some(change_pct)
    }
}
