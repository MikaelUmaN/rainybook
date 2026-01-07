use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error, Clone)]
pub enum OrderBookError {
    #[error("Order {0} not found at price level")]
    OrderNotFound(u64),

    #[error("Attempted to fill {0} units, but only {1} available")]
    FillQuantityExceedsOrderSize(u64, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Bid,
    Ask,
}

/// Price level tracking individual orders (Market-By-Order).
/// Maintains aggregate quantity and individual order quantities.
#[derive(Debug, Default)]
pub struct OrderLevel {
    orders: HashMap<u64, u64>,
}

impl OrderLevel {
    pub fn new() -> Self {
        Self { orders: HashMap::new() }
    }

    /// Iterates over orders and sums size.
    pub fn total_qty(&self) -> u64 {
        self.orders.values().sum()
    }

    /// Add order (idempotent - overwrites if exists).
    pub fn add_order(&mut self, order_id: u64, quantity: u64) {
        match self.orders.entry(order_id) {
            Entry::Vacant(e) => {
                e.insert(quantity);
            }
            Entry::Occupied(mut e) => {
                warn!(
                    "Order {} already exists, overwriting quantity {} -> {}",
                    order_id,
                    e.get(),
                    quantity
                );
                e.insert(quantity);
            }
        }
    }

    /// Remove order entirely (idempotent - no-op if not found).
    pub fn remove_order(&mut self, order_id: u64) {
        match self.orders.entry(order_id) {
            Entry::Vacant(_) => {
                warn!("Order {} not found in level, ignoring removal", order_id);
            }
            Entry::Occupied(e) => {
                e.remove();
            }
        }
    }

    /// Modify order quantity (replace old with new).
    pub fn modify_order(&mut self, order_id: u64, new_quantity: u64) -> Result<(), OrderBookError> {
        match self.orders.entry(order_id) {
            Entry::Vacant(_) => Err(OrderBookError::OrderNotFound(order_id)),
            Entry::Occupied(mut e) => {
                e.insert(new_quantity);
                Ok(())
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    pub fn order_count(&self) -> usize {
        self.orders.len()
    }
}

/// Market-By-Order orderbook tracking individual orders.
/// Prices are integers (cents, ticks, etc.)
#[derive(Debug, Default)]
pub struct OrderBook {
    bids: BTreeMap<i64, OrderLevel>,
    asks: BTreeMap<i64, OrderLevel>,

    /// Mapping from order_id -> (side, price)
    order_index: HashMap<u64, (Side, i64)>,
}

impl OrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an order to the orderbook. If the order id already exists, the old order is replaced,
    /// possibly with changed price and quantity.
    pub fn add_order(&mut self, side: Side, price: i64, order_id: u64, quantity: u64) {
        // If order exists, remove it from old location first (handles price changes)
        if let Some((old_side, old_price)) = self.order_index.get(&order_id) {
            warn!(
                "Order {} already exists at {:?} price {}, moving to {:?} price {}",
                order_id, old_side, old_price, side, price
            );

            let old_levels = match old_side {
                Side::Bid => &mut self.bids,
                Side::Ask => &mut self.asks,
            };

            if let Some(level) = old_levels.get_mut(old_price) {
                level.remove_order(order_id);
                if level.is_empty() {
                    old_levels.remove(old_price);
                }
            }
        }

        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };

        levels
            .entry(price)
            .or_default()
            .add_order(order_id, quantity);

        self.order_index.insert(order_id, (side, price));
    }

    /// Removes an order from the order book. If it is not found,
    /// no operation is performed.
    pub fn remove_order(&mut self, order_id: u64) {
        let Some((side, price)) = self.order_index.remove(&order_id) else {
            warn!("Order {} not found in index, ignoring removal", order_id);
            return;
        };

        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };

        if let Some(level) = levels.get_mut(&price) {
            level.remove_order(order_id);

            // Remove price level if empty
            if level.is_empty() {
                levels.remove(&price);
            }
        } else {
            warn!("Price level {} not found for order {}", price, order_id);
        }
    }

    pub fn modify_order(&mut self, order_id: u64, new_quantity: u64) -> Result<(), OrderBookError> {
        let (side, price) = self
            .order_index
            .get(&order_id)
            .ok_or(OrderBookError::OrderNotFound(order_id))?;

        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };

        if let Some(level) = levels.get_mut(price) {
            level.modify_order(order_id, new_quantity)
        } else {
            Err(OrderBookError::OrderNotFound(order_id))
        }
    }

    /// Fills part or all of an order. If the fill quantity equals
    /// the order quantity, the order is removed.
    pub fn fill_order(&mut self, order_id: u64, fill_quantity: u64) -> Result<(), OrderBookError> {
        let (side, price) = self
            .order_index
            .get(&order_id)
            .ok_or(OrderBookError::OrderNotFound(order_id))?;

        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };

        if let Some(level) = levels.get_mut(price) {
            let current_qty = level
                .orders
                .get(&order_id)
                .ok_or(OrderBookError::OrderNotFound(order_id))?;

            if *current_qty < fill_quantity {
                return Err(OrderBookError::FillQuantityExceedsOrderSize(
                    fill_quantity,
                    *current_qty,
                ));
            }

            let new_quantity = *current_qty - fill_quantity;
            if new_quantity == 0 {
                self.remove_order(order_id);
            } else {
                level.modify_order(order_id, new_quantity)?;
            }
            Ok(())
        } else {
            Err(OrderBookError::OrderNotFound(order_id))
        }
    }

    pub fn best_bid(&self) -> Option<(i64, u64)> {
        self.bids
            .iter()
            .next_back()
            .map(|(&price, level)| (price, level.total_qty()))
    }

    pub fn best_ask(&self) -> Option<(i64, u64)> {
        self.asks
            .iter()
            .next()
            .map(|(&price, level)| (price, level.total_qty()))
    }

    pub fn top_n_bids(&self, n: usize) -> Vec<(i64, u64)> {
        self.bids
            .iter()
            .rev() // Highest to lowest
            .take(n)
            .map(|(&price, level)| (price, level.total_qty()))
            .collect()
    }

    pub fn top_n_asks(&self, n: usize) -> Vec<(i64, u64)> {
        self.asks
            .iter()
            .take(n)
            .map(|(&price, level)| (price, level.total_qty()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_remove_order() {
        let mut book = OrderBook::new();

        book.add_order(Side::Bid, 10050, 123, 100);
        assert_eq!(book.best_bid(), Some((10050, 100)));

        book.remove_order(123);
        assert_eq!(book.best_bid(), None);
    }

    #[test]
    fn test_add_and_modify_order() {
        let mut book = OrderBook::new();

        book.add_order(Side::Bid, 10050, 123, 100);
        assert_eq!(book.best_bid(), Some((10050, 100)));

        book.modify_order(123, 150).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 150)));
    }

    #[test]
    fn test_remove_one_of_two_orders() {
        let mut book = OrderBook::new();

        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10051, 124, 50);

        book.remove_order(123);

        // Second order should still exist
        assert_eq!(book.best_bid(), Some((10051, 50)));
        book.remove_order(124);
    }

    #[test]
    fn test_modify_one_of_two_orders() {
        let mut book = OrderBook::new();

        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10051, 124, 50);

        book.modify_order(123, 200).unwrap();

        // First order modified, second unchanged
        assert_eq!(book.top_n_bids(2), vec![(10051, 50), (10050, 200)]);
    }

    #[test]
    fn test_remove_nonexistent_order_is_noop() {
        let mut book = OrderBook::new();

        // Should not panic or error, just no-op with warning
        book.remove_order(999);

        // Book should still be empty
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), None);
    }

    #[test]
    fn test_add_duplicate_order_id_overwrites() {
        let mut book = OrderBook::new();

        book.add_order(Side::Bid, 10050, 123, 100);
        assert_eq!(book.best_bid(), Some((10050, 100)));

        // Adding same order_id at different price should move it
        book.add_order(Side::Bid, 10051, 123, 150);
        assert_eq!(book.best_bid(), Some((10051, 150)));

        // Old price level should be empty
        assert_eq!(book.top_n_bids(2), vec![(10051, 150)]);
    }

    #[test]
    fn test_empty_price_level_removed() {
        let mut book = OrderBook::new();

        // Add two orders at same price
        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10050, 124, 50);

        assert_eq!(book.best_bid(), Some((10050, 150))); // Total: 100 + 50

        // Remove first order
        book.remove_order(123);
        assert_eq!(book.best_bid(), Some((10050, 50))); // Only second order remains

        // Remove last order at this price
        book.remove_order(124);
        assert_eq!(book.best_bid(), None); // Price level should be gone
    }

    #[test]
    fn test_best_bid_ask_tracking() {
        let mut book = OrderBook::new();

        // Add orders at different prices
        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10048, 124, 50);
        book.add_order(Side::Ask, 10052, 125, 75);
        book.add_order(Side::Ask, 10054, 126, 80);

        // Best bid should be highest price
        assert_eq!(book.best_bid(), Some((10050, 100)));
        // Best ask should be lowest price
        assert_eq!(book.best_ask(), Some((10052, 75)));

        // Remove best bid
        book.remove_order(123);
        assert_eq!(book.best_bid(), Some((10048, 50)));

        // Remove best ask
        book.remove_order(125);
        assert_eq!(book.best_ask(), Some((10054, 80)));
    }

    #[test]
    fn test_multiple_orders_at_same_price() {
        let mut book = OrderBook::new();

        // Add three orders at same price
        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10050, 124, 50);
        book.add_order(Side::Bid, 10050, 125, 75);

        // Total quantity should be sum of all orders
        assert_eq!(book.best_bid(), Some((10050, 225)));

        // Modify one order
        book.modify_order(124, 150).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 325))); // 100 + 150 + 75

        // Remove one order
        book.remove_order(123);
        assert_eq!(book.best_bid(), Some((10050, 225))); // 150 + 75
    }

    #[test]
    fn test_bid_ask_independence() {
        let mut book = OrderBook::new();

        // Add orders to both sides
        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Ask, 10052, 124, 50);

        // Modify bid shouldn't affect ask
        book.modify_order(123, 200).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 200)));
        assert_eq!(book.best_ask(), Some((10052, 50)));

        // Remove bid shouldn't affect ask
        book.remove_order(123);
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), Some((10052, 50)));

        // Ask side still intact
        book.remove_order(124);
        assert_eq!(book.best_ask(), None);
    }

    #[test]
    fn test_fill_partial() {
        let mut book = OrderBook::new();

        // Add order with 100 units
        book.add_order(Side::Bid, 10050, 123, 100);
        assert_eq!(book.best_bid(), Some((10050, 100)));

        // Fill 40 units
        book.fill_order(123, 40).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 60)));

        // Fill another 30 units
        book.fill_order(123, 30).unwrap();
        // -> 30 units remain.
        assert_eq!(book.best_bid(), Some((10050, 30)));
    }

    #[test]
    fn test_fill_complete() {
        let mut book = OrderBook::new();

        // Add order with 100 units
        book.add_order(Side::Bid, 10050, 123, 100);
        assert_eq!(book.best_bid(), Some((10050, 100)));

        // Fill entire order
        book.fill_order(123, 100).unwrap();

        // Order and price level should be gone
        assert_eq!(book.best_bid(), None);
    }

    #[test]
    fn test_fill_complete_with_other_orders() {
        let mut book = OrderBook::new();

        // Add two orders at same price
        book.add_order(Side::Bid, 10050, 123, 100);
        book.add_order(Side::Bid, 10050, 124, 50);
        assert_eq!(book.best_bid(), Some((10050, 150)));

        // Fill first order completely
        book.fill_order(123, 100).unwrap();

        // Second order should remain, price level still exists
        assert_eq!(book.best_bid(), Some((10050, 50)));
    }

    #[test]
    fn test_fill_exceeds_quantity() {
        let mut book = OrderBook::new();

        // Add order with 100 units
        book.add_order(Side::Bid, 10050, 123, 100);

        // Try to fill 150 units (more than available)
        let result = book.fill_order(123, 150);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OrderBookError::FillQuantityExceedsOrderSize(150, 100)
        ));

        // Original order should be unchanged
        assert_eq!(book.best_bid(), Some((10050, 100)));
    }

    #[test]
    fn test_fill_nonexistent_order() {
        let mut book = OrderBook::new();

        // Try to fill order that doesn't exist
        let result = book.fill_order(999, 50);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OrderBookError::OrderNotFound(999)));
    }

    #[test]
    fn test_fill_multiple_sequential() {
        let mut book = OrderBook::new();

        // Add order with 100 units
        book.add_order(Side::Ask, 10052, 125, 100);
        assert_eq!(book.best_ask(), Some((10052, 100)));

        // Fill in multiple steps
        book.fill_order(125, 25).unwrap();
        assert_eq!(book.best_ask(), Some((10052, 75)));

        book.fill_order(125, 25).unwrap();
        assert_eq!(book.best_ask(), Some((10052, 50)));

        book.fill_order(125, 25).unwrap();
        assert_eq!(book.best_ask(), Some((10052, 25)));

        // Final fill removes the order
        book.fill_order(125, 25).unwrap();
        assert_eq!(book.best_ask(), None);
    }

    #[test]
    fn test_fill_zero_quantity() {
        let mut book = OrderBook::new();

        // Add order
        book.add_order(Side::Bid, 10050, 123, 100);

        // Fill zero units (edge case - should succeed but do nothing)
        book.fill_order(123, 0).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 100)));
    }
}
