//! Orderbook benchmarks using criterion.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use rainybook::orderbook::OrderBook;

mod data;

use data::OrderGenerator;

/// Benchmark adding a single order to an empty book.
fn bench_add_order_empty(c: &mut Criterion) {
    c.bench_function("orderbook/add_order_empty", |b| {
        let mut generator = OrderGenerator::default_seeded(42);

        b.iter_batched(
            || (OrderBook::new(), generator.next_order()),
            |(mut book, order)| {
                book.add_order(black_box(order));
                black_box(book)
            },
            BatchSize::SmallInput,
        )
    });
}

/// Benchmark adding orders to a book with existing orders.
fn bench_add_order_populated(c: &mut Criterion) {
    c.bench_function("orderbook/add_order_populated", |b| {
        let mut generator = OrderGenerator::default_seeded(42);

        b.iter_batched(
            || {
                // Setup: create book with 1000 orders
                let mut book = OrderBook::new();
                for order in generator.make_orders(1000) {
                    book.add_order(order);
                }
                let new_order = generator.next_order();
                (book, new_order)
            },
            |(mut book, order)| {
                book.add_order(black_box(order));
                black_box(book)
            },
            BatchSize::LargeInput,
        )
    });
}

/// Benchmark removing an order from a populated book.
fn bench_remove_order(c: &mut Criterion) {
    c.bench_function("orderbook/remove_order", |b| {
        let mut generator = OrderGenerator::default_seeded(42);

        b.iter_batched(
            || {
                let mut book = OrderBook::new();
                let orders = generator.make_orders(1000);
                for order in &orders {
                    book.add_order(*order);
                }
                // Pick an order to remove (middle of the batch)
                let order_to_remove = orders[500].order_id;
                (book, order_to_remove)
            },
            |(mut book, order_id)| {
                book.remove_order(black_box(order_id));
                black_box(book)
            },
            BatchSize::LargeInput,
        )
    });
}

/// Benchmark getting best bid from a populated book.
fn bench_best_bid(c: &mut Criterion) {
    let mut generator = OrderGenerator::default_seeded(42);
    let mut book = OrderBook::new();
    for order in generator.make_orders(1000) {
        book.add_order(order);
    }

    c.bench_function("orderbook/best_bid", |b| {
        b.iter(|| black_box(book.best_bid()))
    });
}

/// Benchmark getting best ask from a populated book.
fn bench_best_ask(c: &mut Criterion) {
    let mut generator = OrderGenerator::default_seeded(42);
    let mut book = OrderBook::new();
    for order in generator.make_orders(1000) {
        book.add_order(order);
    }

    c.bench_function("orderbook/best_ask", |b| {
        b.iter(|| black_box(book.best_ask()))
    });
}

/// Benchmark getting top N bids.
fn bench_top_n_bids(c: &mut Criterion) {
    let mut generator = OrderGenerator::default_seeded(42);
    let mut book = OrderBook::new();
    for order in generator.make_orders(1000) {
        book.add_order(order);
    }

    c.bench_function("orderbook/top_10_bids", |b| {
        b.iter(|| black_box(book.top_n_bids(10)))
    });
}

/// Benchmark modifying an order in a populated book.
fn bench_modify_order(c: &mut Criterion) {
    c.bench_function("orderbook/modify_order", |b| {
        let mut generator = OrderGenerator::default_seeded(42);

        b.iter_batched(
            || {
                let mut book = OrderBook::new();
                let orders = generator.make_orders(1000);
                for order in &orders {
                    book.add_order(*order);
                }
                let order_to_modify = orders[500].order_id;
                (book, order_to_modify)
            },
            |(mut book, order_id)| {
                let _ = book.modify_order(black_box(order_id), black_box(999));
                black_box(book)
            },
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(
    benches,
    bench_add_order_empty,
    bench_add_order_populated,
    bench_remove_order,
    bench_best_bid,
    bench_best_ask,
    bench_top_n_bids,
    bench_modify_order,
);
criterion_main!(benches);
