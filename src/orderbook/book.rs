use std::collections::{ BTreeMap, HashMap };
use uuid::Uuid;

use crate::models::order::{ Order, Side };
use crate::models::orderbook::{ OrderBookResponse, PriceLevelInfo };
use crate::orderbook::price_level::PriceLevel;

/// Order book for a single symbol.
///
/// Uses BTreeMap for price levels to maintain sorted order:
/// - Bids: Highest price first (reverse iteration)
/// - Asks: Lowest price first (forward iteration)
///
/// Uses HashMap for O(1) order lookup by ID.
#[derive(Debug)]
pub struct OrderBook {
    pub symbol: String,
    /// Buy orders: price -> PriceLevel (will iterate highest first)
    bids: BTreeMap<u64, PriceLevel>,
    /// Sell orders: price -> PriceLevel (will iterate lowest first)
    asks: BTreeMap<u64, PriceLevel>,
    /// Fast lookup: order_id -> (side, price)
    order_locations: HashMap<Uuid, (Side, u64)>,
}

impl OrderBook {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_locations: HashMap::new(),
        }
    }
    /// Returns a reference to the bids BTreeMap
    pub fn bids(&self) -> &BTreeMap<u64, PriceLevel> {
        &self.bids
    }

    /// Returns a mutable reference to the bids BTreeMap
    pub fn bids_mut(&mut self) -> &mut BTreeMap<u64, PriceLevel> {
        &mut self.bids
    }

    /// Returns a reference to the asks BTreeMap
    pub fn asks(&self) -> &BTreeMap<u64, PriceLevel> {
        &self.asks
    }

    /// Returns a mutable reference to the asks BTreeMap
    pub fn asks_mut(&mut self) -> &mut BTreeMap<u64, PriceLevel> {
        &mut self.asks
    }
    /// Adds a limit order to the book
    pub fn add_order(&mut self, order: Order) {
        let price = order.price.expect("Order must have a price to be added to book");
        let side = order.side;
        let order_id = order.id;

        // Track order location for fast cancel
        self.order_locations.insert(order_id, (side, price));

        // Add to appropriate side
        let book_side = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        book_side
            .entry(price)
            .or_insert_with(|| PriceLevel::new(price))
            .add_order(order);
    }

    /// Removes an order by ID. Returns the removed order if found.
    pub fn remove_order(&mut self, order_id: Uuid) -> Option<Order> {
        let (side, price) = self.order_locations.remove(&order_id)?;

        let book_side = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        let level = book_side.get_mut(&price)?;
        let order = level.remove_order(order_id)?;

        // Clean up empty price level
        if level.is_empty() {
            book_side.remove(&price);
        }

        Some(order)
    }

    /// Returns the best bid price (highest buy price)
    pub fn best_bid(&self) -> Option<u64> {
        self.bids.keys().next_back().copied()
    }

    /// Returns the best ask price (lowest sell price)
    pub fn best_ask(&self) -> Option<u64> {
        self.asks.keys().next().copied()
    }

    /// Returns a reference to the best bid price level
    pub fn best_bid_level(&self) -> Option<&PriceLevel> {
        self.bids.values().next_back()
    }

    /// Returns a mutable reference to the best bid price level
    pub fn best_bid_level_mut(&mut self) -> Option<&mut PriceLevel> {
        self.bids.values_mut().next_back()
    }

    /// Returns a reference to the best ask price level
    pub fn best_ask_level(&self) -> Option<&PriceLevel> {
        self.asks.values().next()
    }

    /// Returns a mutable reference to the best ask price level
    pub fn best_ask_level_mut(&mut self) -> Option<&mut PriceLevel> {
        self.asks.values_mut().next()
    }

    /// Removes the best bid price level if empty
    pub fn remove_best_bid_if_empty(&mut self) {
        if let Some(price) = self.best_bid() {
            if let Some(level) = self.bids.get(&price) {
                if level.is_empty() {
                    self.bids.remove(&price);
                }
            }
        }
    }

    /// Removes the best ask price level if empty
    pub fn remove_best_ask_if_empty(&mut self) {
        if let Some(price) = self.best_ask() {
            if let Some(level) = self.asks.get(&price) {
                if level.is_empty() {
                    self.asks.remove(&price);
                }
            }
        }
    }

    /// Removes order from location tracking (call when order is fully filled)
    pub fn remove_order_location(&mut self, order_id: Uuid) {
        self.order_locations.remove(&order_id);
    }

    /// Returns total quantity available at the best bid
    pub fn best_bid_quantity(&self) -> u64 {
        self.best_bid_level()
            .map(|l| l.total_quantity())
            .unwrap_or(0)
    }

    /// Returns total quantity available at the best ask
    pub fn best_ask_quantity(&self) -> u64 {
        self.best_ask_level()
            .map(|l| l.total_quantity())
            .unwrap_or(0)
    }

    /// Returns total quantity available on the bid side
    pub fn total_bid_quantity(&self) -> u64 {
        self.bids
            .values()
            .map(|l| l.total_quantity())
            .sum()
    }

    /// Returns total quantity available on the ask side
    pub fn total_ask_quantity(&self) -> u64 {
        self.asks
            .values()
            .map(|l| l.total_quantity())
            .sum()
    }

    /// Returns total number of orders in the book
    pub fn total_order_count(&self) -> usize {
        self.order_locations.len()
    }

    /// Returns number of bid price levels
    pub fn bid_level_count(&self) -> usize {
        self.bids.len()
    }

    /// Returns number of ask price levels
    pub fn ask_level_count(&self) -> usize {
        self.asks.len()
    }

    /// Checks if an order exists in the book
    pub fn contains_order(&self, order_id: Uuid) -> bool {
        self.order_locations.contains_key(&order_id)
    }

    /// Gets order location (side, price) by ID
    pub fn get_order_location(&self, order_id: Uuid) -> Option<(Side, u64)> {
        self.order_locations.get(&order_id).copied()
    }

    /// Creates an OrderBookResponse with the specified depth
    pub fn to_response(&self, depth: usize) -> OrderBookResponse {
        // Bids: highest price first (reverse order)
        let bids: Vec<PriceLevelInfo> = self.bids
            .iter()
            .rev()
            .take(depth)
            .map(|(price, level)| PriceLevelInfo {
                price: *price,
                quantity: level.total_quantity(),
            })
            .collect();

        // Asks: lowest price first (forward order)
        let asks: Vec<PriceLevelInfo> = self.asks
            .iter()
            .take(depth)
            .map(|(price, level)| PriceLevelInfo {
                price: *price,
                quantity: level.total_quantity(),
            })
            .collect();

        OrderBookResponse::new(self.symbol.clone(), bids, asks)
    }

    /// Returns an iterator over ask price levels (lowest first) for matching
    pub fn ask_levels_mut(&mut self) -> impl Iterator<Item = (&u64, &mut PriceLevel)> {
        self.asks.iter_mut()
    }

    /// Returns an iterator over bid price levels (highest first) for matching
    pub fn bid_levels_mut(&mut self) -> impl Iterator<Item = (&u64, &mut PriceLevel)> {
        self.bids.iter_mut().rev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::order::{ CreateOrderRequest, OrderType };

    fn create_limit_order(side: Side, price: u64, quantity: u64) -> Order {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
        };
        Order::new(request)
    }

    #[test]
    fn test_new_order_book() {
        let book = OrderBook::new("AAPL".to_string());

        assert_eq!(book.symbol, "AAPL");
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert_eq!(book.total_order_count(), 0);
    }

    #[test]
    fn test_add_bid_order() {
        let mut book = OrderBook::new("AAPL".to_string());
        let order = create_limit_order(Side::Buy, 10000, 100);
        let order_id = order.id;

        book.add_order(order);

        assert_eq!(book.best_bid(), Some(10000));
        assert!(book.best_ask().is_none());
        assert_eq!(book.total_order_count(), 1);
        assert!(book.contains_order(order_id));
    }

    #[test]
    fn test_add_ask_order() {
        let mut book = OrderBook::new("AAPL".to_string());
        let order = create_limit_order(Side::Sell, 10100, 50);
        let order_id = order.id;

        book.add_order(order);

        assert!(book.best_bid().is_none());
        assert_eq!(book.best_ask(), Some(10100));
        assert_eq!(book.total_order_count(), 1);
        assert!(book.contains_order(order_id));
    }

    #[test]
    fn test_best_bid_is_highest() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Buy, 10000, 100));
        book.add_order(create_limit_order(Side::Buy, 10050, 50));
        book.add_order(create_limit_order(Side::Buy, 9950, 75));

        assert_eq!(book.best_bid(), Some(10050));
    }

    #[test]
    fn test_best_ask_is_lowest() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Sell, 10100, 100));
        book.add_order(create_limit_order(Side::Sell, 10050, 50));
        book.add_order(create_limit_order(Side::Sell, 10150, 75));

        assert_eq!(book.best_ask(), Some(10050));
    }

    #[test]
    fn test_remove_order() {
        let mut book = OrderBook::new("AAPL".to_string());
        let order = create_limit_order(Side::Buy, 10000, 100);
        let order_id = order.id;

        book.add_order(order);
        assert!(book.contains_order(order_id));

        let removed = book.remove_order(order_id);

        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, order_id);
        assert!(!book.contains_order(order_id));
        assert!(book.best_bid().is_none());
    }

    #[test]
    fn test_remove_nonexistent_order() {
        let mut book = OrderBook::new("AAPL".to_string());
        book.add_order(create_limit_order(Side::Buy, 10000, 100));

        let removed = book.remove_order(Uuid::new_v4());

        assert!(removed.is_none());
        assert_eq!(book.total_order_count(), 1);
    }

    #[test]
    fn test_price_level_cleanup() {
        let mut book = OrderBook::new("AAPL".to_string());
        let order = create_limit_order(Side::Buy, 10000, 100);
        let order_id = order.id;

        book.add_order(order);
        assert_eq!(book.bid_level_count(), 1);

        book.remove_order(order_id);
        assert_eq!(book.bid_level_count(), 0);
    }

    #[test]
    fn test_multiple_orders_same_price() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Buy, 10000, 100));
        book.add_order(create_limit_order(Side::Buy, 10000, 50));
        book.add_order(create_limit_order(Side::Buy, 10000, 75));

        assert_eq!(book.bid_level_count(), 1);
        assert_eq!(book.total_order_count(), 3);
        assert_eq!(book.best_bid_quantity(), 225);
    }

    #[test]
    fn test_total_quantities() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Buy, 10000, 100));
        book.add_order(create_limit_order(Side::Buy, 9900, 50));
        book.add_order(create_limit_order(Side::Sell, 10100, 75));
        book.add_order(create_limit_order(Side::Sell, 10200, 25));

        assert_eq!(book.total_bid_quantity(), 150);
        assert_eq!(book.total_ask_quantity(), 100);
    }

    #[test]
    fn test_to_response() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Buy, 10000, 100));
        book.add_order(create_limit_order(Side::Buy, 10050, 50));
        book.add_order(create_limit_order(Side::Buy, 9950, 75));

        book.add_order(create_limit_order(Side::Sell, 10100, 80));
        book.add_order(create_limit_order(Side::Sell, 10150, 60));
        book.add_order(create_limit_order(Side::Sell, 10200, 40));

        let response = book.to_response(2);

        assert_eq!(response.symbol, "AAPL");

        // Bids: highest first
        assert_eq!(response.bids.len(), 2);
        assert_eq!(response.bids[0].price, 10050);
        assert_eq!(response.bids[0].quantity, 50);
        assert_eq!(response.bids[1].price, 10000);
        assert_eq!(response.bids[1].quantity, 100);

        // Asks: lowest first
        assert_eq!(response.asks.len(), 2);
        assert_eq!(response.asks[0].price, 10100);
        assert_eq!(response.asks[0].quantity, 80);
        assert_eq!(response.asks[1].price, 10150);
        assert_eq!(response.asks[1].quantity, 60);
    }

    #[test]
    fn test_to_response_depth_limit() {
        let mut book = OrderBook::new("AAPL".to_string());

        for i in 0..5 {
            book.add_order(create_limit_order(Side::Buy, 10000 - i * 10, 100));
            book.add_order(create_limit_order(Side::Sell, 10100 + i * 10, 100));
        }

        let response = book.to_response(3);

        assert_eq!(response.bids.len(), 3);
        assert_eq!(response.asks.len(), 3);
    }

    #[test]
    fn test_get_order_location() {
        let mut book = OrderBook::new("AAPL".to_string());
        let order = create_limit_order(Side::Buy, 10000, 100);
        let order_id = order.id;

        book.add_order(order);

        let location = book.get_order_location(order_id);
        assert!(location.is_some());

        let (side, price) = location.unwrap();
        assert_eq!(side, Side::Buy);
        assert_eq!(price, 10000);
    }

    #[test]
    fn test_best_level_mut() {
        let mut book = OrderBook::new("AAPL".to_string());

        book.add_order(create_limit_order(Side::Buy, 10000, 100));
        book.add_order(create_limit_order(Side::Sell, 10100, 50));

        // Test mutable access to best bid
        if let Some(level) = book.best_bid_level_mut() {
            assert_eq!(level.price, 10000);
        } else {
            panic!("Expected best bid level");
        }

        // Test mutable access to best ask
        if let Some(level) = book.best_ask_level_mut() {
            assert_eq!(level.price, 10100);
        } else {
            panic!("Expected best ask level");
        }
    }
}
