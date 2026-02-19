# MboProcessor Observer Pattern

## Overview

The `MboProcessor` supports an observer pattern that lets callers react to events as they occur during message processing, rather than polling state after each call. This enables use cases like:

- Collecting a trade stream from Trade/Fill messages
- Sampling top-of-book at time intervals (using exchange timestamps)
- Triggering MBP snapshot generation on each consistent book state
- Any combination of the above, running simultaneously

## Quick Start

```rust
use rainybook::{MboProcessor, MboObserver, TradeCollector};

// No observer (default) — zero overhead, identical to plain MboProcessor
let mut proc = MboProcessor::new();

// With a single observer
let mut proc = MboProcessor::with_observer(TradeCollector::new());
proc.process_message(&msg)?;
let trades = proc.observer().trades(); // borrow accumulated trades

// Extract observer when done (consumes processor)
let trades = proc.into_observer().into_trades();
```

## The MboObserver Trait

Defined in [`src/orderbook/mbo.rs`](../src/orderbook/mbo.rs). All methods have default no-op implementations — observers only override the events they care about.

Callbacks receive **domain event types** (not raw wire-format messages). Each event carries the relevant domain data — order state, level context, timestamps — decoupled from the Databento message format.

```rust
pub trait MboObserver {
    fn on_order_added(&mut self, _event: &OrderAddedEvent) {}
    fn on_order_cancelled(&mut self, _event: &OrderCancelledEvent) {}
    fn on_order_modified(&mut self, _event: &OrderModifiedEvent) {}
    fn on_trade(&mut self, _event: &TradeEvent) {}
    fn on_clear(&mut self) {}
    fn on_event_complete(&mut self, _book: &OrderBook, _event_time: u64, _recv_time: u64) {}
}
```

### Domain Event Types

Defined in [`src/orderbook/events.rs`](../src/orderbook/events.rs). All event types are `Copy` — small, stack-allocated, zero heap allocation.

**`OrderAddedEvent`** — emitted after an Add action places an order in the book:

| Field | Type | Description |
|---|---|---|
| `order` | `Order` | The order that was added |
| `level_qty` | `u64` | Total quantity at this price level after the add |
| `level_order_count` | `usize` | Number of orders at this price level after the add |
| `new_level` | `bool` | True if this order created a new price level |
| `event_time` | `u64` | Exchange event timestamp (ns since UNIX epoch) |
| `recv_time` | `u64` | Server receive timestamp (ns since UNIX epoch) |
| `sequence` | `u32` | Venue-assigned sequence number |

**`OrderCancelledEvent`** — emitted after a Cancel action removes an order:

| Field | Type | Description |
|---|---|---|
| `order` | `Order` | The order that was cancelled |
| `remaining_level_qty` | `u64` | Quantity remaining at this price level (0 if level removed) |
| `remaining_level_count` | `usize` | Orders remaining at this price level |
| `level_removed` | `bool` | True if the price level was removed entirely |
| `event_time` | `u64` | Exchange event timestamp (ns since UNIX epoch) |
| `recv_time` | `u64` | Server receive timestamp (ns since UNIX epoch) |
| `sequence` | `u32` | Venue-assigned sequence number |

**`OrderModifiedEvent`** — emitted after a Modify action updates an order:

| Field | Type | Description |
|---|---|---|
| `order` | `Order` | The order after modification |
| `old_price` | `i64` | Price before modification |
| `old_size` | `u64` | Size before modification |
| `level_qty` | `u64` | Total quantity at the new price level |
| `level_order_count` | `usize` | Number of orders at the new price level |
| `event_time` | `u64` | Exchange event timestamp (ns since UNIX epoch) |
| `recv_time` | `u64` | Server receive timestamp (ns since UNIX epoch) |
| `sequence` | `u32` | Venue-assigned sequence number |

**`TradeEvent`** — emitted for both aggressive and passive sides of a trade:

| Field | Type | Description |
|---|---|---|
| `price` | `i64` | Trade price |
| `size` | `u32` | Trade size |
| `side` | `Side` | Side of the order involved |
| `aggressor` | `bool` | `true` = Trade action (incoming order), `false` = Fill (resting side) |
| `event_time` | `u64` | Exchange event timestamp (ns since UNIX epoch) |
| `recv_time` | `u64` | Server receive timestamp (ns since UNIX epoch) |
| `sequence` | `u32` | Venue-assigned sequence number |

A single exchange trade produces two MBO messages — a Trade (aggressor) and a Fill (resting side). Both are emitted as `TradeEvent` via the same `on_trade` callback; the `aggressor` field distinguishes them.

### Callback Semantics

| Callback | Fires on | Book modified? | Book state when called |
|---|---|---|---|
| `on_order_added` | `Action::Add` | Yes | Order already in book |
| `on_order_cancelled` | `Action::Cancel` | Yes | Order already removed |
| `on_order_modified` | `Action::Modify` | Yes | Order already updated |
| `on_trade` | `Action::Trade` or `Action::Fill` | No | Unchanged |
| `on_clear` | `Action::Clear` | Yes | Book already empty |
| `on_event_complete` | Any message with `is_last == true` | After action | Consistent state |

All callbacks fire **after** the book mutation completes. The `on_event_complete` callback fires at the end, after the per-action callback, and only when the message has `is_last == true` (the Databento `F_LAST` flag). At that point the book is in a consistent state suitable for snapshot extraction.

### MboProcessor API

```rust
// Default processor — MboProcessor<()>, zero overhead
MboProcessor::new()

// With observer — MboProcessor<O> where O: MboObserver
MboProcessor::with_observer(observer)

// Access observer state
processor.observer()       // &O
processor.observer_mut()   // &mut O
processor.into_observer()  // O (consumes processor)
```

## Writing an Observer

Implement `MboObserver` and override only the callbacks you need.

### Example: Trade Collector

The built-in `TradeCollector` (in [`src/orderbook/tradestream.rs`](../src/orderbook/tradestream.rs)) collects trades from both Trade and Fill messages into a single `Vec<TradeEvent>`:

```rust
use rainybook::{MboProcessor, TradeCollector};

let mut proc = MboProcessor::with_observer(TradeCollector::new());

// Process messages...
for msg in messages {
    proc.process_message(&msg)?;
}

// Read collected trades
for trade in proc.observer().trades() {
    let side_label = if trade.aggressor { "aggressor" } else { "passive" };
    println!("{}: {} {} @ {}", trade.event_time, side_label, trade.size, trade.price);
}
```

### Example: Top-of-Book Sampler

Sample best bid/ask at regular intervals using exchange timestamps:

```rust
use rainybook::{MboObserver, OrderBook};

struct TopOfBookSampler {
    interval_ns: u64,
    last_sample_time: u64,
    samples: Vec<(u64, Option<(i64, u64)>, Option<(i64, u64)>)>, // (time, bid, ask)
}

impl TopOfBookSampler {
    fn new(interval_ns: u64) -> Self {
        Self { interval_ns, last_sample_time: 0, samples: Vec::new() }
    }
}

impl MboObserver for TopOfBookSampler {
    fn on_event_complete(&mut self, book: &OrderBook, event_time: u64, _recv_time: u64) {
        if event_time - self.last_sample_time >= self.interval_ns {
            self.samples.push((event_time, book.best_bid(), book.best_ask()));
            self.last_sample_time = event_time;
        }
    }
}
```

Usage:

```rust
use rainybook::MboProcessor;

let sampler = TopOfBookSampler::new(1_000_000_000); // every 1 second
let mut proc = MboProcessor::with_observer(sampler);

for msg in messages {
    proc.process_message(&msg)?;
}

for (time, bid, ask) in &proc.observer().samples {
    println!("{}: bid={:?} ask={:?}", time, bid, ask);
}
```

### Example: MBP Snapshot on Each Event

```rust
use rainybook::{MboObserver, MarketByPrice, OrderBook};

struct MbpSnapshotObserver {
    depth: usize,
    snapshots: Vec<MarketByPrice>,
}

impl MboObserver for MbpSnapshotObserver {
    fn on_event_complete(&mut self, book: &OrderBook, _event_time: u64, _recv_time: u64) {
        self.snapshots.push(MarketByPrice::from_top_n(book, self.depth));
    }
}
```

### Example: Tracking Level Changes

Use domain event fields to track price level creation/removal:

```rust
use rainybook::{MboObserver, OrderAddedEvent, OrderCancelledEvent};

struct LevelTracker {
    levels_created: usize,
    levels_removed: usize,
}

impl MboObserver for LevelTracker {
    fn on_order_added(&mut self, event: &OrderAddedEvent) {
        if event.new_level {
            self.levels_created += 1;
        }
    }

    fn on_order_cancelled(&mut self, event: &OrderCancelledEvent) {
        if event.level_removed {
            self.levels_removed += 1;
        }
    }
}
```

## Composing Multiple Observers

Use tuples to run multiple observers simultaneously. Both receive every event:

```rust
use rainybook::{MboProcessor, TradeCollector};

let trade_collector = TradeCollector::new();
let tob_sampler = TopOfBookSampler::new(1_000_000_000);

let mut proc = MboProcessor::with_observer((trade_collector, tob_sampler));

for msg in messages {
    proc.process_message(&msg)?;
}

let (trades_obs, sampler_obs) = proc.observer();
println!("Collected {} trades", trades_obs.trades().len());
println!("Collected {} TOB samples", sampler_obs.samples.len());
```

Tuples nest for more than two: `((A, B), C)` composes three observers.

## Dynamic Dispatch

If you need runtime-dynamic observer registration (e.g., observers determined by configuration), use `Box<dyn MboObserver>`:

```rust
impl MboObserver for Box<dyn MboObserver> {
    fn on_order_added(&mut self, event: &OrderAddedEvent) {
        (**self).on_order_added(event);
    }
    // ... delegate all methods
}

// Then use Vec<Box<dyn MboObserver>> or a single Box<dyn MboObserver>
let observer: Box<dyn MboObserver> = Box::new(TradeCollector::new());
let mut proc = MboProcessor::with_observer(observer);
```

This adds one vtable indirection per callback — negligible for most workloads, but measurable at millions of messages per second. Prefer the generic approach when observer types are known at compile time.

---

## Design Discussion

### Why This Pattern?

The `MboProcessor` was originally pull-based: process a message, then manually inspect state via getters. This works for simple use cases but becomes awkward when multiple consumers need different views of the event stream. The observer pattern lets the processor push events to interested parties during processing.

### Domain Events, Not Wire Messages

Observer callbacks receive domain event types (`OrderAddedEvent`, `TradeEvent`, etc.) rather than raw Databento `MarketByOrderMessage` references. This is a deliberate decoupling:

- **Semantic clarity.** Each event type carries exactly the fields relevant to that action — an `OrderAddedEvent` has level context (qty, count, new_level), a `TradeEvent` has the `aggressor` flag. Observers don't need to interpret a generic message struct.
- **Enriched context.** The `OrderBook` computes and returns level-state information during mutations (`AddOrderInfo`, `RemoveOrderInfo`). The processor combines this with message timestamps to construct events with richer context than the raw message alone.
- **Format independence.** If the underlying wire format changes (different exchange, different vendor), observer code is unaffected — the domain events are the stable API.

### Unified Trade Model

Databento's MBO feed sends separate Trade and Fill messages for each exchange trade. Rather than exposing two callbacks (`on_trade` + `on_fill`), we unify them into a single `on_trade` callback with a `TradeEvent`:

- `aggressor: true` — the incoming (aggressing) side of the trade
- `aggressor: false` — the resting (passive fill) side

This reflects the reality that Trade and Fill are two views of the same event. Observers that want all trade activity implement one callback; those that need to distinguish aggressor vs passive filter on the `aggressor` field.

### Why Not Channels?

Channels (`std::sync::mpsc`, `crossbeam`) are idiomatic Rust for decoupled concurrent systems. We chose synchronous observers instead because:

- **No threading needed.** MBO message processing is inherently sequential — messages must be applied in order. Channels add overhead (allocation, synchronization) for no concurrency benefit.
- **Zero-copy access.** Observers receive `&OrderAddedEvent`, `&OrderBook`, etc. directly — no serialization, cloning, or ownership transfer.
- **Backpressure is automatic.** Slow observers block processing, which is desirable: we never want the book to advance past what an observer has seen.

If downstream consumers need concurrent processing, an observer can forward events into a channel internally.

### Why Generics Over Trait Objects?

Rust offers two forms of polymorphism:

| | Generic (`MboProcessor<O>`) | Trait object (`Vec<Box<dyn MboObserver>>`) |
|---|---|---|
| Dispatch | Static (inlined) | Dynamic (vtable) |
| Overhead | Zero when `O = ()` | One allocation + vtable per observer |
| Registration | Compile-time | Runtime |
| Composability | Via tuples | Via `Vec::push` |

We chose generics because:

1. **Zero-cost default.** `MboProcessor<()>` compiles to the same code as a processor with no observer support at all. The compiler sees that all `()` methods are empty and optimizes them away entirely.

2. **Static dispatch when used.** With a concrete observer type, the compiler inlines the callback directly into `process_message`. No function pointer, no vtable lookup.

3. **Backward compatible.** The default type parameter `= ()` means existing code using `MboProcessor` (without a type parameter) compiles unchanged.

4. **Precedent.** This is the same pattern used by `serde` (`Visitor`), `syn` (`Visit`), and `tracing` (`Subscriber`).

Users who need dynamic registration can still use `Box<dyn MboObserver>` as the type parameter.

### How the Borrow Checker Allows This

The key concern with observer patterns in Rust is the borrow checker. During `process_message`, we need to:

1. Mutate `self.order_book` (apply the action)
2. Call `self.observer.on_*(...)` with `&mut self.observer`
3. For `on_event_complete`, pass `&self.order_book` to the observer

This works because Rust tracks **field-level borrows**. `self.observer` and `self.order_book` are disjoint fields of `MboProcessor`, so the compiler allows:

```rust
// This compiles — disjoint field borrows
let info = self.order_book.add_order(order);               // &mut self.order_book
self.observer.on_order_added(&OrderAddedEvent { ... });     // &mut self.observer
self.observer.on_event_complete(&self.order_book, ...);     // &mut self.observer + &self.order_book
```

If `observer` and `order_book` were behind the same `RefCell` or stored in a `Vec` together, this would not be possible without runtime borrow checking. The struct field layout makes this zero-cost.

### Callback Ordering

Events are emitted **after** the book mutation. This is a deliberate choice:

- Observers see the book in the state that results from the action (e.g., after `on_order_added`, the order is in the book).
- `on_event_complete` always sees a consistent book state.
- There is no "before mutation" callback. If needed, an observer can maintain its own pre-mutation state by tracking the book via prior `on_event_complete` snapshots.

### Tuple Composition

The `impl MboObserver for (A, B)` pattern allows composing observers without allocation. For three observers, nest: `((A, B), C)`. This is the same approach used by the `bevy` ECS and `axum` web framework for composing handlers.

Each layer of nesting adds one level of function call inlining — the compiler handles this efficiently.

### Alternatives Considered

**Event enum with single `on_event` method:**

```rust
enum MboEvent<'a> {
    OrderAdded(&'a OrderAddedEvent),
    Trade(&'a TradeEvent),
    EventComplete { book: &'a OrderBook, event_time: u64, recv_time: u64 },
}
```

Rejected because: observers must match on every event even if they only care about one, adding boilerplate. The trait-with-defaults approach lets observers ignore events they don't handle — the compiler elides the empty default methods entirely.

**Closures (`Vec<Box<dyn Fn(...)>>`):**

Rejected because: closures can't easily maintain mutable state across calls without `RefCell`, and the closure type erases the observer identity, making it hard to retrieve accumulated results.
