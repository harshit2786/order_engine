use serde::Serialize;

/// A single price level in the order book response
#[derive(Debug, Clone, Serialize)]
pub struct PriceLevelInfo {
    pub price: u64,
    pub quantity: u64,
}

/// Response for GET /api/v1/orderbook/{symbol}
#[derive(Debug, Clone, Serialize)]
pub struct OrderBookResponse {
    pub symbol: String,
    pub timestamp: i64,
    /// Bids sorted by price descending (highest first)
    pub bids: Vec<PriceLevelInfo>,
    /// Asks sorted by price ascending (lowest first)
    pub asks: Vec<PriceLevelInfo>,
}

impl OrderBookResponse {
    pub fn new(symbol: String, bids: Vec<PriceLevelInfo>, asks: Vec<PriceLevelInfo>) -> Self {
        Self {
            symbol,
            timestamp: chrono::Utc::now().timestamp_millis(),
            bids,
            asks,
        }
    }

    pub fn empty(symbol: String) -> Self {
        Self {
            symbol,
            timestamp: chrono::Utc::now().timestamp_millis(),
            bids: Vec::new(),
            asks: Vec::new(),
        }
    }
}