use std::sync::Arc;

use crate::matching::engine::MatchingEngine;
use crate::metrics::tracker::MetricsTracker;

/// Shared application state
/// 
/// This is cloned for each request handler, but the inner Arc ensures
/// all handlers share the same underlying data.
#[derive(Clone)]
pub struct AppState {
    /// The core matching engine
    pub engine: Arc<MatchingEngine>,
    /// Metrics tracker for latency and throughput
    pub metrics: Arc<MetricsTracker>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(MatchingEngine::new()),
            metrics: Arc::new(MetricsTracker::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_creation() {
        let state = AppState::new();
        
        assert_eq!(state.engine.orders_received(), 0);
        assert_eq!(state.engine.orders_in_book(), 0);
        assert_eq!(state.metrics.orders_processed(), 0);
    }

    #[test]
    fn test_app_state_clone_shares_data() {
        let state1 = AppState::new();
        let state2 = state1.clone();

        // Submit order via state1
        use crate::models::order::{CreateOrderRequest, OrderType, Side};
        
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(10000),
            quantity: 100,
        };

        state1.engine.submit_order(request).unwrap();

        // Should be visible via state2
        assert_eq!(state2.engine.orders_received(), 1);
        assert_eq!(state2.engine.orders_in_book(), 1);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        
        assert_eq!(state.engine.orders_received(), 0);
        assert_eq!(state.metrics.orders_processed(), 0);
    }
}