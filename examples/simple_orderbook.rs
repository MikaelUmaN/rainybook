//! Simple orderbook example demonstrating basic order book operations.
//!
//! This example creates an order book, populates it with sample orders,
//! and prints a market-by-price view to stdout.
//!
//! Run with: `cargo run --example simple_orderbook`

use rainybook::{MarketByPrice, Order, OrderBook, Side};

fn main() {
    // Create a new order book
    let mut book = OrderBook::new();

    // Add some bid orders at various price levels
    // Prices are in integer ticks (e.g., cents)
    book.add_order(Order {
        order_id: 1,
        side: Side::Bid,
        price: 10050,
        size: 100,
    }); // Order 1: 100 units @ 100.50
    book.add_order(Order {
        order_id: 2,
        side: Side::Bid,
        price: 10050,
        size: 250,
    }); // Order 2: 250 units @ 100.50
    book.add_order(Order {
        order_id: 3,
        side: Side::Bid,
        price: 10045,
        size: 500,
    }); // Order 3: 500 units @ 100.45
    book.add_order(Order {
        order_id: 4,
        side: Side::Bid,
        price: 10040,
        size: 300,
    }); // Order 4: 300 units @ 100.40
    book.add_order(Order {
        order_id: 5,
        side: Side::Bid,
        price: 10040,
        size: 150,
    }); // Order 5: 150 units @ 100.40

    // Add some ask orders at various price levels
    book.add_order(Order {
        order_id: 6,
        side: Side::Ask,
        price: 10055,
        size: 200,
    }); // Order 6: 200 units @ 100.55
    book.add_order(Order {
        order_id: 7,
        side: Side::Ask,
        price: 10055,
        size: 100,
    }); // Order 7: 100 units @ 100.55
    book.add_order(Order {
        order_id: 8,
        side: Side::Ask,
        price: 10060,
        size: 400,
    }); // Order 8: 400 units @ 100.60
    book.add_order(Order {
        order_id: 9,
        side: Side::Ask,
        price: 10065,
        size: 600,
    }); // Order 9: 600 units @ 100.65

    // Get best bid and ask
    println!("=== Order Book Summary ===\n");

    if let Some((price, qty)) = book.best_bid() {
        println!("Best Bid: {} @ {} (total qty)", format_price(price), qty);
    }
    if let Some((price, qty)) = book.best_ask() {
        println!("Best Ask: {} @ {} (total qty)", format_price(price), qty);
    }

    // Calculate spread
    if let (Some((bid, _)), Some((ask, _))) = (book.best_bid(), book.best_ask()) {
        let spread = ask - bid;
        println!("Spread:   {} ticks", spread);
    }

    println!();

    // Create a Market-By-Price view
    let mbp = MarketByPrice::from(&book);

    // Print the order book view
    println!("=== Market-By-Price View ===\n");
    print_orderbook_view(&mbp);

    // Demonstrate some order operations
    println!("\n=== Order Operations ===\n");

    // Partially fill an order
    println!("Filling 50 units from order 1...");
    book.fill_order(1, 50).expect("fill should succeed");

    // Modify an order
    println!("Modifying order 3 quantity to 750...");
    book.modify_order(3, 750).expect("modify should succeed");

    // Remove an order
    println!("Removing order 7...");
    book.remove_order(7);

    // Show updated view
    let mbp_updated = MarketByPrice::from(&book);
    println!("\n=== Updated Market-By-Price View ===\n");
    print_orderbook_view(&mbp_updated);
}

/// Formats an integer price as a decimal string (assuming 2 decimal places).
fn format_price(price: i64) -> String {
    format!("{:.2}", price as f64 / 100.0)
}

/// Prints a formatted order book view showing bids and asks.
fn print_orderbook_view(mbp: &MarketByPrice) {
    // Collect and sort asks (ascending for display, but we'll reverse for top-of-book first)
    let mut asks: Vec<_> = mbp.asks.values().collect();
    asks.sort_by(|a, b| b.price.cmp(&a.price)); // Descending (highest ask first)

    // Collect and sort bids (descending - highest bid first)
    let mut bids: Vec<_> = mbp.bids.values().collect();
    bids.sort_by(|a, b| b.price.cmp(&a.price)); // Descending (highest bid first)

    // Print header
    println!(
        "{:>10} {:>12} {:>8}  |  {:>10} {:>12} {:>8}",
        "Ask Qty", "Ask Price", "Orders", "Bid Price", "Bid Qty", "Orders"
    );
    println!("{}", "-".repeat(70));

    // Determine max rows
    let max_rows = asks.len().max(bids.len());

    for i in 0..max_rows {
        let ask_str = asks
            .get(i)
            .map_or(format!("{:>10} {:>12} {:>8}", "", "", ""), |a| {
                format!(
                    "{:>10} {:>12} {:>8}",
                    a.total_quantity,
                    format_price(a.price),
                    a.order_count
                )
            });

        let bid_str = bids
            .get(i)
            .map_or(format!("{:>10} {:>12} {:>8}", "", "", ""), |b| {
                format!(
                    "{:>10} {:>12} {:>8}",
                    format_price(b.price),
                    b.total_quantity,
                    b.order_count
                )
            });

        println!("{}  |  {}", ask_str, bid_str);
    }
}
