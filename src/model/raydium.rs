use std::time::Instant;

#[derive(Debug, Clone)]
pub struct RaydiumPoolState {
    pub pool_address: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub price_history: Vec<PriceSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PriceSnapshot {
    pub price: f64,
    pub timestamp: Instant,
}

impl RaydiumPoolState {
    pub fn calculate_price(&self) -> f64 {
        if self.base_reserve == 0 {
            return 0.0;
        }
        (self.quote_reserve as f64) / (self.base_reserve as f64)
    }

    pub fn update_price_history(&mut self) {
        let price = self.calculate_price();
        let now = Instant::now();

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
