use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::book::{OrderBook, OrderLevel};

/// An order level summary gives aggregate information about a price level.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLevelSummary {
    pub price: i64,
    pub total_quantity: u64,
    pub order_count: usize,
}

impl From<&OrderLevel> for OrderLevelSummary {
    fn from(level: &OrderLevel) -> Self {
        Self {
            price: level.price,
            total_quantity: level.total_qty(),
            order_count: level.order_count(),
        }
    }
}

/// Market-By-Price view of the order book.
/// Aggregates each price level into an `OrderLevelSummary`.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct MarketByPrice {
    pub bids: BTreeMap<i64, OrderLevelSummary>,
    pub asks: BTreeMap<i64, OrderLevelSummary>,
}

impl MarketByPrice {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an MBP-N snapshot containing at most `n` levels per side.
    /// Bids are the `n` highest-priced levels; asks are the `n` lowest-priced levels.
    pub fn from_top_n(book: &OrderBook, n: usize) -> Self {
        let bids = book
            .bids
            .iter()
            .rev()
            .take(n)
            .map(|(&price, level)| (price, OrderLevelSummary::from(level)))
            .collect();

        let asks = book
            .asks
            .iter()
            .take(n)
            .map(|(&price, level)| (price, OrderLevelSummary::from(level)))
            .collect();

        Self { bids, asks }
    }

    /// Top-N bid levels, ordered best (highest price) to worst.
    pub fn top_n_bids(&self, n: usize) -> Vec<OrderLevelSummary> {
        self.bids.values().rev().take(n).copied().collect()
    }

    /// Top-N ask levels, ordered best (lowest price) to worst.
    pub fn top_n_asks(&self, n: usize) -> Vec<OrderLevelSummary> {
        self.asks.values().take(n).copied().collect()
    }
}

impl From<&OrderBook> for MarketByPrice {
    fn from(book: &OrderBook) -> Self {
        let bids = book
            .bids
            .iter()
            .map(|(&price, level)| (price, OrderLevelSummary::from(level)))
            .collect();

        let asks = book
            .asks
            .iter()
            .map(|(&price, level)| (price, OrderLevelSummary::from(level)))
            .collect();

        Self { bids, asks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{Order, Side};

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
    fn test_order_level_summary_aggregation() {
        let mut book = OrderBook::new();

        // Add multiple orders at same bid price
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 10000, 200));
        book.add_order(order(3, Side::Bid, 10000, 150));

        // Add multiple orders at same ask price
        book.add_order(order(4, Side::Ask, 10100, 50));
        book.add_order(order(5, Side::Ask, 10100, 75));

        let mbp = MarketByPrice::from(&book);

        // Verify bid level aggregation
        let bid_summary = mbp.bids.get(&10000).expect("Bid level should exist");
        assert_eq!(bid_summary.price, 10000);
        assert_eq!(bid_summary.total_quantity, 450); // 100 + 200 + 150
        assert_eq!(bid_summary.order_count, 3);

        // Verify ask level aggregation
        let ask_summary = mbp.asks.get(&10100).expect("Ask level should exist");
        assert_eq!(ask_summary.price, 10100);
        assert_eq!(ask_summary.total_quantity, 125); // 50 + 75
        assert_eq!(ask_summary.order_count, 2);
    }

    #[test]
    fn test_market_by_price_multiple_levels() {
        let mut book = OrderBook::new();

        // Create 3 bid levels
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 9900, 200));
        book.add_order(order(3, Side::Bid, 9800, 300));

        // Create 3 ask levels
        book.add_order(order(4, Side::Ask, 10100, 50));
        book.add_order(order(5, Side::Ask, 10200, 75));
        book.add_order(order(6, Side::Ask, 10300, 100));

        let mbp = MarketByPrice::from(&book);

        // Verify structure
        assert_eq!(mbp.bids.len(), 3);
        assert_eq!(mbp.asks.len(), 3);

        // Verify bids are ordered (BTreeMap guarantees this)
        let bid_prices: Vec<i64> = mbp.bids.keys().copied().collect();
        assert_eq!(bid_prices, vec![9800, 9900, 10000]);

        // Verify asks are ordered
        let ask_prices: Vec<i64> = mbp.asks.keys().copied().collect();
        assert_eq!(ask_prices, vec![10100, 10200, 10300]);

        // Verify best bid/ask
        assert_eq!(mbp.bids.last_key_value().unwrap().0, &10000);
        assert_eq!(mbp.asks.first_key_value().unwrap().0, &10100);
    }

    #[test]
    fn test_empty_orderbook_to_market_by_price() {
        let book = OrderBook::new();
        let mbp = MarketByPrice::from(&book);

        // Empty book should produce empty MBP
        assert!(mbp.bids.is_empty());
        assert!(mbp.asks.is_empty());
    }

    #[test]
    fn test_orderbook_becomes_empty_after_cancels() {
        let mut book = OrderBook::new();

        // Add orders
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 10000, 200));
        book.add_order(order(3, Side::Ask, 10100, 150));

        // Create MBP before cancellation
        let mbp_before = MarketByPrice::from(&book);
        assert_eq!(mbp_before.bids.len(), 1);
        assert_eq!(mbp_before.asks.len(), 1);

        // Cancel all orders
        book.remove_order(1);
        book.remove_order(2);
        book.remove_order(3);

        // Verify book is empty
        let mbp_after = MarketByPrice::from(&book);
        assert!(
            mbp_after.bids.is_empty(),
            "Bids should be empty after all cancels"
        );
        assert!(
            mbp_after.asks.is_empty(),
            "Asks should be empty after all cancels"
        );
    }

    #[test]
    fn test_orderbook_becomes_empty_after_fills() {
        let mut book = OrderBook::new();

        // Add orders
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Ask, 10100, 200));

        let mbp_before = MarketByPrice::from(&book);
        assert_eq!(mbp_before.bids.get(&10000).unwrap().total_quantity, 100);
        assert_eq!(mbp_before.asks.get(&10100).unwrap().total_quantity, 200);

        // Fully fill both orders
        book.fill_order(1, 100).expect("Fill should succeed");
        book.fill_order(2, 200).expect("Fill should succeed");

        // Verify book is empty
        let mbp_after = MarketByPrice::from(&book);
        assert!(
            mbp_after.bids.is_empty(),
            "Bids should be empty after full fills"
        );
        assert!(
            mbp_after.asks.is_empty(),
            "Asks should be empty after full fills"
        );
    }

    #[test]
    fn test_from_top_n_limits_levels() {
        let mut book = OrderBook::new();

        // Create 5 bid levels
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 9900, 200));
        book.add_order(order(3, Side::Bid, 9800, 300));
        book.add_order(order(4, Side::Bid, 9700, 400));
        book.add_order(order(5, Side::Bid, 9600, 500));

        // Create 5 ask levels
        book.add_order(order(6, Side::Ask, 10100, 50));
        book.add_order(order(7, Side::Ask, 10200, 60));
        book.add_order(order(8, Side::Ask, 10300, 70));
        book.add_order(order(9, Side::Ask, 10400, 80));
        book.add_order(order(10, Side::Ask, 10500, 90));

        // MBP-3 should only have the top 3 levels per side
        let mbp3 = MarketByPrice::from_top_n(&book, 3);
        assert_eq!(mbp3.bids.len(), 3);
        assert_eq!(mbp3.asks.len(), 3);

        // Best 3 bids: 10000, 9900, 9800
        let bid_prices: Vec<i64> = mbp3.bids.keys().copied().collect();
        assert_eq!(bid_prices, vec![9800, 9900, 10000]);

        // Best 3 asks: 10100, 10200, 10300
        let ask_prices: Vec<i64> = mbp3.asks.keys().copied().collect();
        assert_eq!(ask_prices, vec![10100, 10200, 10300]);
    }

    #[test]
    fn test_top_n_bids_ordering() {
        let mut book = OrderBook::new();
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 9900, 200));
        book.add_order(order(3, Side::Bid, 9800, 300));

        let mbp = MarketByPrice::from(&book);
        let top2 = mbp.top_n_bids(2);

        // Best (highest) first
        assert_eq!(top2.len(), 2);
        assert_eq!(top2[0].price, 10000);
        assert_eq!(top2[1].price, 9900);
    }

    #[test]
    fn test_top_n_asks_ordering() {
        let mut book = OrderBook::new();
        book.add_order(order(1, Side::Ask, 10100, 50));
        book.add_order(order(2, Side::Ask, 10200, 60));
        book.add_order(order(3, Side::Ask, 10300, 70));

        let mbp = MarketByPrice::from(&book);
        let top2 = mbp.top_n_asks(2);

        // Best (lowest) first
        assert_eq!(top2.len(), 2);
        assert_eq!(top2[0].price, 10100);
        assert_eq!(top2[1].price, 10200);
    }

    #[test]
    fn test_from_top_n_fewer_levels_than_n() {
        let mut book = OrderBook::new();
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Ask, 10100, 50));

        // Request 10 levels but only 1 exists per side
        let mbp10 = MarketByPrice::from_top_n(&book, 10);
        assert_eq!(mbp10.bids.len(), 1);
        assert_eq!(mbp10.asks.len(), 1);
    }

    #[test]
    fn test_order_level_summary_equality() {
        let a = OrderLevelSummary {
            price: 100,
            total_quantity: 50,
            order_count: 3,
        };
        let b = OrderLevelSummary {
            price: 100,
            total_quantity: 50,
            order_count: 3,
        };
        let c = OrderLevelSummary {
            price: 100,
            total_quantity: 51,
            order_count: 3,
        };

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_partial_fills_update_quantities() {
        let mut book = OrderBook::new();

        // Add orders with multiple orders at same level
        book.add_order(order(1, Side::Bid, 10000, 100));
        book.add_order(order(2, Side::Bid, 10000, 200));
        book.add_order(order(3, Side::Bid, 10000, 150));

        let mbp_before = MarketByPrice::from(&book);
        assert_eq!(mbp_before.bids.get(&10000).unwrap().total_quantity, 450);
        assert_eq!(mbp_before.bids.get(&10000).unwrap().order_count, 3);

        // Partially fill one order
        book.fill_order(1, 50).unwrap();

        let mbp_after_partial = MarketByPrice::from(&book);
        assert_eq!(
            mbp_after_partial.bids.get(&10000).unwrap().total_quantity,
            400
        ); // 450 - 50
        assert_eq!(mbp_after_partial.bids.get(&10000).unwrap().order_count, 3); // Still 3 orders

        // Fully fill the partially filled order
        book.fill_order(1, 50).unwrap();

        let mbp_after_full = MarketByPrice::from(&book);
        assert_eq!(mbp_after_full.bids.get(&10000).unwrap().total_quantity, 350); // 400 - 50
        assert_eq!(mbp_after_full.bids.get(&10000).unwrap().order_count, 2); // Now 2 orders
    }
}
