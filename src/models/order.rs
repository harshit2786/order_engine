use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::trade::TradeInfo;

/// Side of the order - Buy or Sell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

/// Type of order - Limit or Market
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Limit,
    Market,
}

/// Current status of an order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderStatus {
    /// Order has been accepted and added to the book
    Accepted,
    /// Order has been partially filled (only for LIMIT orders)
    #[serde(rename = "PARTIAL_FILL")]
    PartialFill,
    /// Order has been completely filled
    Filled,
    /// Order has been cancelled by the user
    Cancelled,
}

/// Request payload for creating a new order
#[derive(Debug, Clone, Deserialize)]
pub struct CreateOrderRequest {
    pub symbol: String,
    pub side: Side,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    /// Price in the smallest unit (e.g., cents). Required for LIMIT orders.
    pub price: Option<u64>,
    pub quantity: u64,
}

impl CreateOrderRequest {
    /// Validates the order request
    pub fn validate(&self) -> Result<(), String> {
        if self.symbol.is_empty() {
            return Err("Symbol cannot be empty".to_string());
        }

        if self.quantity == 0 {
            return Err("Quantity must be positive".to_string());
        }

        match self.order_type {
            OrderType::Limit => {
                if self.price.is_none() {
                    return Err("Price is required for LIMIT orders".to_string());
                }
                if self.price == Some(0) {
                    return Err("Price must be positive".to_string());
                }
            }
            OrderType::Market => {
                if self.price.is_some() {
                    return Err("Price should not be specified for MARKET orders".to_string());
                }
            }
        }

        Ok(())
    }
}

/// Response for order submission - Accepted (added to book, no fills)
#[derive(Debug, Clone, Serialize)]
pub struct OrderAcceptedResponse {
    pub order_id: Uuid,
    pub status: OrderStatus,
    pub message: String,
}

/// Response for order submission - Partial Fill
#[derive(Debug, Clone, Serialize)]
pub struct OrderPartialFillResponse {
    pub order_id: Uuid,
    pub status: OrderStatus,
    pub filled_quantity: u64,
    pub remaining_quantity: u64,
    pub trades: Vec<TradeInfo>,
}

/// Response for order submission - Fully Filled
#[derive(Debug, Clone, Serialize)]
pub struct OrderFilledResponse {
    pub order_id: Uuid,
    pub status: OrderStatus,
    pub filled_quantity: u64,
    pub trades: Vec<TradeInfo>,
}

/// Unified response enum for order submission
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SubmitOrderResponse {
    Accepted(OrderAcceptedResponse),
    PartialFill(OrderPartialFillResponse),
    Filled(OrderFilledResponse),
}

impl SubmitOrderResponse {
    /// Creates an Accepted response
    pub fn accepted(order_id: Uuid) -> Self {
        Self::Accepted(OrderAcceptedResponse {
            order_id,
            status: OrderStatus::Accepted,
            message: "Order added to book".to_string(),
        })
    }

    /// Creates a Partial Fill response
    pub fn partial_fill(
        order_id: Uuid,
        filled_quantity: u64,
        remaining_quantity: u64,
        trades: Vec<TradeInfo>,
    ) -> Self {
        Self::PartialFill(OrderPartialFillResponse {
            order_id,
            status: OrderStatus::PartialFill,
            filled_quantity,
            remaining_quantity,
            trades,
        })
    }

    /// Creates a Filled response
    pub fn filled(order_id: Uuid, filled_quantity: u64, trades: Vec<TradeInfo>) -> Self {
        Self::Filled(OrderFilledResponse {
            order_id,
            status: OrderStatus::Filled,
            filled_quantity,
            trades,
        })
    }
}

/// Response for getting order status
#[derive(Debug, Clone, Serialize)]
pub struct OrderStatusResponse {
    pub order_id: Uuid,
    pub symbol: String,
    pub side: Side,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    pub quantity: u64,
    pub filled_quantity: u64,
    pub remaining_quantity: u64,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Response for cancel order endpoint
#[derive(Debug, Clone, Serialize)]
pub struct CancelOrderResponse {
    pub order_id: Uuid,
    pub status: OrderStatus,
}

impl CancelOrderResponse {
    pub fn new(order_id: Uuid) -> Self {
        Self {
            order_id,
            status: OrderStatus::Cancelled,
        }
    }
}

/// Internal representation of an order
#[derive(Debug, Clone)]
pub struct Order {
    pub id: Uuid,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub price: Option<u64>,
    pub quantity: u64,
    pub filled_quantity: u64,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Order {
    /// Creates a new order from a request
    pub fn new(request: CreateOrderRequest) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            symbol: request.symbol,
            side: request.side,
            order_type: request.order_type,
            price: request.price,
            quantity: request.quantity,
            filled_quantity: 0,
            status: OrderStatus::Accepted,
            created_at: now,
            updated_at: now,
        }
    }

    /// Returns the remaining quantity to be filled
    pub fn remaining_quantity(&self) -> u64 {
        self.quantity.saturating_sub(self.filled_quantity)
    }

    /// Checks if the order is fully filled
    pub fn is_filled(&self) -> bool {
        self.filled_quantity >= self.quantity
    }

    /// Checks if the order can be matched (is active in the book)
    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            OrderStatus::Accepted | OrderStatus::PartialFill
        )
    }

    /// Fills the order by the given quantity
    pub fn fill(&mut self, quantity: u64) {
        self.filled_quantity += quantity;
        self.updated_at = Utc::now();

        if self.is_filled() {
            self.status = OrderStatus::Filled;
        } else if self.filled_quantity > 0 {
            self.status = OrderStatus::PartialFill;
        }
    }

    /// Cancels the order
    pub fn cancel(&mut self) {
        self.status = OrderStatus::Cancelled;
        self.updated_at = Utc::now();
    }

    /// Converts the order to a status response
    pub fn to_status_response(&self) -> OrderStatusResponse {
        OrderStatusResponse {
            order_id: self.id,
            symbol: self.symbol.clone(),
            side: self.side,
            order_type: self.order_type,
            price: self.price,
            quantity: self.quantity,
            filled_quantity: self.filled_quantity,
            remaining_quantity: self.remaining_quantity(),
            status: self.status,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_limit_order() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        assert!(request.validate().is_ok());

        let order = Order::new(request);
        assert_eq!(order.symbol, "AAPL");
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.price, Some(15050));
        assert_eq!(order.quantity, 100);
        assert_eq!(order.filled_quantity, 0);
        assert_eq!(order.status, OrderStatus::Accepted);
    }

    #[test]
    fn test_create_market_order() {
        let request = CreateOrderRequest {
            symbol: "BTC".to_string(),
            side: Side::Sell,
            order_type: OrderType::Market,
            price: None,
            quantity: 50,
        };

        assert!(request.validate().is_ok());

        let order = Order::new(request);
        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.price, None);
    }

    #[test]
    fn test_limit_order_without_price_fails() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: None,
            quantity: 100,
        };

        let result = request.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Price is required for LIMIT orders");
    }

    #[test]
    fn test_market_order_with_price_fails() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            price: Some(15050),
            quantity: 100,
        };

        let result = request.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Price should not be specified for MARKET orders"
        );
    }

    #[test]
    fn test_zero_quantity_fails() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 0,
        };

        let result = request.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Quantity must be positive");
    }

    #[test]
    fn test_order_fill_partial() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        let mut order = Order::new(request);

        order.fill(30);
        assert_eq!(order.filled_quantity, 30);
        assert_eq!(order.remaining_quantity(), 70);
        assert_eq!(order.status, OrderStatus::PartialFill);
        assert!(!order.is_filled());
        assert!(order.is_active());
    }

    #[test]
    fn test_order_fill_complete() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        let mut order = Order::new(request);

        order.fill(100);
        assert_eq!(order.filled_quantity, 100);
        assert_eq!(order.remaining_quantity(), 0);
        assert_eq!(order.status, OrderStatus::Filled);
        assert!(order.is_filled());
        assert!(!order.is_active());
    }

    #[test]
    fn test_order_fill_multiple_times() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        let mut order = Order::new(request);

        order.fill(30);
        assert_eq!(order.status, OrderStatus::PartialFill);

        order.fill(70);
        assert_eq!(order.filled_quantity, 100);
        assert_eq!(order.status, OrderStatus::Filled);
        assert!(order.is_filled());
    }

    #[test]
    fn test_order_cancel() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        let mut order = Order::new(request);
        order.cancel();

        assert_eq!(order.status, OrderStatus::Cancelled);
        assert!(!order.is_active());
    }

    #[test]
    fn test_order_to_status_response() {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(15050),
            quantity: 100,
        };

        let mut order = Order::new(request);
        order.fill(40);

        let response = order.to_status_response();

        assert_eq!(response.order_id, order.id);
        assert_eq!(response.symbol, "AAPL");
        assert_eq!(response.side, Side::Buy);
        assert_eq!(response.order_type, OrderType::Limit);
        assert_eq!(response.price, Some(15050));
        assert_eq!(response.quantity, 100);
        assert_eq!(response.filled_quantity, 40);
        assert_eq!(response.remaining_quantity, 60);
        assert_eq!(response.status, OrderStatus::PartialFill);
    }

    #[test]
    fn test_submit_order_response_accepted() {
        let order_id = Uuid::new_v4();
        let response = SubmitOrderResponse::accepted(order_id);

        if let SubmitOrderResponse::Accepted(r) = response {
            assert_eq!(r.order_id, order_id);
            assert_eq!(r.status, OrderStatus::Accepted);
            assert_eq!(r.message, "Order added to book");
        } else {
            panic!("Expected Accepted response");
        }
    }

    #[test]
    fn test_submit_order_response_partial_fill() {
        let order_id = Uuid::new_v4();
        let trade_info = TradeInfo {
            trade_id: Uuid::new_v4(),
            price: 15050,
            quantity: 60,
            timestamp: Utc::now().timestamp_millis(),
        };

        let response = SubmitOrderResponse::partial_fill(order_id, 60, 40, vec![trade_info]);

        if let SubmitOrderResponse::PartialFill(r) = response {
            assert_eq!(r.order_id, order_id);
            assert_eq!(r.status, OrderStatus::PartialFill);
            assert_eq!(r.filled_quantity, 60);
            assert_eq!(r.remaining_quantity, 40);
            assert_eq!(r.trades.len(), 1);
        } else {
            panic!("Expected PartialFill response");
        }
    }

    #[test]
    fn test_submit_order_response_filled() {
        let order_id = Uuid::new_v4();
        let trade_info = TradeInfo {
            trade_id: Uuid::new_v4(),
            price: 15050,
            quantity: 100,
            timestamp: Utc::now().timestamp_millis(),
        };

        let response = SubmitOrderResponse::filled(order_id, 100, vec![trade_info]);

        if let SubmitOrderResponse::Filled(r) = response {
            assert_eq!(r.order_id, order_id);
            assert_eq!(r.status, OrderStatus::Filled);
            assert_eq!(r.filled_quantity, 100);
            assert_eq!(r.trades.len(), 1);
        } else {
            panic!("Expected Filled response");
        }
    }
}