use std::collections::VecDeque;
use uuid::Uuid;

use crate::models::order::Order;

/// Represents a single price level in the order book.
/// Orders at the same price are stored in FIFO order for time priority.
#[derive(Debug, Clone)]
pub struct PriceLevel {
    pub price: u64,
    /// Orders stored in FIFO order (front = oldest, back = newest)
    pub orders: VecDeque<Order>,
    /// Total quantity available at this price level
    total_quantity: u64,
}

impl PriceLevel {
    pub fn new(price: u64) -> Self {
        Self {
            price,
            orders: VecDeque::new(),
            total_quantity: 0,
        }
    }

    /// Adds an order to the back of the queue (newest)
    pub fn add_order(&mut self, order: Order) {
        self.total_quantity += order.remaining_quantity();
        self.orders.push_back(order);
    }

    /// Removes an order by ID. Returns the removed order if found.
    pub fn remove_order(&mut self, order_id: Uuid) -> Option<Order> {
        if let Some(pos) = self.orders.iter().position(|o| o.id == order_id) {
            let order = self.orders.remove(pos)?;
            self.total_quantity = self.total_quantity.saturating_sub(order.remaining_quantity());
            Some(order)
        } else {
            None
        }
    }

    /// Returns a reference to the front order (oldest, highest priority)
    pub fn front(&self) -> Option<&Order> {
        self.orders.front()
    }

    /// Returns a mutable reference to the front order
    pub fn front_mut(&mut self) -> Option<&mut Order> {
        self.orders.front_mut()
    }

    /// Removes and returns the front order
    pub fn pop_front(&mut self) -> Option<Order> {
        if let Some(order) = self.orders.pop_front() {
            self.total_quantity = self.total_quantity.saturating_sub(order.remaining_quantity());
            Some(order)
        } else {
            None
        }
    }

    /// Call this after modifying the front order's filled_quantity.
    pub fn reduce_total_quantity(&mut self, amount: u64) {
        self.total_quantity = self.total_quantity.saturating_sub(amount);
    }

    /// Returns true if there are no orders at this price level
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Returns the number of orders at this price level
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    /// Returns the total quantity available at this price level
    pub fn total_quantity(&self) -> u64 {
        self.total_quantity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::order::{CreateOrderRequest, OrderType, Side};

    fn create_test_order(price: u64, quantity: u64) -> Order {
        let request = CreateOrderRequest {
            symbol: "AAPL".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(price),
            quantity,
        };
        Order::new(request)
    }

    #[test]
    fn test_new_price_level() {
        let level = PriceLevel::new(10000);
        assert_eq!(level.price, 10000);
        assert!(level.is_empty());
        assert_eq!(level.total_quantity(), 0);
        assert_eq!(level.order_count(), 0);
    }

    #[test]
    fn test_add_order() {
        let mut level = PriceLevel::new(10000);
        let order = create_test_order(10000, 100);

        level.add_order(order);

        assert!(!level.is_empty());
        assert_eq!(level.total_quantity(), 100);
        assert_eq!(level.order_count(), 1);
    }

    #[test]
    fn test_add_multiple_orders() {
        let mut level = PriceLevel::new(10000);

        level.add_order(create_test_order(10000, 100));
        level.add_order(create_test_order(10000, 50));
        level.add_order(create_test_order(10000, 75));

        assert_eq!(level.total_quantity(), 225);
        assert_eq!(level.order_count(), 3);
    }

    #[test]
    fn test_fifo_order() {
        let mut level = PriceLevel::new(10000);

        let order1 = create_test_order(10000, 100);
        let order2 = create_test_order(10000, 50);
        let order1_id = order1.id;
        let order2_id = order2.id;

        level.add_order(order1);
        level.add_order(order2);

        // First order should be at front
        assert_eq!(level.front().unwrap().id, order1_id);

        // Pop should return first order
        let popped = level.pop_front().unwrap();
        assert_eq!(popped.id, order1_id);

        // Now second order should be at front
        assert_eq!(level.front().unwrap().id, order2_id);
        assert_eq!(level.total_quantity(), 50);
    }

    #[test]
    fn test_remove_order_by_id() {
        let mut level = PriceLevel::new(10000);

        let order1 = create_test_order(10000, 100);
        let order2 = create_test_order(10000, 50);
        let order3 = create_test_order(10000, 75);
        let order2_id = order2.id;

        level.add_order(order1);
        level.add_order(order2);
        level.add_order(order3);

        // Remove middle order
        let removed = level.remove_order(order2_id);

        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, order2_id);
        assert_eq!(level.total_quantity(), 175); // 100 + 75
        assert_eq!(level.order_count(), 2);
    }

    #[test]
    fn test_remove_nonexistent_order() {
        let mut level = PriceLevel::new(10000);
        level.add_order(create_test_order(10000, 100));

        let removed = level.remove_order(Uuid::new_v4());

        assert!(removed.is_none());
        assert_eq!(level.total_quantity(), 100);
    }

    
    #[test]
    fn test_front_mut() {
        let mut level = PriceLevel::new(10000);
        level.add_order(create_test_order(10000, 100));

        // Modify front order
        if let Some(order) = level.front_mut() {
            order.fill(30);
        }

        assert_eq!(level.front().unwrap().filled_quantity, 30);
        assert_eq!(level.front().unwrap().remaining_quantity(), 70);
    }
}