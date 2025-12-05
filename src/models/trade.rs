use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::order::Side;

/// Trade information included in order responses
#[derive(Debug, Clone, Serialize)]
pub struct TradeInfo {
    pub trade_id: Uuid,
    pub price: u64,
    pub quantity: u64,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
}

/// Full trade record for internal storage
#[derive(Debug, Clone, Serialize)]
pub struct Trade {
    pub id: Uuid,
    pub symbol: String,
    /// Price at which the trade was executed
    pub price: u64,
    /// Quantity that was traded
    pub quantity: u64,
    /// The order ID of the buyer
    pub buyer_order_id: Uuid,
    /// The order ID of the seller
    pub seller_order_id: Uuid,
    /// Which side was the taker (aggressor)
    pub taker_side: Side,
    /// Timestamp when the trade was executed
    pub executed_at: DateTime<Utc>,
}

impl Trade {
    /// Creates a new trade
    pub fn new(
        symbol: String,
        price: u64,
        quantity: u64,
        buyer_order_id: Uuid,
        seller_order_id: Uuid,
        taker_side: Side,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            symbol,
            price,
            quantity,
            buyer_order_id,
            seller_order_id,
            taker_side,
            executed_at: Utc::now(),
        }
    }

    /// Converts to TradeInfo for API responses
    pub fn to_trade_info(&self) -> TradeInfo {
        TradeInfo {
            trade_id: self.id,
            price: self.price,
            quantity: self.quantity,
            timestamp: self.executed_at.timestamp_millis(),
        }
    }
}

/// Summary of trades executed for an order
#[derive(Debug, Clone, Default)]
pub struct TradeSummary {
    pub total_quantity: u64,
    pub total_value: u64,
    pub trades: Vec<Trade>,
}

impl TradeSummary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_trade(&mut self, trade: Trade) {
        self.total_quantity += trade.quantity;
        self.total_value += trade.price * trade.quantity;
        self.trades.push(trade);
    }

    /// Returns the average execution price
    pub fn average_price(&self) -> Option<f64> {
        if self.total_quantity > 0 {
            Some(self.total_value as f64 / self.total_quantity as f64)
        } else {
            None
        }
    }

    /// Returns trade info list for API responses
    pub fn to_trade_info_list(&self) -> Vec<TradeInfo> {
        self.trades.iter().map(|t| t.to_trade_info()).collect()
    }

    /// Returns the number of trades
    pub fn trade_count(&self) -> usize {
        self.trades.len()
    }

    /// Checks if any trades occurred
    pub fn has_trades(&self) -> bool {
        !self.trades.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_trade() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let trade = Trade::new(
            "AAPL".to_string(),
            15050,
            100,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        assert_eq!(trade.symbol, "AAPL");
        assert_eq!(trade.price, 15050);
        assert_eq!(trade.quantity, 100);
        assert_eq!(trade.buyer_order_id, buyer_id);
        assert_eq!(trade.seller_order_id, seller_id);
        assert_eq!(trade.taker_side, Side::Buy);
    }

    #[test]
    fn test_trade_to_trade_info() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let trade = Trade::new(
            "AAPL".to_string(),
            15050,
            100,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        let info = trade.to_trade_info();

        assert_eq!(info.trade_id, trade.id);
        assert_eq!(info.price, 15050);
        assert_eq!(info.quantity, 100);
        assert!(info.timestamp > 0);
    }

    #[test]
    fn test_trade_summary_empty() {
        let summary = TradeSummary::new();

        assert_eq!(summary.total_quantity, 0);
        assert_eq!(summary.total_value, 0);
        assert_eq!(summary.trade_count(), 0);
        assert!(!summary.has_trades());
        assert!(summary.average_price().is_none());
    }

    #[test]
    fn test_trade_summary_single_trade() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let mut summary = TradeSummary::new();

        let trade = Trade::new(
            "AAPL".to_string(),
            15050,
            100,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        summary.add_trade(trade);

        assert_eq!(summary.total_quantity, 100);
        assert_eq!(summary.total_value, 15050 * 100);
        assert_eq!(summary.trade_count(), 1);
        assert!(summary.has_trades());
        assert_eq!(summary.average_price().unwrap(), 15050.0);
    }

    #[test]
    fn test_trade_summary_multiple_trades() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let mut summary = TradeSummary::new();

        let trade1 = Trade::new(
            "AAPL".to_string(),
            15000,
            50,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        let trade2 = Trade::new(
            "AAPL".to_string(),
            15100,
            50,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        summary.add_trade(trade1);
        summary.add_trade(trade2);

        assert_eq!(summary.total_quantity, 100);
        assert_eq!(summary.total_value, 15000 * 50 + 15100 * 50);
        assert_eq!(summary.trade_count(), 2);
        assert!(summary.has_trades());

        let avg_price = summary.average_price().unwrap();
        assert!((avg_price - 15050.0).abs() < 0.01);
    }

    #[test]
    fn test_trade_summary_to_trade_info_list() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let mut summary = TradeSummary::new();

        let trade1 = Trade::new(
            "AAPL".to_string(),
            15000,
            50,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        let trade2 = Trade::new(
            "AAPL".to_string(),
            15100,
            50,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        let trade1_id = trade1.id;
        let trade2_id = trade2.id;

        summary.add_trade(trade1);
        summary.add_trade(trade2);

        let info_list = summary.to_trade_info_list();

        assert_eq!(info_list.len(), 2);
        assert_eq!(info_list[0].trade_id, trade1_id);
        assert_eq!(info_list[0].price, 15000);
        assert_eq!(info_list[0].quantity, 50);
        assert_eq!(info_list[1].trade_id, trade2_id);
        assert_eq!(info_list[1].price, 15100);
        assert_eq!(info_list[1].quantity, 50);
    }

    #[test]
    fn test_trade_summary_weighted_average() {
        let buyer_id = Uuid::new_v4();
        let seller_id = Uuid::new_v4();

        let mut summary = TradeSummary::new();

        // 75 shares at $100
        let trade1 = Trade::new(
            "TEST".to_string(),
            10000, // $100.00 in cents
            75,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        // 25 shares at $120
        let trade2 = Trade::new(
            "TEST".to_string(),
            12000, // $120.00 in cents
            25,
            buyer_id,
            seller_id,
            Side::Buy,
        );

        summary.add_trade(trade1);
        summary.add_trade(trade2);

        // Weighted average: (75 * 10000 + 25 * 12000) / 100 = 10500
        let avg_price = summary.average_price().unwrap();
        assert!((avg_price - 10500.0).abs() < 0.01);
    }
}