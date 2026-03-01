use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use num_enum::{IntoPrimitive, TryFromPrimitive};
use thiserror::Error;
use tracing::warn;

/// Information returned by `OrderBook::add_order`.
#[derive(Debug, Clone, Copy)]
pub struct AddOrderInfo {
    /// The order that was added.
    pub order: Order,
    /// Total quantity at this price level after the add.
    pub level_qty: u64,
    /// Number of orders at this price level after the add.
    pub level_order_count: usize,
    /// True if this order created a new price level.
    pub new_level: bool,
}

/// Information returned by `OrderBook::remove_order`.
#[derive(Debug, Clone, Copy)]
pub struct RemoveOrderInfo {
    /// The order that was removed.
    pub order: Order,
    /// Quantity remaining at this price level after removal (0 if level removed).
    pub remaining_level_qty: u64,
    /// Orders remaining at this price level after removal (0 if level removed).
    pub remaining_level_count: usize,
    /// True if the price level was removed (no more orders at this price).
    pub level_removed: bool,
}

/// Information returned by `OrderBook::update_order_size`.
#[derive(Debug, Clone, Copy)]
pub struct UpdateSizeInfo {
    /// The order after the size update.
    pub order: Order,
    /// Total quantity at this price level after the update.
    pub level_qty: u64,
    /// Number of orders at this price level after the update.
    pub level_order_count: usize,
    /// Retained 0-indexed queue position (unchanged by a size-only update).
    pub queue_position: usize,
}

/// Information returned by `OrderBook::modify_order`.
#[derive(Debug, Clone, Copy)]
pub struct ModifyOrderInfo {
    /// The order after modification.
    pub order: Order,
    /// Price before modification.
    pub old_price: i64,
    /// Size before modification.
    pub old_size: u64,
    /// Total quantity at the (new) price level after modification.
    pub level_qty: u64,
    /// Number of orders at the (new) price level after modification.
    pub level_order_count: usize,
    /// True if the order kept its queue position (same price + size decrease).
    /// False if queue position was reset (price change or size increase).
    pub retained_queue_position: bool,
}

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
    /// Exchange sequence at which this order holds its current queue position.
    ///
    /// For a size-decrease modify, this stays as the original Add's sequence.
    /// For a size-increase or price change, it becomes the Modify's sequence.
    pub sequence: u32,
}

/// Price level tracking individual orders (Market-By-Order).
///
/// Orders are maintained in price-time (FIFO) order using the exchange sequence number
/// as the BTreeMap key. A lower sequence means an earlier (better) queue position.
#[derive(Debug)]
pub struct OrderLevel {
    pub price: i64,
    /// BTreeMap from (sequence, order_id) → Order. The composite key ensures uniqueness
    /// even when multiple orders share the same sequence (e.g. SNAPSHOT-phase Add records).
    /// Natural iteration order equals queue order (sequence-primary, order_id tiebreaker).
    queue: BTreeMap<(u32, u64), Order>,
    /// Index from order_id → sequence for O(1) lookup.
    order_index: HashMap<u64, u32>,
}

impl OrderLevel {
    pub fn new(price: i64) -> Self {
        Self {
            price,
            queue: BTreeMap::new(),
            order_index: HashMap::new(),
        }
    }

    /// Iterates over orders and sums size.
    pub fn total_qty(&self) -> u64 {
        self.queue.values().map(|o| o.size).sum()
    }

    /// Add order. If an order with the same `order_id` already exists it is removed first
    /// (idempotent overwrite with a warning).
    pub fn add_order(&mut self, order: Order) {
        let key = (order.sequence, order.order_id);
        // Remove any existing entry for this order_id before inserting
        if let Some(old_seq) = self.order_index.get(&order.order_id).copied() {
            warn!(
                "Order {} already exists at sequence {}, overwriting at sequence {}",
                order.order_id, old_seq, order.sequence
            );
            self.queue.remove(&(old_seq, order.order_id));
        }
        self.order_index.insert(order.order_id, order.sequence);
        self.queue.insert(key, order);
    }

    /// Remove order entirely (idempotent — no-op with warning if not found).
    pub fn remove_order(&mut self, order_id: u64) -> Option<Order> {
        let seq = match self.order_index.entry(order_id) {
            Entry::Vacant(_) => {
                warn!("Order {} not found in level, ignoring removal", order_id);
                return None;
            }
            Entry::Occupied(e) => e.remove(),
        };
        self.queue.remove(&(seq, order_id))
    }

    /// Update order size in place without changing queue position.
    pub fn update_size_in_place(
        &mut self,
        order_id: u64,
        new_size: u64,
    ) -> Result<(), OrderBookError> {
        let seq = self
            .order_index
            .get(&order_id)
            .copied()
            .ok_or(OrderBookError::OrderNotFound(order_id))?;
        let order = self
            .queue
            .get_mut(&(seq, order_id))
            .ok_or(OrderBookError::OrderNotFound(order_id))?;
        order.size = new_size;
        Ok(())
    }

    /// Gets an order by id.
    pub fn get_order(&self, order_id: u64) -> Option<&Order> {
        let seq = *self.order_index.get(&order_id)?;
        self.queue.get(&(seq, order_id))
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns the number of orders at this price level.
    pub fn order_count(&self) -> usize {
        self.queue.len()
    }

    /// Returns the 0-indexed queue position of the given order (0 = first/best). O(n).
    /// Returns `None` if the order is not in this level.
    pub fn queue_position(&self, order_id: u64) -> Option<usize> {
        let seq = *self.order_index.get(&order_id)?;
        let key = (seq, order_id);
        Some(self.queue.range(..key).count())
    }

    /// Returns the total quantity of all orders ahead of this order in the queue. O(n).
    /// Returns `None` if the order is not in this level.
    pub fn queue_depth_ahead(&self, order_id: u64) -> Option<u64> {
        let seq = *self.order_index.get(&order_id)?;
        let key = (seq, order_id);
        Some(self.queue.range(..key).map(|(_, o)| o.size).sum())
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
    ///
    /// Returns information about the added order and its price level.
    pub fn add_order(&mut self, order: Order) -> AddOrderInfo {
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
        let side = order.side;
        let order_id = order.order_id;
        let order_copy = order;
        let levels = self.levels_mut(side);

        levels
            .entry(price)
            .or_insert_with(|| OrderLevel::new(price))
            .add_order(order);

        self.order_index.insert(order_id, price);

        // Read level info after insertion (mutable borrow has ended)
        let levels = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        let level = levels.get(&price).expect("level must exist after add");
        AddOrderInfo {
            order: order_copy,
            level_qty: level.total_qty(),
            level_order_count: level.order_count(),
            new_level: level.order_count() == 1,
        }
    }

    /// Removes an order from the order book. If it is not found, no operation is performed.
    /// Returns information about the removed order and the remaining level state.
    pub fn remove_order(&mut self, order_id: u64) -> Option<RemoveOrderInfo> {
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

        // Capture level info and clean up empty levels
        if let Some(order) = removed {
            let levels = self.levels_mut(order.side);
            let (remaining_qty, remaining_count, level_removed) = match levels.get(&price) {
                Some(level) if level.is_empty() => {
                    levels.remove(&price);
                    (0, 0, true)
                }
                Some(level) => (level.total_qty(), level.order_count(), false),
                None => (0, 0, true),
            };

            Some(RemoveOrderInfo {
                order,
                remaining_level_qty: remaining_qty,
                remaining_level_count: remaining_count,
                level_removed,
            })
        } else {
            warn!("Price level {} not found for order {}", price, order_id);
            None
        }
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

    /// Updates an order's size in place, preserving its queue position.
    /// Returns `None` if the order is not found.
    pub fn update_order_size(&mut self, order_id: u64, new_size: u64) -> Option<UpdateSizeInfo> {
        let &price = self.order_index.get(&order_id)?;

        if let Some(level) = self.bids.get_mut(&price)
            && level.get_order(order_id).is_some()
        {
            level.update_size_in_place(order_id, new_size).ok()?;
            let queue_position = level.queue_position(order_id).unwrap_or(0);
            let order = *level.get_order(order_id)?;
            return Some(UpdateSizeInfo {
                order,
                level_qty: level.total_qty(),
                level_order_count: level.order_count(),
                queue_position,
            });
        }
        if let Some(level) = self.asks.get_mut(&price)
            && level.get_order(order_id).is_some()
        {
            level.update_size_in_place(order_id, new_size).ok()?;
            let queue_position = level.queue_position(order_id).unwrap_or(0);
            let order = *level.get_order(order_id)?;
            return Some(UpdateSizeInfo {
                order,
                level_qty: level.total_qty(),
                level_order_count: level.order_count(),
                queue_position,
            });
        }
        None
    }

    /// Modifies an order's price and/or size.
    ///
    /// **Queue-position policy**: if the price is unchanged and the new size is
    /// less than or equal to the old size, the order retains its queue position
    /// (in-place size update). Otherwise, the order is removed and re-added at
    /// the new price/size, losing its queue position.
    ///
    /// Returns `None` if the order is not found in the book.
    pub fn modify_order(&mut self, new_order: Order) -> Option<ModifyOrderInfo> {
        let old = self.get_order(new_order.order_id).copied()?;
        let old_price = old.price;
        let old_size = old.size;

        let retained = old_price == new_order.price && new_order.size <= old_size;

        let (order, level_qty, level_order_count) = if retained {
            let info = self
                .update_order_size(new_order.order_id, new_order.size)
                .expect("order must exist after get_order succeeded");
            (info.order, info.level_qty, info.level_order_count)
        } else {
            self.remove_order(new_order.order_id);
            let info = self.add_order(new_order);
            (info.order, info.level_qty, info.level_order_count)
        };

        Some(ModifyOrderInfo {
            order,
            old_price,
            old_size,
            level_qty,
            level_order_count,
            retained_queue_position: retained,
        })
    }

    /// Fills part or all of an order. If the fill quantity equals the order size, the order
    /// is removed. Partial fills preserve queue position.
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
            self.update_order_size(order_id, new_size)
                .ok_or(OrderBookError::OrderNotFound(order_id))?;
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

    /// Returns the 0-indexed queue position of the order within its price level (0 = best). O(n).
    /// Returns `None` if the order is not in the book.
    pub fn queue_position(&self, order_id: u64) -> Option<usize> {
        let price = self.order_index.get(&order_id)?;
        self.bids
            .get(price)
            .and_then(|l| l.queue_position(order_id))
            .or_else(|| {
                self.asks
                    .get(price)
                    .and_then(|l| l.queue_position(order_id))
            })
    }

    /// Returns the total quantity of all orders ahead of this order in its price-level queue. O(n).
    /// Returns `None` if the order is not in the book.
    pub fn queue_depth_ahead(&self, order_id: u64) -> Option<u64> {
        let price = self.order_index.get(&order_id)?;
        self.bids
            .get(price)
            .and_then(|l| l.queue_depth_ahead(order_id))
            .or_else(|| {
                self.asks
                    .get(price)
                    .and_then(|l| l.queue_depth_ahead(order_id))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create an Order for tests.
    /// Uses `order_id as u32` for the sequence so each order gets a distinct,
    /// monotonically-increasing queue key.
    fn order(order_id: u64, side: Side, price: i64, size: u64) -> Order {
        Order {
            order_id,
            side,
            price,
            size,
            sequence: order_id as u32,
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
    fn test_add_and_update_order_size() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        assert_eq!(book.best_bid(), Some((10050, 100)));

        book.update_order_size(123, 150).unwrap();
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
    fn test_update_size_of_one_of_two_orders() {
        let mut book = OrderBook::new();

        book.add_order(order(123, Side::Bid, 10050, 100));
        book.add_order(order(124, Side::Bid, 10051, 50));

        book.update_order_size(123, 200).unwrap();

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

        // Update size of one order
        book.update_order_size(124, 150).unwrap();
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

        // Update bid size shouldn't affect ask
        book.update_order_size(123, 200).unwrap();
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

    // --- Queue position tests ---

    #[test]
    fn test_queue_position_sequential_adds() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(2), Some(1));
        assert_eq!(level.queue_position(3), Some(2));
        assert_eq!(level.queue_position(99), None); // not in level
    }

    #[test]
    fn test_queue_depth_ahead() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_depth_ahead(1), Some(0)); // nothing ahead
        assert_eq!(level.queue_depth_ahead(2), Some(100)); // order 1 ahead
        assert_eq!(level.queue_depth_ahead(3), Some(150)); // orders 1+2 ahead
        assert_eq!(level.queue_depth_ahead(99), None); // not in level
    }

    #[test]
    fn test_update_order_size_retains_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        // Size decrease on order 2 (position 1) should retain its queue position
        let info = book.update_order_size(2, 30).unwrap();
        assert_eq!(info.queue_position, 1);
        assert_eq!(info.order.size, 30);

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(2), Some(1)); // still position 1
        assert_eq!(level.queue_position(3), Some(2));
    }

    #[test]
    fn test_remove_and_readd_loses_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        // Simulate size-increase: remove and re-add order 2 with a higher sequence
        book.remove_order(2);
        book.add_order(Order {
            order_id: 2,
            side: Side::Bid,
            price: 10050,
            size: 80,
            sequence: 100, // higher than 3 → end of queue
        });

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(3), Some(1)); // moved forward
        assert_eq!(level.queue_position(2), Some(2)); // now at back
    }

    #[test]
    fn test_fill_partial_retains_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        // Partial fill on order 2 should retain its queue position
        book.fill_order(2, 20).unwrap();

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(2), Some(1)); // retained
        assert_eq!(level.get_order(2).unwrap().size, 30); // size reduced
    }

    #[test]
    fn test_queue_position_independent_across_price_levels() {
        let mut book = OrderBook::new();

        // Orders at different prices have independent queue positions
        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10051, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        let level_10050 = book.bids.get(&10050).unwrap();
        let level_10051 = book.bids.get(&10051).unwrap();

        assert_eq!(level_10050.queue_position(1), Some(0));
        assert_eq!(level_10050.queue_position(3), Some(1));
        assert_eq!(level_10051.queue_position(2), Some(0));
    }

    // --- modify_order tests ---

    #[test]
    fn test_modify_order_size_decrease_retains_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        // Size decrease at same price → retains queue position
        let info = book
            .modify_order(order(2, Side::Bid, 10050, 30))
            .unwrap();
        assert!(info.retained_queue_position);
        assert_eq!(info.old_price, 10050);
        assert_eq!(info.old_size, 50);
        assert_eq!(info.order.size, 30);

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(2), Some(1)); // still position 1
        assert_eq!(level.queue_position(3), Some(2));
    }

    #[test]
    fn test_modify_order_size_increase_resets_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));
        book.add_order(order(3, Side::Bid, 10050, 75));

        // Size increase at same price → loses queue position (re-added with higher sequence)
        let new = Order {
            order_id: 2,
            side: Side::Bid,
            price: 10050,
            size: 80,
            sequence: 100,
        };
        let info = book.modify_order(new).unwrap();
        assert!(!info.retained_queue_position);
        assert_eq!(info.old_price, 10050);
        assert_eq!(info.old_size, 50);
        assert_eq!(info.order.size, 80);

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(3), Some(1)); // moved forward
        assert_eq!(level.queue_position(2), Some(2)); // now at back
    }

    #[test]
    fn test_modify_order_price_change_resets_queue_position() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));

        // Price change → loses queue position, moves to new level
        let new = Order {
            order_id: 2,
            side: Side::Bid,
            price: 10051,
            size: 50,
            sequence: 100,
        };
        let info = book.modify_order(new).unwrap();
        assert!(!info.retained_queue_position);
        assert_eq!(info.old_price, 10050);
        assert_eq!(info.order.price, 10051);

        // Old level has only order 1
        assert_eq!(book.bids.get(&10050).unwrap().order_count(), 1);
        // New level has the moved order
        assert_eq!(book.bids.get(&10051).unwrap().order_count(), 1);
        assert_eq!(book.best_bid(), Some((10051, 50)));
    }

    #[test]
    fn test_modify_order_nonexistent_returns_none() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));

        let result = book.modify_order(order(99, Side::Bid, 10050, 50));
        assert!(result.is_none());

        // Book unchanged
        assert_eq!(book.best_bid(), Some((10050, 100)));
    }

    #[test]
    fn test_modify_order_same_price_same_size_retains() {
        let mut book = OrderBook::new();

        book.add_order(order(1, Side::Bid, 10050, 100));
        book.add_order(order(2, Side::Bid, 10050, 50));

        // Same price, same size → retains (new_size <= old_size is true when equal)
        let info = book
            .modify_order(order(2, Side::Bid, 10050, 50))
            .unwrap();
        assert!(info.retained_queue_position);
        assert_eq!(info.old_size, 50);
        assert_eq!(info.order.size, 50);

        let level = book.bids.get(&10050).unwrap();
        assert_eq!(level.queue_position(1), Some(0));
        assert_eq!(level.queue_position(2), Some(1));
    }
}
