//! Order generation utilities for testing and benchmarking.
//!
//! Provides randomized order generation with configurable distributions
//! for price and quantity, while ensuring the order book never crosses.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Exp, Normal};

use rainybook::orderbook::{Order, Side};

/// Stateful order generator that tracks market state to prevent crossed books.
///
/// Maintains `max_bid` and `min_ask` to ensure generated orders
/// never cross the spread (bid >= ask).
pub struct OrderGenerator<P, Q, R> {
    rng: R,
    price_dist: P,
    qty_dist: Q,
    next_order_id: u64,
    bid_probability: f64,

    /// Highest bid price seen so far.
    max_bid: Option<i64>,
    /// Lowest ask price seen so far.
    min_ask: Option<i64>,
}

impl<P, Q, R> OrderGenerator<P, Q, R>
where
    P: Distribution<f64>,
    Q: Distribution<f64>,
    R: Rng,
{
    /// Create a new order generator.
    ///
    /// # Arguments
    /// - `rng`: Random number generator
    /// - `price_dist`: Distribution for sampling raw prices
    /// - `qty_dist`: Distribution for sampling quantities
    /// - `bid_probability`: Probability [0, 1] that an order is a bid
    pub fn new(rng: R, price_dist: P, qty_dist: Q, bid_probability: f64) -> Self {
        Self {
            rng,
            price_dist,
            qty_dist,
            next_order_id: 1,
            bid_probability: bid_probability.clamp(0.0, 1.0),
            max_bid: None,
            min_ask: None,
        }
    }

    /// Sample a side using the configured bid probability.
    fn sample_side(&mut self) -> Side {
        if self.rng.random_bool(self.bid_probability) {
            Side::Bid
        } else {
            Side::Ask
        }
    }

    /// Sample a price from the distribution, clamped to prevent crossing.
    ///
    /// - Bids are clamped to be strictly less than `min_ask` (if any).
    /// - Asks are clamped to be strictly greater than `max_bid` (if any).
    fn sample_price(&mut self, side: Side) -> i64 {
        let raw_price = self.price_dist.sample(&mut self.rng).round() as i64;

        match side {
            Side::Bid => self
                .min_ask
                .map_or(raw_price, |min_ask| raw_price.min(min_ask - 1)),
            Side::Ask => self
                .max_bid
                .map_or(raw_price, |max_bid| raw_price.max(max_bid + 1)),
        }
    }

    /// Sample a quantity from the distribution.
    fn sample_qty(&mut self) -> u64 {
        (self.qty_dist.sample(&mut self.rng).round().abs() as u64).max(1)
    }

    /// Generate the next order and update market state.
    pub fn next_order(&mut self) -> Order {
        let side = self.sample_side();
        let price = self.sample_price(side);
        let size = self.sample_qty();
        let order_id = self.next_order_id;
        self.next_order_id += 1;

        // Update market state
        match side {
            Side::Bid => {
                self.max_bid = Some(self.max_bid.map_or(price, |b| b.max(price)));
            }
            Side::Ask => {
                self.min_ask = Some(self.min_ask.map_or(price, |a| a.min(price)));
            }
        }

        Order {
            order_id,
            side,
            price,
            size,
        }
    }

    /// Generate `n` orders, updating market state after each.
    pub fn make_orders(&mut self, n: usize) -> Vec<Order> {
        (0..n).map(|_| self.next_order()).collect()
    }
}

/// Convenience constructor for testing with sensible defaults.
impl OrderGenerator<Normal<f64>, Exp<f64>, ChaCha8Rng> {
    /// Create a generator with default distributions:
    /// - Price: Normal(10000, 100) — centered at 10000 with std dev 100
    /// - Quantity: Exponential(0.1) — mean of 10
    /// - 50% bid probability
    ///
    /// Uses a seeded RNG for reproducibility.
    pub fn default_seeded(seed: u64) -> Self {
        let rng = ChaCha8Rng::seed_from_u64(seed);
        let price_dist = Normal::new(10000.0, 100.0).expect("valid normal distribution");
        let qty_dist = Exp::new(0.1).expect("valid exponential distribution");

        Self::new(rng, price_dist, qty_dist, 0.5)
    }
}
