use std::collections::HashMap;

use parking_lot::RwLock;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::order::{
    CancelOrderResponse,
    CreateOrderRequest,
    Order,
    OrderStatus,
    OrderStatusResponse,
    OrderType,
    Side,
    SubmitOrderResponse,
};
use crate::models::orderbook::OrderBookResponse;
use crate::models::trade::{ Trade, TradeInfo };
use crate::orderbook::book::OrderBook;

/// Result of a matching operation
#[derive(Debug)]
pub struct MatchResult {
    pub trades: Vec<Trade>,
    pub fully_filled: bool,
}

impl MatchResult {
    pub fn new() -> Self {
        Self {
            trades: Vec::new(),
            fully_filled: false,
        }
    }

    pub fn to_trade_info_list(&self) -> Vec<TradeInfo> {
        self.trades
            .iter()
            .map(|t| t.to_trade_info())
            .collect()
    }

    pub fn total_filled_quantity(&self) -> u64 {
        self.trades
            .iter()
            .map(|t| t.quantity)
            .sum()
    }
}

impl Default for MatchResult {
    fn default() -> Self {
        Self::new()
    }
}

/// The core matching engine
///
/// Thread-safe via RwLock - allows concurrent reads, exclusive writes
pub struct MatchingEngine {
    /// Order books per symbol
    order_books: RwLock<HashMap<String, OrderBook>>,
    /// All orders for status lookup (includes filled/cancelled)
    orders: RwLock<HashMap<Uuid, Order>>,
    /// Trade counter for metrics
    trade_count: RwLock<u64>,
}

impl MatchingEngine {
    pub fn new() -> Self {
        Self {
            order_books: RwLock::new(HashMap::new()),
            orders: RwLock::new(HashMap::new()),
            trade_count: RwLock::new(0),
        }
    }

    /// Submits a new order for processing
    pub fn submit_order(
        &self,
        request: CreateOrderRequest
    ) -> Result<SubmitOrderResponse, AppError> {
        // Validate request
        request.validate().map_err(|e| AppError::InvalidOrder(e))?;

        let mut order = Order::new(request);
        let symbol = order.symbol.clone();

        // Process based on order type
        let response = match order.order_type {
            OrderType::Market => self.process_market_order(&mut order, &symbol)?,
            OrderType::Limit => self.process_limit_order(&mut order, &symbol)?,
        };

        Ok(response)
    }

    /// Processes a market order
    /// Market orders must be fully filled or rejected
    fn process_market_order(
        &self,
        order: &mut Order,
        symbol: &str
    ) -> Result<SubmitOrderResponse, AppError> {
        let mut order_books = self.order_books.write();

        // Get or create order book for symbol
        let book = order_books
            .entry(symbol.to_string())
            .or_insert_with(|| OrderBook::new(symbol.to_string()));

        // Check available liquidity first
        let available_quantity = match order.side {
            Side::Buy => book.total_ask_quantity(),
            Side::Sell => book.total_bid_quantity(),
        };

        if available_quantity < order.quantity {
            return Err(AppError::InsufficientLiquidity {
                available: available_quantity,
                requested: order.quantity,
            });
        }

        // Match the order
        let match_result = self.match_order(order, book);

        // Market order should be fully filled at this point
        assert!(order.is_filled(), "Market order should be fully filled after matching");

        // Update trade count
        {
            let mut count = self.trade_count.write();
            *count += match_result.trades.len() as u64;
        }

        // Store the order for status queries
        {
            let mut orders = self.orders.write();
            orders.insert(order.id, order.clone());
        }

        Ok(
            SubmitOrderResponse::filled(
                order.id,
                order.filled_quantity,
                match_result.to_trade_info_list()
            )
        )
    }

    /// Processes a limit order
    /// Limit orders can be partially filled and rest added to book
    fn process_limit_order(
        &self,
        order: &mut Order,
        symbol: &str
    ) -> Result<SubmitOrderResponse, AppError> {
        let mut order_books = self.order_books.write();

        // Get or create order book for symbol
        let book = order_books
            .entry(symbol.to_string())
            .or_insert_with(|| OrderBook::new(symbol.to_string()));

        // Try to match the order
        let match_result = self.match_order(order, book);

        // Update trade count
        if !match_result.trades.is_empty() {
            let mut count = self.trade_count.write();
            *count += match_result.trades.len() as u64;
        }

        // Build response based on fill status
        let response = if order.is_filled() {
            // Fully filled - don't add to book
            SubmitOrderResponse::filled(
                order.id,
                order.filled_quantity,
                match_result.to_trade_info_list()
            )
        } else if order.filled_quantity > 0 {
            // Partially filled - add remaining to book
            book.add_order(order.clone());
            SubmitOrderResponse::partial_fill(
                order.id,
                order.filled_quantity,
                order.remaining_quantity(),
                match_result.to_trade_info_list()
            )
        } else {
            // No fill - add to book
            book.add_order(order.clone());
            SubmitOrderResponse::accepted(order.id)
        };

        // Store the order for status queries
        {
            let mut orders = self.orders.write();
            orders.insert(order.id, order.clone());
        }

        Ok(response)
    }

    /// Core matching logic
    fn match_order(&self, order: &mut Order, book: &mut OrderBook) -> MatchResult {
        let mut result = MatchResult::new();

        match order.side {
            Side::Buy => self.match_buy_order(order, book, &mut result),
            Side::Sell => self.match_sell_order(order, book, &mut result),
        }

        result.fully_filled = order.is_filled();
        result
    }

    /// Matches a buy order against asks
    fn match_buy_order(&self, order: &mut Order, book: &mut OrderBook, result: &mut MatchResult) {
        let order_price = order.price; // None for market orders

        // Collect prices to process (to avoid borrow issues)
        let ask_prices: Vec<u64> = book
            .asks()
            .keys()
            .copied()
            .take_while(|&ask_price| {
                // Market orders match any price
                // Limit orders only match if ask <= bid
                order_price.map_or(true, |limit_price| ask_price <= limit_price)
            })
            .collect();

        for ask_price in ask_prices {
            if order.is_filled() {
                break;
            }

            self.match_at_price_level(order, book, Side::Sell, ask_price, result);
        }
    }

    /// Matches a sell order against bids
    fn match_sell_order(&self, order: &mut Order, book: &mut OrderBook, result: &mut MatchResult) {
        let order_price = order.price; // None for market orders

        // Collect prices to process (highest first for bids)
        let bid_prices: Vec<u64> = book
            .bids()
            .keys()
            .rev()
            .copied()
            .take_while(|&bid_price| {
                // Market orders match any price
                // Limit orders only match if bid >= ask
                order_price.map_or(true, |limit_price| bid_price >= limit_price)
            })
            .collect();

        for bid_price in bid_prices {
            if order.is_filled() {
                break;
            }

            self.match_at_price_level(order, book, Side::Buy, bid_price, result);
        }
    }

    /// Matches orders at a specific price level
    fn match_at_price_level(
        &self,
        incoming_order: &mut Order,
        book: &mut OrderBook,
        book_side: Side,
        price: u64,
        result: &mut MatchResult
    ) {
        // Track orders to remove
        let mut orders_to_remove: Vec<Uuid> = Vec::new();
        let mut filled_order_ids_and_qty: Vec<(Uuid, u64)> = Vec::new();

        // Scope the mutable borrow of book_levels
        {
            let book_levels = match book_side {
                Side::Buy => book.bids_mut(),
                Side::Sell => book.asks_mut(),
            };

            let level = match book_levels.get_mut(&price) {
                Some(l) => l,
                None => {
                    return;
                }
            };

            while !incoming_order.is_filled() {
                let resting_order = match level.front_mut() {
                    Some(o) => o,
                    None => {
                        break;
                    }
                };

                // Calculate fill quantity
                let fill_qty = std::cmp::min(
                    incoming_order.remaining_quantity(),
                    resting_order.remaining_quantity()
                );

                // Determine buyer and seller
                let (buyer_id, seller_id) = match incoming_order.side {
                    Side::Buy => (incoming_order.id, resting_order.id),
                    Side::Sell => (resting_order.id, incoming_order.id),
                };

                let resting_order_id = resting_order.id;

                // Create trade
                let trade = Trade::new(
                    incoming_order.symbol.clone(),
                    price,
                    fill_qty,
                    buyer_id,
                    seller_id,
                    incoming_order.side
                );

                // Update orders
                incoming_order.fill(fill_qty);
                resting_order.fill(fill_qty);

                // Track for storage update
                filled_order_ids_and_qty.push((resting_order_id, fill_qty));

                result.trades.push(trade);

                // If resting order is filled, remove from level
                if resting_order.is_filled() {
                    level.pop_front();
                    orders_to_remove.push(resting_order_id);
                } else {
                    // Partially filled, reduce level quantity
                    level.reduce_total_quantity(fill_qty);
                }
            }
        }
        // Mutable borrow of book_levels ends here

        // Update resting orders in storage
        {
            let mut orders = self.orders.write();
            for (order_id, fill_qty) in filled_order_ids_and_qty {
                if let Some(stored_order) = orders.get_mut(&order_id) {
                    stored_order.fill(fill_qty);
                }
            }
        }

        // Remove filled orders from location tracking
        for order_id in orders_to_remove {
            book.remove_order_location(order_id);
        }

        // Clean up empty price level
        let book_levels = match book_side {
            Side::Buy => book.bids_mut(),
            Side::Sell => book.asks_mut(),
        };

        if let Some(level) = book_levels.get(&price) {
            if level.is_empty() {
                book_levels.remove(&price);
            }
        }
    }
    /// Cancels an order by ID
    pub fn cancel_order(&self, order_id: Uuid) -> Result<CancelOrderResponse, AppError> {
        // Check if order exists and its status
        {
            let orders = self.orders.read();
            let order = orders.get(&order_id).ok_or(AppError::OrderNotFound(order_id))?;

            // Cannot cancel filled orders
            if order.status == OrderStatus::Filled {
                return Err(AppError::CannotCancelFilledOrder(order_id));
            }

            // Cannot cancel already cancelled orders
            if order.status == OrderStatus::Cancelled {
                return Err(AppError::OrderNotFound(order_id));
            }
        }

        // Remove from order book
        {
            let mut order_books = self.order_books.write();

            // Find and remove from appropriate order book
            for book in order_books.values_mut() {
                if book.remove_order(order_id).is_some() {
                    break;
                }
            }
        }

        // Update order status
        {
            let mut orders = self.orders.write();
            if let Some(order) = orders.get_mut(&order_id) {
                order.cancel();
            }
        }

        Ok(CancelOrderResponse::new(order_id))
    }

    /// Gets order status by ID
    pub fn get_order(&self, order_id: Uuid) -> Result<OrderStatusResponse, AppError> {
        let orders = self.orders.read();
        let order = orders.get(&order_id).ok_or(AppError::OrderNotFound(order_id))?;

        Ok(order.to_status_response())
    }

    /// Gets order book for a symbol
    pub fn get_order_book(&self, symbol: &str, depth: usize) -> OrderBookResponse {
        let order_books = self.order_books.read();

        match order_books.get(symbol) {
            Some(book) => book.to_response(depth),
            None => OrderBookResponse::empty(symbol.to_string()),
        }
    }

    /// Gets total number of orders currently in all order books
    pub fn orders_in_book(&self) -> usize {
        let order_books = self.order_books.read();
        order_books
            .values()
            .map(|b| b.total_order_count())
            .sum()
    }

    /// Gets total number of trades executed
    pub fn trades_executed(&self) -> u64 {
        *self.trade_count.read()
    }

    /// Gets total number of orders received
    pub fn orders_received(&self) -> usize {
        self.orders.read().len()
    }

    /// Gets count of orders that were matched (partially or fully)
    pub fn orders_matched(&self) -> usize {
        let orders = self.orders.read();
        orders
            .values()
            .filter(|o| o.filled_quantity > 0)
            .count()
    }

    /// Gets count of cancelled orders
    pub fn orders_cancelled(&self) -> usize {
        let orders = self.orders.read();
        orders
            .values()
            .filter(|o| o.status == OrderStatus::Cancelled)
            .count()
    }
}

impl Default for MatchingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_limit_request(side: Side, price: u64, quantity: u64) -> CreateOrderRequest {
        CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
        }
    }

    fn create_market_request(side: Side, quantity: u64) -> CreateOrderRequest {
        CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side,
            order_type: OrderType::Market,
            price: None,
            quantity,
        }
    }
    // ==================== Submit Order Tests ====================

    #[test]
    fn test_submit_limit_order_no_match() {
        let engine = MatchingEngine::new();

        let request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(request).unwrap();

        match response {
            SubmitOrderResponse::Accepted(r) => {
                assert_eq!(r.status, OrderStatus::Accepted);
                assert_eq!(r.message, "Order added to book");
            }
            _ => panic!("Expected Accepted response"),
        }

        assert_eq!(engine.orders_in_book(), 1);
        assert_eq!(engine.orders_received(), 1);
    }

    #[test]
    fn test_submit_limit_order_full_match() {
        let engine = MatchingEngine::new();

        // Add sell order to book
        let sell_request = create_limit_request(Side::Sell, 10000, 100);
        engine.submit_order(sell_request).unwrap();

        // Submit matching buy order
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.status, OrderStatus::Filled);
                assert_eq!(r.filled_quantity, 100);
                assert_eq!(r.trades.len(), 1);
                assert_eq!(r.trades[0].price, 10000);
                assert_eq!(r.trades[0].quantity, 100);
            }
            _ => panic!("Expected Filled response"),
        }

        assert_eq!(engine.orders_in_book(), 0);
        assert_eq!(engine.trades_executed(), 1);
    }

    #[test]
    fn test_submit_limit_order_partial_match() {
        let engine = MatchingEngine::new();

        // Add sell order to book (50 qty)
        let sell_request = create_limit_request(Side::Sell, 10000, 50);
        engine.submit_order(sell_request).unwrap();

        // Submit buy order for more (100 qty)
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::PartialFill(r) => {
                assert_eq!(r.status, OrderStatus::PartialFill);
                assert_eq!(r.filled_quantity, 50);
                assert_eq!(r.remaining_quantity, 50);
                assert_eq!(r.trades.len(), 1);
            }
            _ => panic!("Expected PartialFill response"),
        }

        assert_eq!(engine.orders_in_book(), 1);
    }

    #[test]
    fn test_submit_market_order_full_match() {
        let engine = MatchingEngine::new();

        // Add sell order to book
        let sell_request = create_limit_request(Side::Sell, 10000, 100);
        engine.submit_order(sell_request).unwrap();

        // Submit market buy order
        let buy_request = create_market_request(Side::Buy, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.status, OrderStatus::Filled);
                assert_eq!(r.filled_quantity, 100);
            }
            _ => panic!("Expected Filled response"),
        }

        assert_eq!(engine.orders_in_book(), 0);
    }

    #[test]
    fn test_submit_market_order_insufficient_liquidity() {
        let engine = MatchingEngine::new();

        // Add sell order to book (only 50 qty)
        let sell_request = create_limit_request(Side::Sell, 10000, 50);
        engine.submit_order(sell_request).unwrap();

        // Submit market buy order for more (100 qty)
        let buy_request = create_market_request(Side::Buy, 100);
        let response = engine.submit_order(buy_request);

        match response {
            Err(AppError::InsufficientLiquidity { available, requested }) => {
                assert_eq!(available, 50);
                assert_eq!(requested, 100);
            }
            _ => panic!("Expected InsufficientLiquidity error"),
        }

        // Original sell order should still be in book
        assert_eq!(engine.orders_in_book(), 1);
    }

    #[test]
    fn test_submit_market_order_no_liquidity() {
        let engine = MatchingEngine::new();

        // Submit market buy order with empty book
        let buy_request = create_market_request(Side::Buy, 100);
        let response = engine.submit_order(buy_request);

        match response {
            Err(AppError::InsufficientLiquidity { available, requested }) => {
                assert_eq!(available, 0);
                assert_eq!(requested, 100);
            }
            _ => panic!("Expected InsufficientLiquidity error"),
        }
    }

    #[test]
    fn test_submit_market_sell_order() {
        let engine = MatchingEngine::new();

        // Add buy order to book
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        engine.submit_order(buy_request).unwrap();

        // Submit market sell order
        let sell_request = create_market_request(Side::Sell, 100);
        let response = engine.submit_order(sell_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.status, OrderStatus::Filled);
                assert_eq!(r.filled_quantity, 100);
            }
            _ => panic!("Expected Filled response"),
        }

        assert_eq!(engine.orders_in_book(), 0);
    }

    // ==================== Price Priority Tests ====================

    #[test]
    fn test_price_priority_buy_matches_lowest_ask() {
        let engine = MatchingEngine::new();

        // Add sell orders at different prices
        engine.submit_order(create_limit_request(Side::Sell, 10200, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10100, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10000, 50)).unwrap();

        // Buy order should match lowest ask first
        let buy_request = create_limit_request(Side::Buy, 10200, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.trades.len(), 2);
                assert_eq!(r.trades[0].price, 10000);
                assert_eq!(r.trades[0].quantity, 50);
                assert_eq!(r.trades[1].price, 10100);
                assert_eq!(r.trades[1].quantity, 50);
            }
            _ => panic!("Expected Filled response"),
        }
    }

    #[test]
    fn test_price_priority_sell_matches_highest_bid() {
        let engine = MatchingEngine::new();

        // Add buy orders at different prices
        engine.submit_order(create_limit_request(Side::Buy, 9800, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Buy, 9900, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Buy, 10000, 50)).unwrap();

        // Sell order should match highest bid first
        let sell_request = create_limit_request(Side::Sell, 9800, 100);
        let response = engine.submit_order(sell_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.trades.len(), 2);
                assert_eq!(r.trades[0].price, 10000);
                assert_eq!(r.trades[0].quantity, 50);
                assert_eq!(r.trades[1].price, 9900);
                assert_eq!(r.trades[1].quantity, 50);
            }
            _ => panic!("Expected Filled response"),
        }
    }

    // ==================== Time Priority Tests ====================

    #[test]
    fn test_time_priority_fifo() {
        let engine = MatchingEngine::new();

        // Add two sell orders at same price
        let sell1 = create_limit_request(Side::Sell, 10000, 50);
        let response1 = engine.submit_order(sell1).unwrap();
        let order1_id = match response1 {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        let sell2 = create_limit_request(Side::Sell, 10000, 50);
        let response2 = engine.submit_order(sell2).unwrap();
        let order2_id = match response2 {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Buy should match first order first (FIFO)
        let buy_request = create_limit_request(Side::Buy, 10000, 50);
        engine.submit_order(buy_request).unwrap();

        // First order should be filled, second should still be open
        let order1_status = engine.get_order(order1_id).unwrap();
        let order2_status = engine.get_order(order2_id).unwrap();

        assert_eq!(order1_status.status, OrderStatus::Filled);
        assert_eq!(order2_status.status, OrderStatus::Accepted);
    }

    // ==================== Cancel Order Tests ====================

    #[test]
    fn test_cancel_order_success() {
        let engine = MatchingEngine::new();

        // Submit order
        let request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Cancel order
        let cancel_response = engine.cancel_order(order_id).unwrap();
        assert_eq!(cancel_response.order_id, order_id);
        assert_eq!(cancel_response.status, OrderStatus::Cancelled);

        // Order should be removed from book
        assert_eq!(engine.orders_in_book(), 0);

        // Order should still exist in storage with cancelled status
        let status = engine.get_order(order_id).unwrap();
        assert_eq!(status.status, OrderStatus::Cancelled);
    }

    #[test]
    fn test_cancel_order_not_found() {
        let engine = MatchingEngine::new();

        let fake_id = Uuid::new_v4();
        let result = engine.cancel_order(fake_id);

        match result {
            Err(AppError::OrderNotFound(id)) => assert_eq!(id, fake_id),
            _ => panic!("Expected OrderNotFound error"),
        }
    }

    #[test]
    fn test_cancel_filled_order_fails() {
        let engine = MatchingEngine::new();

        // Add sell order
        let sell_request = create_limit_request(Side::Sell, 10000, 100);
        let sell_response = engine.submit_order(sell_request).unwrap();
        let sell_order_id = match sell_response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Fill it with a buy order
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        engine.submit_order(buy_request).unwrap();

        // Try to cancel the filled sell order
        let result = engine.cancel_order(sell_order_id);

        match result {
            Err(AppError::CannotCancelFilledOrder(id)) => assert_eq!(id, sell_order_id),
            _ => panic!("Expected CannotCancelFilledOrder error"),
        }
    }

    #[test]
    fn test_cancel_partially_filled_order() {
        let engine = MatchingEngine::new();

        // Add sell order (50 qty)
        let sell_request = create_limit_request(Side::Sell, 10000, 50);
        engine.submit_order(sell_request).unwrap();

        // Add buy order (100 qty) - will be partially filled
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        let buy_response = engine.submit_order(buy_request).unwrap();
        let buy_order_id = match buy_response {
            SubmitOrderResponse::PartialFill(r) => r.order_id,
            _ => panic!("Expected PartialFill"),
        };

        // Cancel the partially filled buy order
        let cancel_response = engine.cancel_order(buy_order_id).unwrap();
        assert_eq!(cancel_response.status, OrderStatus::Cancelled);

        // Verify order status
        let status = engine.get_order(buy_order_id).unwrap();
        assert_eq!(status.status, OrderStatus::Cancelled);
        assert_eq!(status.filled_quantity, 50);
        assert_eq!(status.remaining_quantity, 50);
    }

    #[test]
    fn test_cancel_already_cancelled_order() {
        let engine = MatchingEngine::new();

        // Submit and cancel order
        let request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        engine.cancel_order(order_id).unwrap();

        // Try to cancel again
        let result = engine.cancel_order(order_id);

        match result {
            Err(AppError::OrderNotFound(_)) => {}
            _ => panic!("Expected OrderNotFound error"),
        }
    }

    // ==================== Get Order Tests ====================

    #[test]
    fn test_get_order_success() {
        let engine = MatchingEngine::new();

        let request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        let status = engine.get_order(order_id).unwrap();

        assert_eq!(status.order_id, order_id);
        assert_eq!(status.symbol, "AAPL");
        assert_eq!(status.side, Side::Buy);
        assert_eq!(status.order_type, OrderType::Limit);
        assert_eq!(status.price, Some(10000));
        assert_eq!(status.quantity, 100);
        assert_eq!(status.filled_quantity, 0);
        assert_eq!(status.remaining_quantity, 100);
        assert_eq!(status.status, OrderStatus::Accepted);
    }

    #[test]
    fn test_get_order_not_found() {
        let engine = MatchingEngine::new();

        let fake_id = Uuid::new_v4();
        let result = engine.get_order(fake_id);

        match result {
            Err(AppError::OrderNotFound(id)) => assert_eq!(id, fake_id),
            _ => panic!("Expected OrderNotFound error"),
        }
    }

    // ==================== Get Order Book Tests ====================

    #[test]
    fn test_get_order_book() {
        let engine = MatchingEngine::new();

        // Add some orders
        engine.submit_order(create_limit_request(Side::Buy, 9900, 100)).unwrap();
        engine.submit_order(create_limit_request(Side::Buy, 10000, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10100, 75)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10200, 25)).unwrap();

        let book = engine.get_order_book("AAPL", 10);

        assert_eq!(book.symbol, "AAPL");

        // Bids: highest first
        assert_eq!(book.bids.len(), 2);
        assert_eq!(book.bids[0].price, 10000);
        assert_eq!(book.bids[0].quantity, 50);
        assert_eq!(book.bids[1].price, 9900);
        assert_eq!(book.bids[1].quantity, 100);

        // Asks: lowest first
        assert_eq!(book.asks.len(), 2);
        assert_eq!(book.asks[0].price, 10100);
        assert_eq!(book.asks[0].quantity, 75);
        assert_eq!(book.asks[1].price, 10200);
        assert_eq!(book.asks[1].quantity, 25);
    }

    #[test]
    fn test_get_order_book_empty_symbol() {
        let engine = MatchingEngine::new();

        let book = engine.get_order_book("UNKNOWN", 10);

        assert_eq!(book.symbol, "UNKNOWN");
        assert!(book.bids.is_empty());
        assert!(book.asks.is_empty());
    }

    #[test]
    fn test_get_order_book_depth_limit() {
        let engine = MatchingEngine::new();

        // Add 5 price levels
        for i in 0..5 {
            engine.submit_order(create_limit_request(Side::Buy, 10000 - i * 100, 100)).unwrap();
        }

        let book = engine.get_order_book("AAPL", 3);

        // Should only return 3 levels
        assert_eq!(book.bids.len(), 3);
        assert_eq!(book.bids[0].price, 10000);
        assert_eq!(book.bids[1].price, 9900);
        assert_eq!(book.bids[2].price, 9800);
    }

    // ==================== Metrics Tests ====================

    #[test]
    fn test_metrics_orders_received() {
        let engine = MatchingEngine::new();

        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10100, 50)).unwrap();

        assert_eq!(engine.orders_received(), 2);
    }

    #[test]
    fn test_metrics_orders_matched() {
        let engine = MatchingEngine::new();

        // Add sell order
        engine.submit_order(create_limit_request(Side::Sell, 10000, 100)).unwrap();

        // Add matching buy order
        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();

        // Both orders were matched
        assert_eq!(engine.orders_matched(), 2);
    }

    #[test]
    fn test_metrics_orders_cancelled() {
        let engine = MatchingEngine::new();

        let request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(request).unwrap();
        let order_id = match response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        engine.cancel_order(order_id).unwrap();

        assert_eq!(engine.orders_cancelled(), 1);
    }

    #[test]
    fn test_metrics_trades_executed() {
        let engine = MatchingEngine::new();

        // Add sell order
        engine.submit_order(create_limit_request(Side::Sell, 10000, 100)).unwrap();

        // Add matching buy order
        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();

        assert_eq!(engine.trades_executed(), 1);
    }

    #[test]
    fn test_metrics_multiple_trades() {
        let engine = MatchingEngine::new();

        // Add multiple sell orders
        engine.submit_order(create_limit_request(Side::Sell, 10000, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10100, 50)).unwrap();

        // Buy order matches both
        engine.submit_order(create_limit_request(Side::Buy, 10100, 100)).unwrap();

        assert_eq!(engine.trades_executed(), 2);
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_no_match_buy_price_too_low() {
        let engine = MatchingEngine::new();

        // Add sell order at 10100
        engine.submit_order(create_limit_request(Side::Sell, 10100, 100)).unwrap();

        // Add buy order at 10000 (below ask)
        let response = engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();

        match response {
            SubmitOrderResponse::Accepted(_) => {}
            _ => panic!("Expected Accepted response"),
        }

        // Both orders should be in book (no match)
        assert_eq!(engine.orders_in_book(), 2);
        assert_eq!(engine.trades_executed(), 0);
    }

    #[test]
    fn test_no_match_sell_price_too_high() {
        let engine = MatchingEngine::new();

        // Add buy order at 10000
        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();

        // Add sell order at 10100 (above bid)
        let response = engine.submit_order(create_limit_request(Side::Sell, 10100, 100)).unwrap();

        match response {
            SubmitOrderResponse::Accepted(_) => {}
            _ => panic!("Expected Accepted response"),
        }

        // Both orders should be in book (no match)
        assert_eq!(engine.orders_in_book(), 2);
        assert_eq!(engine.trades_executed(), 0);
    }

    #[test]
    fn test_multiple_symbols() {
        let engine = MatchingEngine::new();

        // Add orders for AAPL
        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();

        // Add orders for different symbol
        let btc_request = CreateOrderRequest {
            symbol: "BTC".to_string(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            price: Some(50000),
            quantity: 10,
        };
        engine.submit_order(btc_request).unwrap();

        // Check order books are separate
        let aapl_book = engine.get_order_book("AAPL", 10);
        let btc_book = engine.get_order_book("BTC", 10);

        assert_eq!(aapl_book.bids.len(), 1);
        assert!(aapl_book.asks.is_empty());

        assert!(btc_book.bids.is_empty());
        assert_eq!(btc_book.asks.len(), 1);
    }

    #[test]
    fn test_validation_empty_symbol() {
        let engine = MatchingEngine::new();

        let request = CreateOrderRequest {
            symbol: "".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(10000),
            quantity: 100,
        };

        let result = engine.submit_order(request);

        match result {
            Err(AppError::InvalidOrder(msg)) => {
                assert!(msg.contains("Symbol"));
            }
            _ => panic!("Expected InvalidOrder error"),
        }
    }

    #[test]
    fn test_validation_zero_quantity() {
        let engine = MatchingEngine::new();

        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(10000),
            quantity: 0,
        };

        let result = engine.submit_order(request);

        match result {
            Err(AppError::InvalidOrder(msg)) => {
                assert!(msg.contains("Quantity"));
            }
            _ => panic!("Expected InvalidOrder error"),
        }
    }

    #[test]
    fn test_validation_limit_order_no_price() {
        let engine = MatchingEngine::new();

        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: None,
            quantity: 100,
        };

        let result = engine.submit_order(request);

        match result {
            Err(AppError::InvalidOrder(msg)) => {
                assert!(msg.contains("Price"));
            }
            _ => panic!("Expected InvalidOrder error"),
        }
    }

    #[test]
    fn test_validation_market_order_with_price() {
        let engine = MatchingEngine::new();

        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            price: Some(10000),
            quantity: 100,
        };

        let result = engine.submit_order(request);

        match result {
            Err(AppError::InvalidOrder(msg)) => {
                assert!(msg.contains("Price"));
            }
            _ => panic!("Expected InvalidOrder error"),
        }
    }

    #[test]
    fn test_large_order_multiple_fills() {
        let engine = MatchingEngine::new();

        // Add many small sell orders
        for i in 0..10 {
            engine.submit_order(create_limit_request(Side::Sell, 10000 + i * 10, 10)).unwrap();
        }

        // Large buy order that matches all of them
        let buy_request = create_limit_request(Side::Buy, 10100, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.filled_quantity, 100);
                assert_eq!(r.trades.len(), 10);
            }
            _ => panic!("Expected Filled response"),
        }

        assert_eq!(engine.orders_in_book(), 0);
        assert_eq!(engine.trades_executed(), 10);
    }
    #[test]
    fn test_aggregated_quantity_at_price_level() {
        let engine = MatchingEngine::new();

        // Add multiple orders at same price
        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();
        engine.submit_order(create_limit_request(Side::Buy, 10000, 50)).unwrap();
        engine.submit_order(create_limit_request(Side::Buy, 10000, 75)).unwrap();

        let book = engine.get_order_book("AAPL", 10);

        // Should show aggregated quantity
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids[0].price, 10000);
        assert_eq!(book.bids[0].quantity, 225);
    }

    #[test]
    fn test_market_order_matches_multiple_price_levels() {
        let engine = MatchingEngine::new();

        // Add sell orders at different prices
        engine.submit_order(create_limit_request(Side::Sell, 10000, 30)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10100, 30)).unwrap();
        engine.submit_order(create_limit_request(Side::Sell, 10200, 40)).unwrap();

        // Market buy should fill across all levels
        let buy_request = create_market_request(Side::Buy, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                assert_eq!(r.filled_quantity, 100);
                assert_eq!(r.trades.len(), 3);
                // Should match in price order (lowest first)
                assert_eq!(r.trades[0].price, 10000);
                assert_eq!(r.trades[0].quantity, 30);
                assert_eq!(r.trades[1].price, 10100);
                assert_eq!(r.trades[1].quantity, 30);
                assert_eq!(r.trades[2].price, 10200);
                assert_eq!(r.trades[2].quantity, 40);
            }
            _ => panic!("Expected Filled response"),
        }

        assert_eq!(engine.orders_in_book(), 0);
    }

    #[test]
    fn test_resting_order_status_updated_after_match() {
        let engine = MatchingEngine::new();

        // Add sell order
        let sell_request = create_limit_request(Side::Sell, 10000, 100);
        let sell_response = engine.submit_order(sell_request).unwrap();
        let sell_order_id = match sell_response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Partially fill with buy order
        let buy_request = create_limit_request(Side::Buy, 10000, 40);
        engine.submit_order(buy_request).unwrap();

        // Check sell order status is updated
        let sell_status = engine.get_order(sell_order_id).unwrap();
        assert_eq!(sell_status.status, OrderStatus::PartialFill);
        assert_eq!(sell_status.filled_quantity, 40);
        assert_eq!(sell_status.remaining_quantity, 60);
    }

    #[test]
    fn test_resting_order_fully_filled() {
        let engine = MatchingEngine::new();

        // Add sell order
        let sell_request = create_limit_request(Side::Sell, 10000, 100);
        let sell_response = engine.submit_order(sell_request).unwrap();
        let sell_order_id = match sell_response {
            SubmitOrderResponse::Accepted(r) => r.order_id,
            _ => panic!("Expected Accepted"),
        };

        // Fully fill with buy order
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        engine.submit_order(buy_request).unwrap();

        // Check sell order status is updated
        let sell_status = engine.get_order(sell_order_id).unwrap();
        assert_eq!(sell_status.status, OrderStatus::Filled);
        assert_eq!(sell_status.filled_quantity, 100);
        assert_eq!(sell_status.remaining_quantity, 0);
    }

    #[test]
    fn test_order_book_updates_after_partial_fill() {
        let engine = MatchingEngine::new();

        // Add sell order with 100 qty
        engine.submit_order(create_limit_request(Side::Sell, 10000, 100)).unwrap();

        // Partially fill with 40 qty
        engine.submit_order(create_limit_request(Side::Buy, 10000, 40)).unwrap();

        // Order book should show remaining 60 qty
        let book = engine.get_order_book("AAPL", 10);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.asks[0].quantity, 60);
    }

    #[test]
    fn test_taker_side_in_trade() {
        let engine = MatchingEngine::new();

        // Add sell order (maker)
        engine.submit_order(create_limit_request(Side::Sell, 10000, 100)).unwrap();

        // Buy order is the taker
        let buy_request = create_limit_request(Side::Buy, 10000, 100);
        let response = engine.submit_order(buy_request).unwrap();

        match response {
            SubmitOrderResponse::Filled(r) => {
                // The trade info doesn't include taker_side, but we can verify
                // the trade happened correctly
                assert_eq!(r.trades.len(), 1);
                assert_eq!(r.trades[0].quantity, 100);
            }
            _ => panic!("Expected Filled response"),
        }
    }

    #[test]
    fn test_orders_in_book_count() {
        let engine = MatchingEngine::new();

        assert_eq!(engine.orders_in_book(), 0);

        engine.submit_order(create_limit_request(Side::Buy, 10000, 100)).unwrap();
        assert_eq!(engine.orders_in_book(), 1);

        engine.submit_order(create_limit_request(Side::Sell, 10100, 50)).unwrap();
        assert_eq!(engine.orders_in_book(), 2);

        // Add matching order that fills one
        engine.submit_order(create_limit_request(Side::Buy, 10100, 50)).unwrap();
        assert_eq!(engine.orders_in_book(), 1); // Sell filled, buy still there
    }

    #[test]
    fn test_concurrent_safety() {
        use std::sync::Arc;
        use std::thread;

        let engine = Arc::new(MatchingEngine::new());
        let mut handles = vec![];

        // Spawn multiple threads submitting orders
        for i in 0..10 {
            let engine_clone = Arc::clone(&engine);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    let side = if (i + j) % 2 == 0 { Side::Buy } else { Side::Sell };
                    let price = 10000 + (j % 10) * 10;
                    let request = CreateOrderRequest {
                        symbol: "AAPL".to_string(),
                        side,
                        order_type: OrderType::Limit,
                        price: Some(price),
                        quantity: 10,
                    };
                    let _ = engine_clone.submit_order(request);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify engine is in a consistent state
        assert_eq!(engine.orders_received(), 1000);
        // orders_in_book + matched orders should be consistent
        let in_book = engine.orders_in_book();
        let matched = engine.orders_matched();
        assert!(in_book <= 1000);
        assert!(matched <= 1000);
    }
}
