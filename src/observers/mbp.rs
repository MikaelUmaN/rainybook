use time::{Duration, OffsetDateTime};
use tracing::info;

use crate::orderbook::{MarketByPrice, MboObserver, OrderBook};

/// Trait for consuming MarketByPrice snapshots produced by `MbpObserver`.
///
/// Implement this trait for custom output destinations (files, databases,
/// channels, etc.), or pass a closure which implements `MbpSink` automatically.
pub trait MbpSink {
    fn on_snapshot(
        &mut self,
        snapshot: &MarketByPrice,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    );
}

/// No-op sink. Used as the default when no sink is configured.
impl MbpSink for () {
    fn on_snapshot(&mut self, _: &MarketByPrice, _: OffsetDateTime, _: OffsetDateTime) {}
}

/// Compose two sinks. Both receive every snapshot.
impl<A: MbpSink, B: MbpSink> MbpSink for (A, B) {
    fn on_snapshot(
        &mut self,
        snapshot: &MarketByPrice,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    ) {
        self.0.on_snapshot(snapshot, event_time, recv_time);
        self.1.on_snapshot(snapshot, event_time, recv_time);
    }
}

/// Any closure matching the signature is a sink.
impl<F: FnMut(&MarketByPrice, OffsetDateTime, OffsetDateTime)> MbpSink for F {
    fn on_snapshot(
        &mut self,
        snapshot: &MarketByPrice,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    ) {
        self(snapshot, event_time, recv_time);
    }
}

pub struct MbpLogSink;
impl MbpSink for MbpLogSink {
    fn on_snapshot(
        &mut self,
        snapshot: &MarketByPrice,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    ) {
        match snapshot.top_of_book() {
            Some((best_bid, best_ask)) => {
                info!(
                    %recv_time,
                    %event_time,
                    n_bids = snapshot.bids.len(),
                    n_asks = snapshot.asks.len(),
                    best_bid = best_bid.price,
                    best_ask = best_ask.price,
                    "MBP snapshot"
                );
            }
            None => info!(
                %recv_time,
                %event_time,
                n_bids = snapshot.bids.len(),
                n_asks = snapshot.asks.len(),
                "MBP snapshot (no two-sided TOB)"
            ),
        }
    }
}

/// Collector that accumulates MarketByPrice snapshots into a `Vec`.
///
/// Mirrors the `TradeCollector` pattern. Use with `MbpObserver::with_sink`,
/// then retrieve results via `sink().snapshots()` or
/// `into_sink().into_snapshots()`.
#[derive(Debug, Default)]
pub struct MbpCollector {
    snapshots: Vec<MarketByPrice>,
}

impl MbpCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshots(&self) -> &[MarketByPrice] {
        &self.snapshots
    }

    pub fn into_snapshots(self) -> Vec<MarketByPrice> {
        self.snapshots
    }
}

impl MbpSink for MbpCollector {
    fn on_snapshot(
        &mut self,
        snapshot: &MarketByPrice,
        _event_time: OffsetDateTime,
        _recv_time: OffsetDateTime,
    ) {
        self.snapshots.push(snapshot.clone());
    }
}

/// Observer that generates Market By Price (MBP) snapshots from Market By Order (MBO) events.
/// Defaults to 1 minute snapshots and 10 levels.
///
/// Snapshots are forwarded to the configured `MbpSink`. Use `with_sink` to
/// provide a closure, `MbpCollector`, or any custom `MbpSink` implementation.
#[derive(Debug)]
pub struct MbpObserver<S: MbpSink = ()> {
    sample_frequency: Duration,
    last_sample_time: Option<OffsetDateTime>,
    n_levels: usize,
    sink: S,
}

impl MbpObserver<()> {
    pub fn new(sample_frequency: Duration) -> Self {
        Self {
            sample_frequency,
            last_sample_time: None,
            n_levels: 0,
            sink: (),
        }
    }
}

impl<S: MbpSink> MbpObserver<S> {
    pub fn with_sink(sample_frequency: Duration, sink: S) -> Self {
        Self {
            sample_frequency,
            last_sample_time: None,
            n_levels: 0,
            sink,
        }
    }

    pub fn with_sink_and_levels(sample_frequency: Duration, n_levels: usize, sink: S) -> Self {
        Self {
            sample_frequency,
            last_sample_time: None,
            n_levels,
            sink,
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
    }
}

impl Default for MbpObserver<()> {
    fn default() -> Self {
        Self {
            sample_frequency: Duration::minutes(1),
            last_sample_time: None,
            n_levels: 10,
            sink: (),
        }
    }
}

impl<S: MbpSink> MboObserver for MbpObserver<S> {
    fn on_event_complete(
        &mut self,
        book: &OrderBook,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    ) {
        let baseline = *self.last_sample_time.get_or_insert(recv_time);
        if recv_time - baseline >= self.sample_frequency {
            let mbp_snapshot = match self.n_levels {
                0 => MarketByPrice::from(book),
                n => MarketByPrice::from_top_n(book, n),
            };

            self.sink.on_snapshot(&mbp_snapshot, event_time, recv_time);
            self.last_sample_time = Some(recv_time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::mbo::{Action, MarketByOrderMessage, MboProcessor};
    use crate::orderbook::{Side, TradeCollector};
    use std::cell::RefCell;
    use std::rc::Rc;
    use time::{Duration, OffsetDateTime};

    fn ts(s: &str) -> OffsetDateTime {
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).unwrap()
    }

    /// Helper for creating test messages with auto-incrementing sequence numbers.
    /// Configurable start time and increment for sampling frequency tests.
    struct TestMessageBuilder {
        next_sequence: u32,
        next_event_time: OffsetDateTime,
        time_increment: Duration,
    }

    impl TestMessageBuilder {
        fn new() -> Self {
            Self {
                next_sequence: 1,
                next_event_time: ts("2024-01-01T00:00:00Z"),
                time_increment: Duration::seconds(1),
            }
        }

        fn at(mut self, time: OffsetDateTime) -> Self {
            self.next_event_time = time;
            self
        }

        fn msg(
            &mut self,
            action: Action,
            order_id: u64,
            side: Side,
            price: i64,
            size: u32,
            is_last: bool,
        ) -> MarketByOrderMessage {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            let event_time = self.next_event_time;
            self.next_event_time += self.time_increment;
            let recv_time = event_time + Duration::microseconds(50);
            MarketByOrderMessage {
                action,
                side,
                price,
                order_id,
                size,
                is_last,
                sequence,
                event_time,
                recv_time,
                ts_in_delta: Duration::ZERO,
            }
        }
    }

    #[test]
    fn test_unit_sink_compiles_and_is_noop() {
        let observer = MbpObserver::default();
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();
        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
    }

    #[test]
    fn test_closure_sink_receives_snapshots() {
        let received = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&received);

        let observer = MbpObserver::with_sink(
            Duration::ZERO,
            move |mbp: &MarketByPrice, _et: OffsetDateTime, _rt: OffsetDateTime| {
                captured.borrow_mut().push(mbp.bids.len() + mbp.asks.len());
            },
        );
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Add, 2, Side::Ask, 110, 5, true))
            .unwrap();

        let counts = received.borrow();
        assert_eq!(counts.len(), 2);
        assert_eq!(counts[0], 1); // 1 bid level
        assert_eq!(counts[1], 2); // 1 bid + 1 ask
    }

    #[test]
    fn test_collector_accumulates_snapshots() {
        let observer = MbpObserver::with_sink(Duration::ZERO, MbpCollector::new());
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Add, 2, Side::Ask, 110, 5, true))
            .unwrap();

        let snapshots = proc.into_observer().into_sink().into_snapshots();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].bids.len(), 1);
        assert_eq!(snapshots[0].asks.len(), 0);
        assert_eq!(snapshots[1].bids.len(), 1);
        assert_eq!(snapshots[1].asks.len(), 1);
    }

    #[test]
    fn test_tuple_sink_both_receive() {
        let received = Rc::new(RefCell::new(0u32));
        let captured = Rc::clone(&received);

        let sink = (
            MbpCollector::new(),
            move |_mbp: &MarketByPrice, _et: OffsetDateTime, _rt: OffsetDateTime| {
                *captured.borrow_mut() += 1;
            },
        );
        let observer = MbpObserver::with_sink(Duration::ZERO, sink);
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Add, 2, Side::Ask, 110, 5, true))
            .unwrap();

        // Closure was called twice
        assert_eq!(*received.borrow(), 2);

        // Collector also got both
        let (collector, _) = proc.into_observer().into_sink();
        assert_eq!(collector.snapshots().len(), 2);
    }

    #[test]
    fn test_composed_with_trade_collector_at_mbo_level() {
        let observer = (
            TradeCollector::new(),
            MbpObserver::with_sink(Duration::ZERO, MbpCollector::new()),
        );
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Trade, 99, Side::Ask, 100, 5, true))
            .unwrap();

        let (trade_collector, mbp_observer) = proc.into_observer();
        assert_eq!(trade_collector.trades().len(), 1);
        assert_eq!(mbp_observer.into_sink().snapshots().len(), 2);
    }

    #[test]
    fn test_sampling_frequency_filters() {
        let observer = MbpObserver::with_sink(Duration::minutes(1), MbpCollector::new());
        let mut proc = MboProcessor::with_observer(observer);

        // First message at t=0 — establishes baseline, no snapshot
        let mut b = TestMessageBuilder::new().at(ts("2024-01-01T00:00:00Z"));
        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();

        // Second message 30s later — delta 30s < 1min, skipped
        let mut b = b.at(ts("2024-01-01T00:00:30Z"));
        proc.process_message(&b.msg(Action::Add, 2, Side::Ask, 110, 5, true))
            .unwrap();

        // Third message 61s after baseline — delta 61s ≥ 1min, snapshot #1
        let mut b = b.at(ts("2024-01-01T00:01:01Z"));
        proc.process_message(&b.msg(Action::Add, 3, Side::Bid, 99, 20, true))
            .unwrap();

        // Fourth message 61s after snapshot #1 — snapshot #2
        let mut b = b.at(ts("2024-01-01T00:02:02Z"));
        proc.process_message(&b.msg(Action::Add, 4, Side::Ask, 111, 8, true))
            .unwrap();

        let snapshots = proc.into_observer().into_sink().into_snapshots();
        assert_eq!(snapshots.len(), 2); // t=+61s and t=+122s only
    }

    #[test]
    fn test_n_levels_limits_snapshot() {
        let observer = MbpObserver::with_sink_and_levels(Duration::ZERO, 1, MbpCollector::new());
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        // Add 3 bid levels
        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Add, 2, Side::Bid, 99, 20, true))
            .unwrap();
        proc.process_message(&b.msg(Action::Add, 3, Side::Bid, 98, 30, true))
            .unwrap();

        let snapshots = proc.into_observer().into_sink().into_snapshots();
        // Last snapshot should only have 1 bid level (the best)
        let last = snapshots.last().unwrap();
        assert_eq!(last.bids.len(), 1);
        assert!(last.bids.contains_key(&100));
    }

    #[test]
    fn test_non_last_messages_do_not_trigger_sink() {
        let observer = MbpObserver::with_sink(Duration::ZERO, MbpCollector::new());
        let mut proc = MboProcessor::with_observer(observer);
        let mut b = TestMessageBuilder::new();

        // is_last = false — no on_event_complete fired
        proc.process_message(&b.msg(Action::Add, 1, Side::Bid, 100, 10, false))
            .unwrap();

        assert_eq!(proc.observer().sink().snapshots().len(), 0);

        // Now is_last = true — fires
        proc.process_message(&b.msg(Action::Add, 2, Side::Ask, 110, 5, true))
            .unwrap();

        assert_eq!(proc.observer().sink().snapshots().len(), 1);
    }
}
