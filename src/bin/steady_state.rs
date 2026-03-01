//! Steady-state order book simulator for performance profiling.
//!
//! This binary simulates order book activity by maintaining a steady state
//! of approximately N levels of bid/ask depth. It dynamically adjusts operation
//! probabilities to prevent the book from growing unbounded or collapsing.
//!
//! Intended for CPU profiling with Linux perf and flamegraph generation.

use clap::Parser;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::time::Instant;
use tracing::{error, info};

use rainybook::generators::OrderGenerator;
use rainybook::orderbook::{Order, OrderBook, Side};

#[derive(Parser, Debug)]
#[command(name = "steady_state")]
#[command(about = "Steady-state order book simulator for performance profiling")]
struct Cli {
    /// Duration to run simulation in seconds
    #[arg(long, default_value = "30")]
    duration: u64,

    /// Target depth per side (number of price levels)
    #[arg(long, default_value = "12")]
    target_depth: usize,

    /// Random seed for deterministic execution
    #[arg(long, default_value = "42")]
    seed: u64,

    /// Base probability of add operation [0.0-1.0]
    #[arg(long, default_value = "0.45")]
    prob_add: f64,

    /// Base probability of cancel operation [0.0-1.0]
    #[arg(long, default_value = "0.35")]
    prob_cancel: f64,

    /// Base probability of fill operation [0.0-1.0]
    #[arg(long, default_value = "0.15")]
    prob_fill: f64,

    /// Base probability of modify operation [0.0-1.0]
    #[arg(long, default_value = "0.05")]
    prob_modify: f64,

    /// Report progress every N operations
    #[arg(long, default_value = "100000")]
    report_interval: u64,
}

/// Action probabilities for operation selection.
#[derive(Debug, Clone, Copy)]
struct ActionProbabilities {
    add: f64,
    cancel: f64,
    fill: f64,
    modify: f64,
}

impl ActionProbabilities {
    fn new(add: f64, cancel: f64, fill: f64, modify: f64) -> Self {
        Self {
            add,
            cancel,
            fill,
            modify,
        }
    }

    /// Normalize probabilities to sum to 1.0.
    ///
    /// Assumes total > 0.0 (validated at CLI parsing).
    fn normalize(&self) -> Self {
        let total = self.add + self.cancel + self.fill + self.modify;
        Self::new(
            self.add / total,
            self.cancel / total,
            self.fill / total,
            self.modify / total,
        )
    }
}

/// Order book state tracking.
#[derive(Default)]
struct BookState {
    bid_orders: Vec<u64>,
    ask_orders: Vec<u64>,
}

impl BookState {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bid_orders: Vec::with_capacity(capacity),
            ask_orders: Vec::with_capacity(capacity),
        }
    }

    fn bid_depth(&self) -> usize {
        self.bid_orders.len()
    }

    fn ask_depth(&self) -> usize {
        self.ask_orders.len()
    }

    fn total_orders(&self) -> usize {
        self.bid_orders.len() + self.ask_orders.len()
    }

    fn add_order(&mut self, order: &Order) {
        match order.side {
            Side::Bid => self.bid_orders.push(order.order_id),
            Side::Ask => self.ask_orders.push(order.order_id),
        }
    }

    fn remove_order(&mut self, order_id: u64, side: Side) -> bool {
        let orders = match side {
            Side::Bid => &mut self.bid_orders,
            Side::Ask => &mut self.ask_orders,
        };

        if let Some(pos) = orders.iter().position(|&id| id == order_id) {
            orders.swap_remove(pos);
            true
        } else {
            false
        }
    }

    fn random_order(&self, side: Side, rng: &mut impl Rng) -> Option<u64> {
        let orders = match side {
            Side::Bid => &self.bid_orders,
            Side::Ask => &self.ask_orders,
        };

        if orders.is_empty() {
            None
        } else {
            let idx = rng.random_range(0..orders.len());
            Some(orders[idx])
        }
    }
}

/// Actions that can be performed on the order book.
#[derive(Debug, Clone, Copy)]
enum Action {
    Add,
    Cancel,
    Fill,
    Modify,
}

/// Select an action based on current depth and base probabilities.
///
/// Dynamically adjusts probabilities to maintain steady state:
/// - If depth < 70% of target: boost add, reduce cancel/fill
/// - If depth > 130% of target: reduce add, boost cancel/fill
/// - Otherwise: use base probabilities
fn select_action(
    bid_depth: usize,
    ask_depth: usize,
    target_depth: usize,
    base_probs: &ActionProbabilities,
    rng: &mut ChaCha8Rng,
) -> Action {
    let avg_depth = (bid_depth + ask_depth) as f64 / 2.0;
    let depth_ratio = avg_depth / target_depth as f64;

    // Adjust probabilities based on depth
    let adjusted = if depth_ratio < 0.7 {
        // Book too shallow - encourage adds
        ActionProbabilities::new(
            base_probs.add * 1.5,
            base_probs.cancel * 0.5,
            base_probs.fill * 0.5,
            base_probs.modify,
        )
    } else if depth_ratio > 1.3 {
        // Book too deep - encourage cancels/fills
        ActionProbabilities::new(
            base_probs.add * 0.5,
            base_probs.cancel * 1.5,
            base_probs.fill * 1.5,
            base_probs.modify,
        )
    } else {
        // Within range - use base probabilities
        *base_probs
    };

    // Normalize and sample
    let normalized = adjusted.normalize();
    let rand_val = rng.random::<f64>();

    let mut cumulative = 0.0;
    cumulative += normalized.add;
    if rand_val < cumulative {
        return Action::Add;
    }
    cumulative += normalized.cancel;
    if rand_val < cumulative {
        return Action::Cancel;
    }
    cumulative += normalized.fill;
    if rand_val < cumulative {
        return Action::Fill;
    }
    Action::Modify
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    // Validate probabilities
    if cli.prob_add < 0.0 || cli.prob_cancel < 0.0 || cli.prob_fill < 0.0 || cli.prob_modify < 0.0 {
        error!("All probabilities must be non-negative");
        std::process::exit(1);
    }
    let prob_sum = cli.prob_add + cli.prob_cancel + cli.prob_fill + cli.prob_modify;
    if prob_sum == 0.0 {
        error!("Sum of probabilities must be > 0");
        std::process::exit(1);
    }

    info!("Steady-State Order Book Simulator");
    info!("==================================");
    info!("Duration:        {}s", cli.duration);
    info!("Target depth:    {} levels per side", cli.target_depth);
    info!("Seed:            {}", cli.seed);
    info!(
        "Probabilities:   add={:.2} cancel={:.2} fill={:.2} modify={:.2}",
        cli.prob_add, cli.prob_cancel, cli.prob_fill, cli.prob_modify
    );
    info!("");

    // Initialize components
    let mut book = OrderBook::new();
    let mut generator = OrderGenerator::default_seeded(cli.seed);
    let mut rng = ChaCha8Rng::seed_from_u64(cli.seed.wrapping_add(1));
    let mut state = BookState::with_capacity(cli.target_depth * 4);

    let base_probs = ActionProbabilities::new(
        cli.prob_add,
        cli.prob_cancel,
        cli.prob_fill,
        cli.prob_modify,
    );

    // Phase 1: Initialize book to target depth
    info!("Phase 1: Building initial depth...");
    let init_count = cli.target_depth * 2;
    for _ in 0..init_count {
        let order = generator.next_order();
        book.add_order(order);
        state.add_order(&order);
    }
    info!(
        "Initialized: {} orders ({} bid, {} ask)",
        state.total_orders(),
        state.bid_depth(),
        state.ask_depth()
    );
    info!("");

    // Phase 2: Steady-state simulation
    info!("Phase 2: Running steady-state simulation...");
    let start_time = Instant::now();
    let duration_secs = cli.duration;

    let mut i: u64 = 0;
    while start_time.elapsed().as_secs() < duration_secs {
        i += 1;
        // Select action based on current state
        let action = select_action(
            state.bid_depth(),
            state.ask_depth(),
            cli.target_depth,
            &base_probs,
            &mut rng,
        );

        // Execute action
        match action {
            Action::Add => {
                let order = generator.next_order();
                book.add_order(order);
                state.add_order(&order);
            }

            Action::Cancel => {
                // Select random order to cancel
                let side = if rng.random_bool(0.5) {
                    Side::Bid
                } else {
                    Side::Ask
                };

                if let Some(order_id) = state.random_order(side, &mut rng) {
                    let result = book.remove_order(order_id);

                    if result.is_some() {
                        state.remove_order(order_id, side);
                    }
                } else {
                    // No orders to cancel - fall back to add
                    let order = generator.next_order();
                    book.add_order(order);
                    state.add_order(&order);
                }
            }

            Action::Fill => {
                // Select random order to fill
                let side = if rng.random_bool(0.5) {
                    Side::Bid
                } else {
                    Side::Ask
                };

                if let Some(order_id) = state.random_order(side, &mut rng) {
                    // Get current order size to determine fill quantity
                    let order_size = book.get_order(order_id).map(|o| o.size);

                    if let Some(size) = order_size {
                        let fill_qty = if size > 1 {
                            rng.random_range(1..=size)
                        } else {
                            1
                        };

                        let result = book.fill_order(order_id, fill_qty);

                        // If fully filled, remove from tracking
                        if result.is_ok() && fill_qty == size {
                            state.remove_order(order_id, side);
                        }
                    } else {
                        // Order not found - fall back to add
                        let order = generator.next_order();
                        book.add_order(order);
                        state.add_order(&order);
                    }
                } else {
                    // No orders to fill - fall back to add
                    let order = generator.next_order();
                    book.add_order(order);
                    state.add_order(&order);
                }
            }

            Action::Modify => {
                // Select random order to modify
                let side = if rng.random_bool(0.5) {
                    Side::Bid
                } else {
                    Side::Ask
                };

                if let Some(order_id) = state.random_order(side, &mut rng) {
                    // Generate new size
                    let new_size = rng.random_range(1..=100);

                    let result = book.update_order_size(order_id, new_size);

                    if result.is_none() {
                        // Order not found - fall back to add
                        let order = generator.next_order();
                        book.add_order(order);
                        state.add_order(&order);
                    }
                } else {
                    // No orders to modify - fall back to add
                    let order = generator.next_order();
                    book.add_order(order);
                    state.add_order(&order);
                }
            }
        }

        // Periodic reporting
        if (i + 1).is_multiple_of(cli.report_interval) {
            let elapsed = start_time.elapsed();
            let ops_per_sec = (i + 1) as f64 / elapsed.as_secs_f64();
            info!(
                "Progress: {:>10} ops | {:>5} bid depth | {:>5} ask depth | {:>8.0} ops/sec",
                i + 1,
                state.bid_depth(),
                state.ask_depth(),
                ops_per_sec
            );
        }
    }

    // Final report
    let total_time = start_time.elapsed();
    let ops_per_sec = i as f64 / total_time.as_secs_f64();

    info!("");
    info!("{}", "=".repeat(60));
    info!("Simulation Complete");
    info!("{}", "=".repeat(60));
    info!("Total time:      {:?}", total_time);
    info!("Operations:      {}", i);
    info!("Operations/sec:  {:.0}", ops_per_sec);
    info!("Final bid depth: {}", state.bid_depth());
    info!("Final ask depth: {}", state.ask_depth());
    info!("Total orders:    {}", state.total_orders());

    if let Some((bid_price, _)) = book.best_bid() {
        info!("Best bid:        {}", bid_price);
    }
    if let Some((ask_price, _)) = book.best_ask() {
        info!("Best ask:        {}", ask_price);
    }
}
