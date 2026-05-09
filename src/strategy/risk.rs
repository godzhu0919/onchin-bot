use crate::model::state::{PumpState, RaydiumState};

/// Risk assessment for arbitrage opportunities
#[derive(Debug, Clone)]
pub struct RiskAssessment {
    pub is_safe: bool,
    pub risk_score: f64, // 0-100, lower is safer
    pub warnings: Vec<String>,
}

/// Assess risk factors for an arbitrage trade
pub fn assess_risk(
    pump_state: &PumpState,
    raydium_state: &RaydiumState,
    trade_size_usdc: f64,
    sol_usdc_price: f64,
) -> RiskAssessment {
    let mut warnings = Vec::new();
    let mut risk_score = 0.0;

    // Check liquidity depth
    let pump_liquidity_sol = pump_state.sol_reserve as f64 / 1e9;
    let pump_liquidity_usdc = pump_liquidity_sol * sol_usdc_price;
    let raydium_liquidity_usdc = raydium_state.get_liquidity_usdc(sol_usdc_price);

    // Risk: Low liquidity (need at least 20x trade size for safety)
    if pump_liquidity_usdc < trade_size_usdc * 20.0 {
        warnings.push(format!(
            "Low Pump liquidity: ${:.0} (need 20x trade size)",
            pump_liquidity_usdc
        ));
        risk_score += 35.0;
    }

    if raydium_liquidity_usdc < trade_size_usdc * 20.0 {
        warnings.push(format!(
            "Low Raydium liquidity: ${:.0} (need 20x trade size)",
            raydium_liquidity_usdc
        ));
        risk_score += 35.0;
    }

    // Risk: Price volatility (check recent price changes)
    if let Some(change_5s) = pump_state.get_price_change(5) {
        if change_5s.abs() > 5.0 {
            warnings.push(format!("High volatility: {:.1}% in 5s", change_5s));
            risk_score += 25.0;
        }
    }

    if let Some(change_10s) = raydium_state.get_price_change(10) {
        if change_10s.abs() > 3.0 {
            warnings.push(format!("Raydium volatility: {:.1}% in 10s", change_10s));
            risk_score += 15.0;
        }
    }

    // Risk: Large trade relative to pool size (should be <2% for low slippage)
    let trade_pct_pump = (trade_size_usdc / pump_liquidity_usdc) * 100.0;
    let trade_pct_raydium = (trade_size_usdc / raydium_liquidity_usdc) * 100.0;

    if trade_pct_pump > 2.0 {
        warnings.push(format!(
            "Large trade vs Pump: {:.1}% of pool",
            trade_pct_pump
        ));
        risk_score += 30.0;
    }

    if trade_pct_raydium > 2.0 {
        warnings.push(format!(
            "Large trade vs Raydium: {:.1}% of pool",
            trade_pct_raydium
        ));
        risk_score += 30.0;
    }

    // Risk: Very small pools (rug pull risk)
    if pump_liquidity_usdc < 5000.0 {
        warnings.push(format!(
            "Small Pump pool: ${:.0} (rug risk)",
            pump_liquidity_usdc
        ));
        risk_score += 40.0;
    }

    if raydium_liquidity_usdc < 10000.0 {
        warnings.push(format!(
            "Small Raydium pool: ${:.0}",
            raydium_liquidity_usdc
        ));
        risk_score += 20.0;
    }

    // Risk: Insufficient price history
    if pump_state.price_history.len() < 3 {
        warnings.push("Insufficient Pump price history".to_string());
        risk_score += 15.0;
    }

    let is_safe = risk_score < 50.0 && warnings.len() < 3;

    RiskAssessment {
        is_safe,
        risk_score,
        warnings,
    }
}

/// Check if conditions are favorable for executing arbitrage
pub fn should_execute(risk: &RiskAssessment, profit_pct: f64, min_profit_pct: f64) -> bool {
    // Must be profitable
    if profit_pct < min_profit_pct {
        return false;
    }

    // Must pass risk assessment
    if !risk.is_safe {
        return false;
    }

    // Higher profit required for higher risk
    let required_profit = min_profit_pct + (risk.risk_score / 10.0);
    profit_pct >= required_profit
}
