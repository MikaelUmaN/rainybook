use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use num_enum::{IntoPrimitive, TryFromPrimitive};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error, Clone)]
pub enum OrderBookError {
    #[error("Order {0} not found at price level")]
    OrderNotFound(u64),

    #[error("Attempted to fill {0} units, but only {1} available")]
    FillQuantityExceedsOrderSize(u64, u64),
}

#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromPrimitive, IntoPrimitive)]
pub enum Side {
    Bid = 1,
    Ask = 2,
}

/// A single order in the order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Order {
    pub order_id: u64,
    pub side: Side,
    pub price: i64,
    pub size: u64,
}

/// Price level tracking individual orders (Market-By-Order).
/// Maintains aggregate quantity and individual order quantities.
#[derive(Debug)]
pub struct OrderLevel {
    pub price: i64,
    orders: HashMap<u64, Order>,
}

impl OrderLevel {
    pub fn new(price: i64) -> Self {
        Self {
            price,
            orders: HashMap::new(),
        }
    }

    /// Iterates over orders and sums size.
    pub fn total_qty(&self) -> u64 {
        self.orders.values().map(|o| o.size).sum()
    }

    /// Add order (idempotent - overwrites if exists).
    pub fn add_order(&mut self, order: Order) {
        match self.orders.entry(order.order_id) {
            Entry::Vacant(e) => {
                e.insert(order);
            }
            Entry::Occupied(mut e) => {
                warn!(
                    "Order {} already exists, overwriting size {} -> {}",
                    order.order_id,
                    e.get().size,
                    order.size
                );
                e.insert(order);
            }
        }
    }

    /// Remove order entirely (idempotent - no-op if not found).
    pub fn remove_order(&mut self, order_id: u64) -> Option<Order> {
        match self.orders.entry(order_id) {
            Entry::Vacant(_) => {
                warn!("Order {} not found in level, ignoring removal", order_id);
                None
            }
            Entry::Occupied(e) => Some(e.remove()),
        }
    }

    /// Modify order size (replace old with new).
    pub fn modify_order(&mut self, order_id: u64, new_size: u64) -> Result<(), OrderBookError> {
        match self.orders.entry(order_id) {
            Entry::Vacant(_) => Err(OrderBookError::OrderNotFound(order_id)),
            Entry::Occupied(mut e) => {
                e.get_mut().size = new_size;
                Ok(())
            }
        }
    }

    /// Gets an order by id.
    pub fn get_order(&self, order_id: u64) -> Option<&Order> {
        self.orders.get(&order_id)
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Returns the number of orders at this price level.
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }
}

/// Market-By-Order orderbook tracking individual orders.
/// Prices are integers (cents, ticks, etc.)
#[derive(Debug, Default)]
pub struct OrderBook {
    pub bids: BTreeMap<i64, OrderLevel>,
    pub asks: BTreeMap<i64, OrderLevel>,

    /// Mapping from order_id -> price for fast order lookup.
    /// Side is stored in the Order itself.
    order_index: HashMap<u64, i64>,
}

impl OrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets the side of the book (bids or asks) for the given side.
    fn levels_mut(&mut self, side: Side) -> &mut BTreeMap<i64, OrderLevel> {
        match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        }
    }

    /// Adds an order to the orderbook. If the order id already exists, the old order is replaced,
    /// possibly with changed price and size.
    pub fn add_order(&mut self, order: Order) {
        // If order exists, remove it from old location first (handles price changes)
        if let Some(&old_price) = self.order_index.get(&order.order_id) {
            // Look up the old order to get its side
            let old_side = self
                .bids
                .get(&old_price)
                .and_then(|level| level.get_order(order.order_id))
                .map(|o| o.side)
                .or_else(|| {
                    self.asks
                        .get(&old_price)
                        .and_then(|level| level.get_order(order.order_id))
                        .map(|o| o.side)
                });

            if let Some(old_side) = old_side {
                warn!(
                    "Order {} already exists at {:?} price {}, moving to {:?} price {}",
                    order.order_id, old_side, old_price, order.side, order.price
                );

                let old_levels = self.levels_mut(old_side);
                if let Some(level) = old_levels.get_mut(&old_price) {
                    level.remove_order(order.order_id);
                    if level.is_empty() {
                        old_levels.remove(&old_price);
                    }
                }
            }
        }

        let price = order.price;
        let order_id = order.order_id;
        let levels = self.levels_mut(order.side);

        levels
            .entry(price)
            .or_insert_with(|| OrderLevel::new(price))
            .add_order(order);

        self.order_index.insert(order_id, price);
    }

    /// Removes an order from the order book. If it is not found, no operation is performed.
    /// Returns the removed Order if found, None otherwise.
    pub fn remove_order(&mut self, order_id: u64) -> Option<Order> {
        let Some(price) = self.order_index.remove(&order_id) else {
            warn!("Order {} not found in index, ignoring removal", order_id);
            return None;
        };

        // Try bids first, then asks
        let removed = self
            .bids
            .get_mut(&price)
            .and_then(|level| level.remove_order(order_id))
            .or_else(|| {
                self.asks
                    .get_mut(&price)
                    .and_then(|level| level.remove_order(order_id))
            });

        // Clean up empty levels
        if let Some(ref order) = removed {
            let levels = self.levels_mut(order.side);
            if levels.get(&price).is_some_and(|l| l.is_empty()) {
                levels.remove(&price);
            }
        } else {
            warn!("Price level {} not found for order {}", price, order_id);
        }

        removed
    }

    /// Gets an order by id.
    pub fn get_order(&self, order_id: u64) -> Option<&Order> {
        let price = self.order_index.get(&order_id)?;
        self.bids
            .get(price)
            .and_then(|level| level.get_order(order_id))
            .or_else(|| {
                self.asks
                    .get(price)
                    .and_then(|level| level.get_order(order_id))
            })
    }

    pub fn modify_order(&mut self, order_id: u64, new_size: u64) -> Result<(), OrderBookError> {
        let &price = self
            .order_index
            .get(&order_id)
            .ok_or(OrderBookError::OrderNotFound(order_id))?;

        // Try bids first, then asks
        if let Some(level) = self.bids.get_mut(&price)
            && level.get_order(order_id).is_some()
        {
            return level.modify_order(order_id, new_size);
        }
        if let Some(level) = self.asks.get_mut(&price)
            && level.get_order(order_id).is_some()
        {
            return level.modify_order(order_id, new_size);
        }
        Err(OrderBookError::OrderNotFound(order_id))
    }

    /// Fills part or all of an order. If the fill quantity equals
    /// the order size, the order is removed.
    pub fn fill_order(&mut self, order_id: u64, fill_quantity: u64) -> Result<(), OrderBookError> {
        let current_size = self
            .get_order(order_id)
            .ok_or(OrderBookError::OrderNotFound(order_id))?
            .size;

        if current_size < fill_quantity {
            return Err(OrderBookError::FillQuantityExceedsOrderSize(
                fill_quantity,
                current_size,
            ));
        }

        let new_size = current_size - fill_quantity;
        if new_size == 0 {
            self.remove_order(order_id);
        } else {
            self.modify_order(order_id, new_size)?;
        }
        Ok(())
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

    /// Helper to create an Order for tests.
    fn order(order_id: u64, side: Side, price: i64, size: u64) -> Order {
        Order {
            order_id,
            side,
            price,
            size,
        }
    }

    #[test]
    fn test_add_and_remove_order() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        assert_eq!(book.best_bid(), Some((10050, 100)));

        book.remove_order(123);
        assert_eq!(book.best_bid(), None);
    }

    #[test]
    fn test_add_and_modify_order() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        assert_eq!(book.best_bid(), Some((10050, 100)));

        book.modify_order(123, 150).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 150)));
    }

    #[test]
    fn test_remove_one_of_two_orders() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10051, 50));

        book.remove_order(123);

        // Second order should still exist
        assert_eq!(book.best_bid(), Some((10051, 50)));
        book.remove_order(124);
    }

    #[test]
    fn test_modify_one_of_two_orders() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10051, 50));

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

        book.add_order(order(123, Side::Bid, 10050, 100));
        assert_eq!(book.best_bid(), Some((10050, 100)));

        // Adding same order_id at different price should move it
        book.add_order(order(123, Side::Bid, 10051, 150));
        assert_eq!(book.best_bid(), Some((10051, 150)));

        // Old price level should be empty
        assert_eq!(book.top_n_bids(2), vec![(10051, 150)]);
    }

    #[test]
    fn test_empty_price_level_removed() {
        let mut book = OrderBook::new();

        // Add two orders at same price
        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10050, 50));

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
        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10048, 50));
        book.add_order(order(125, Side::Ask, 10052, 75));
        book.add_order(order(126, Side::Ask, 10054, 80));

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
        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10050, 50));
        book.add_order(order(125, Side::Bid, 10050, 75));

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
        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Ask, 10052, 50));

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
        book.add_order(order(123, Side::Bid, 10050, 100));
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
        book.add_order(order(123, Side::Bid, 10050, 100));
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
        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10050, 50));
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
        book.add_order(order(123, Side::Bid, 10050, 100));

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
        assert!(matches!(
            result.unwrap_err(),
            OrderBookError::OrderNotFound(999)
        ));
    }

    #[test]
    fn test_fill_multiple_sequential() {
        let mut book = OrderBook::new();

        // Add order with 100 units
        book.add_order(order(125, Side::Ask, 10052, 100));
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
        book.add_order(order(123, Side::Bid, 10050, 100));

        // Fill zero units (edge case - should succeed but do nothing)
        book.fill_order(123, 0).unwrap();
        assert_eq!(book.best_bid(), Some((10050, 100)));
    }
}
