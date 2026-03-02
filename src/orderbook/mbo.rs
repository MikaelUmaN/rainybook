use dbn::MboMsg;
use dbn::enums::Action as DbnAction;
use dbn::enums::Side as DbnSide;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use strum::Display;
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use tracing::debug;

use crate::orderbook::events::{
    OrderAddedEvent, OrderCancelledEvent, OrderModifiedEvent, TradeEvent,
};
use crate::orderbook::{Order, OrderBook, OrderBookError, Side};

/// Observer trait for reacting to MBO message processing events.
///
/// All methods have default no-op implementations. Observers only override
/// the events they care about. Called synchronously during `process_message`.
///
/// The processor is generic over this trait: `MboProcessor<O: MboObserver>`.
/// Multiple observers can be composed via tuples: `(A, B)` where both implement
/// `MboObserver`.
pub trait MboObserver {
    /// Called after an Add action places an order in the book.
    fn on_order_added(&mut self, _event: &OrderAddedEvent) {}

    /// Called after a Cancel action removes an order from the book.
    fn on_order_cancelled(&mut self, _event: &OrderCancelledEvent) {}

    /// Called after a Modify action updates an order in the book.
    fn on_order_modified(&mut self, _event: &OrderModifiedEvent) {}

    /// Called when a trade occurs (both aggressive and passive sides).
    /// The `aggressor` field distinguishes Trade (true) from Fill (false).
    fn on_trade(&mut self, _event: &TradeEvent) {}

    /// Called after a Clear action resets the book.
    fn on_clear(&mut self) {}

    /// Called after any message where `is_last` is true.
    /// The book is in a consistent state at this point, suitable for
    /// snapshot extraction or top-of-book sampling.
    fn on_event_complete(
        &mut self,
        _book: &OrderBook,
        _event_time: OffsetDateTime,
        _recv_time: OffsetDateTime,
    ) {
    }
}

/// Zero-cost no-op observer. All methods are optimized away by the compiler.
impl MboObserver for () {}

/// Compose two observers. Both receive every event.
/// Usage: `MboProcessor::with_observer((observer_a, observer_b))`
impl<A: MboObserver, B: MboObserver> MboObserver for (A, B) {
    fn on_order_added(&mut self, event: &OrderAddedEvent) {
        self.0.on_order_added(event);
        self.1.on_order_added(event);
    }

    fn on_order_cancelled(&mut self, event: &OrderCancelledEvent) {
        self.0.on_order_cancelled(event);
        self.1.on_order_cancelled(event);
    }

    fn on_order_modified(&mut self, event: &OrderModifiedEvent) {
        self.0.on_order_modified(event);
        self.1.on_order_modified(event);
    }

    fn on_trade(&mut self, event: &TradeEvent) {
        self.0.on_trade(event);
        self.1.on_trade(event);
    }

    fn on_clear(&mut self) {
        self.0.on_clear();
        self.1.on_clear();
    }

    fn on_event_complete(
        &mut self,
        book: &OrderBook,
        event_time: OffsetDateTime,
        recv_time: OffsetDateTime,
    ) {
        self.0.on_event_complete(book, event_time, recv_time);
        self.1.on_event_complete(book, event_time, recv_time);
    }
}

#[derive(Debug, Error, Clone)]
pub enum MboProcessError {
    #[error("Action {0} is not supported.")]
    UnknownAction(i8),

    #[error("Could not convert {0} to a bid/ask.")]
    SideConversionError(i8),

    #[error(transparent)]
    OrderBookError(#[from] OrderBookError),

    #[error("Record type from flag bits {0} is not supported. Only MBO records are supported.")]
    UnsupportedRecordType(u8),
}

/// Action for an market-by-order record.
#[repr(i8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display, TryFromPrimitive, IntoPrimitive)]
pub enum Action {
    Add = 1,
    Cancel = 2,
    Modify = 3,
    Fill = 4,
    /// Record for when the book is cleared (e.g., at the start of a new trading day).
    Clear = 5,
    Trade = 6,
}

/// A market-by-order message that is either an order, a trade or a system event.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct MarketByOrderMessage {
    pub action: Action,
    pub side: Side,
    pub price: i64,
    pub order_id: u64,
    pub size: u32,
    /// True when the dbn LAST flag (`F_LAST`) is set, marking the end of an event.
    /// The order book is only in a consistent state after processing a LAST-flagged message.
    pub is_last: bool,
    /// The sequence number (assigned by the venue) of the message.
    pub sequence: u32,
    /// Exchange event timestamp.
    pub event_time: OffsetDateTime,
    /// Server receive timestamp.
    pub recv_time: OffsetDateTime,
    /// Duration delta before `recv_time`.
    pub ts_in_delta: Duration,
}

fn convert_action(dbn_action: DbnAction) -> Result<Action, MboProcessError> {
    match dbn_action {
        DbnAction::Add => Ok(Action::Add),
        DbnAction::Cancel => Ok(Action::Cancel),
        DbnAction::Modify => Ok(Action::Modify),
        DbnAction::Fill => Ok(Action::Fill),
        DbnAction::Clear => Ok(Action::Clear),
        DbnAction::Trade => Ok(Action::Trade),
        _ => Err(MboProcessError::UnknownAction(dbn_action as i8)),
    }
}

fn convert_side(dbn_side: DbnSide, action: Action) -> Result<Side, MboProcessError> {
    match dbn_side {
        DbnSide::Bid => Ok(Side::Bid),
        DbnSide::Ask => Ok(Side::Ask),
        DbnSide::None => match action {
            // These actions don't use side in processing; use dummy value.
            // Cancel/Fill look up by order_id only. Clear resets book. Trade is ignored.
            Action::Clear | Action::Trade | Action::Cancel | Action::Fill => Ok(Side::Bid),
            _ => Err(MboProcessError::SideConversionError(b'N' as i8)),
        },
    }
}

impl TryFrom<&MboMsg> for MarketByOrderMessage {
    type Error = MboProcessError;

    fn try_from(msg: &MboMsg) -> Result<Self, Self::Error> {
        if msg.flags.is_mbp() || msg.flags.is_tob() {
            return Err(MboProcessError::UnsupportedRecordType(msg.flags.raw()));
        }

        let dbn_action = msg
            .action()
            .map_err(|_| MboProcessError::UnknownAction(msg.action))?;
        let action = convert_action(dbn_action)?;

        let dbn_side = msg
            .side()
            .map_err(|_| MboProcessError::SideConversionError(msg.side))?;
        let side = convert_side(dbn_side, action)?;

        Ok(MarketByOrderMessage {
            action,
            side,
            price: msg.price,
            order_id: msg.order_id,
            size: msg.size,
            is_last: msg.flags.is_last(),
            sequence: msg.sequence,
            event_time: OffsetDateTime::from_unix_timestamp_nanos(msg.hd.ts_event as i128)
                .expect("dbn ts_event is within supported range"),
            recv_time: OffsetDateTime::from_unix_timestamp_nanos(msg.ts_recv as i128)
                .expect("dbn ts_recv is within supported range"),
            ts_in_delta: Duration::nanoseconds(msg.ts_in_delta as i64),
        })
    }
}

impl From<&MarketByOrderMessage> for Order {
    fn from(msg: &MarketByOrderMessage) -> Self {
        Self {
            order_id: msg.order_id,
            side: msg.side,
            price: msg.price,
            size: msg.size.into(),
            sequence: msg.sequence,
        }
    }
}

/// Market-By-Order processor that maintains an in-memory order book,
/// and emits desired market-by-price or other views.
///
/// The order book is only in a consistent, queryable state after a message
/// with `is_last == true` has been processed (the dbn `F_LAST` flag marks
/// the end of an exchange event).
///
/// Generic over an observer `O` that receives events during message processing.
/// Defaults to `()` (zero-cost no-op). Use `with_observer` to supply a custom
/// observer, or compose multiple via tuples: `MboProcessor::with_observer((a, b))`.
#[derive(Debug)]
pub struct MboProcessor<O: MboObserver = ()> {
    order_book: OrderBook,
    observer: O,
    /// Whether the last processed message had the LAST flag set.
    event_complete: bool,
    /// The sequence number (assigned by the venue) of the last record processed.
    sequence_number: u32,
    /// Exchange event timestamp of the last processed message.
    last_event_time: OffsetDateTime,
    /// Server receive timestamp of the last processed message.
    last_recv_time: OffsetDateTime,
    /// Duration delta of the last processed message.
    last_ts_in_delta: Duration,
}

impl Default for MboProcessor {
    fn default() -> Self {
        Self {
            order_book: OrderBook::default(),
            observer: (),
            // Start as true so the initial (empty) state is considered consistent.
            event_complete: true,
            sequence_number: 0,
            last_event_time: OffsetDateTime::UNIX_EPOCH,
            last_recv_time: OffsetDateTime::UNIX_EPOCH,
            last_ts_in_delta: Duration::ZERO,
        }
    }
}

impl MboProcessor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<O: MboObserver> MboProcessor<O> {
    /// Creates a new processor with the given observer.
    pub fn with_observer(observer: O) -> Self {
        Self {
            order_book: OrderBook::default(),
            observer,
            event_complete: true,
            sequence_number: 0,
            last_event_time: OffsetDateTime::UNIX_EPOCH,
            last_recv_time: OffsetDateTime::UNIX_EPOCH,
            last_ts_in_delta: Duration::ZERO,
        }
    }

    /// Returns a reference to the observer.
    pub fn observer(&self) -> &O {
        &self.observer
    }

    /// Returns a mutable reference to the observer.
    pub fn observer_mut(&mut self) -> &mut O {
        &mut self.observer
    }

    /// Consumes the processor and returns the observer.
    pub fn into_observer(self) -> O {
        self.observer
    }

    pub fn order_book(&self) -> &OrderBook {
        &self.order_book
    }

    /// Returns true if the last processed message had the LAST flag set,
    /// meaning the order book is in a consistent state suitable for
    /// MBP snapshot extraction.
    pub fn is_event_complete(&self) -> bool {
        self.event_complete
    }

    pub fn last_sequence_number(&self) -> u32 {
        self.sequence_number
    }

    /// Returns the event timestamp (exchange timestamp) of the last processed message.
    /// Returns `OffsetDateTime::UNIX_EPOCH` if no messages have been processed.
    pub fn last_event_time(&self) -> OffsetDateTime {
        self.last_event_time
    }

    /// Returns the receive timestamp (server timestamp) of the last processed message.
    /// Returns `OffsetDateTime::UNIX_EPOCH` if no messages have been processed.
    pub fn last_recv_time(&self) -> OffsetDateTime {
        self.last_recv_time
    }

    /// Returns the duration delta of the last processed message.
    pub fn last_ts_in_delta(&self) -> Duration {
        self.last_ts_in_delta
    }

    /// Returns all timestamp information as a tuple: (event_time, recv_time, ts_in_delta).
    /// This is useful when you need all timestamp data together.
    pub fn last_timestamps(&self) -> (OffsetDateTime, OffsetDateTime, Duration) {
        (
            self.last_event_time,
            self.last_recv_time,
            self.last_ts_in_delta,
        )
    }

    /// Processes an incoming MBO message and updates the order book accordingly.
    ///
    /// Only Add, Cancel, Modify, and Clear actions modify the order book.
    /// Fill and Trade are informational and do not change order sizes —
    /// actual size changes arrive as separate Modify or Cancel messages.
    ///
    /// Observer callbacks are fired after the book mutation completes.
    /// If `is_last` is set, `on_event_complete` is called with the consistent book state.
    pub fn process_message(
        &mut self,
        message: &MarketByOrderMessage,
    ) -> Result<(), MboProcessError> {
        self.event_complete = message.is_last;
        self.sequence_number = message.sequence;
        self.last_event_time = message.event_time;
        self.last_recv_time = message.recv_time;
        self.last_ts_in_delta = message.ts_in_delta;

        match message.action {
            Action::Add => {
                debug!(
                    "Adding order ID {}: side {:?}, price {}, size {}",
                    message.order_id, message.side, message.price, message.size
                );
                let info = self.order_book.add_order(Order::from(message));
                self.observer.on_order_added(&OrderAddedEvent {
                    order: info.order,
                    level_qty: info.level_qty,
                    level_order_count: info.level_order_count,
                    new_level: info.new_level,
                    event_time: message.event_time,
                    recv_time: message.recv_time,
                    sequence: message.sequence,
                });
            }
            Action::Cancel => {
                debug!("Cancelling order ID {}", message.order_id);
                if let Some(info) = self.order_book.remove_order(message.order_id) {
                    self.observer.on_order_cancelled(&OrderCancelledEvent {
                        order: info.order,
                        remaining_level_qty: info.remaining_level_qty,
                        remaining_level_count: info.remaining_level_count,
                        level_removed: info.level_removed,
                        event_time: message.event_time,
                        recv_time: message.recv_time,
                        sequence: message.sequence,
                    });
                }
            }
            Action::Modify => {
                debug!(
                    "Modifying order ID {} to price {}, size {}",
                    message.order_id, message.price, message.size
                );
                if let Some(info) = self.order_book.modify_order(Order::from(message)) {
                    self.observer.on_order_modified(&OrderModifiedEvent {
                        order: info.order,
                        old_price: info.old_price,
                        old_size: info.old_size,
                        level_qty: info.level_qty,
                        level_order_count: info.level_order_count,
                        retained_queue_position: info.retained_queue_position,
                        event_time: message.event_time,
                        recv_time: message.recv_time,
                        sequence: message.sequence,
                    });
                }
            }
            Action::Fill | Action::Trade => {
                // Fill and Trade do NOT modify the order book.
                // If a trade affects a resting order's size, Databento sends
                // a separate Modify or Cancel message for that change.
                self.observer.on_trade(&TradeEvent {
                    price: message.price,
                    size: message.size,
                    side: message.side,
                    aggressor: message.action == Action::Trade,
                    event_time: message.event_time,
                    recv_time: message.recv_time,
                    sequence: message.sequence,
                });
            }
            Action::Clear => {
                // Order book will be rebuilt using subsequent messages.
                debug!("Clearing order book");
                self.order_book = OrderBook::new();
                self.observer.on_clear();
            }
        }

        if message.is_last {
            self.observer.on_event_complete(
                &self.order_book,
                self.last_event_time,
                self.last_recv_time,
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use time::{Duration, OffsetDateTime};

    fn ts(s: &str) -> OffsetDateTime {
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).unwrap()
    }

    /// Helper for creating test messages with auto-incrementing sequence numbers and timestamps.
    /// Each test should create its own instance to get monotonic sequences and timestamps.
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
                time_increment: Duration::milliseconds(1),
            }
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
                ts_in_delta: Duration::microseconds(-10),
            }
        }
    }

    #[test]
    fn test_fill_does_not_modify_book() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        // Add an order
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Fill should NOT change the order
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 20, false))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Full fill should also NOT change the order
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));
    }

    #[test]
    fn test_trade_does_not_modify_book() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Ask, 200, 30, true))
            .unwrap();
        assert_eq!(proc.order_book().best_ask(), Some((200, 30)));

        // Trade should NOT change the order
        proc.process_message(&seq.msg(Action::Trade, 1, Side::Ask, 200, 10, true))
            .unwrap();
        assert_eq!(proc.order_book().best_ask(), Some((200, 30)));
    }

    #[test]
    fn test_event_complete_tracks_last_flag() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        // Initially event_complete is true
        assert!(proc.is_event_complete());

        // Non-LAST message sets event_complete to false
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, false))
            .unwrap();
        assert!(!proc.is_event_complete());

        // LAST message sets event_complete to true
        proc.process_message(&seq.msg(Action::Add, 2, Side::Bid, 100, 30, true))
            .unwrap();
        assert!(proc.is_event_complete());

        // Trade without LAST -> incomplete
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Bid, 100, 10, false))
            .unwrap();
        assert!(!proc.is_event_complete());

        // Fill with LAST -> complete
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 10, true))
            .unwrap();
        assert!(proc.is_event_complete());
    }

    #[test]
    fn test_add_cancel_modify_still_work() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        // Add
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Modify (changes price and size)
        proc.process_message(&seq.msg(Action::Modify, 1, Side::Bid, 110, 60, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((110, 60)));

        // Cancel
        proc.process_message(&seq.msg(Action::Cancel, 1, Side::Bid, 0, 0, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), None);
    }

    #[test]
    fn test_clear_resets_book() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Add, 2, Side::Ask, 200, 30, true))
            .unwrap();
        assert!(proc.order_book().best_bid().is_some());
        assert!(proc.order_book().best_ask().is_some());

        proc.process_message(&seq.msg(Action::Clear, 0, Side::Bid, 0, 0, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), None);
        assert_eq!(proc.order_book().best_ask(), None);
    }

    #[test]
    fn test_processor_tracks_timestamps() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        // Initially timestamps are at UNIX epoch
        assert_eq!(proc.last_event_time(), OffsetDateTime::UNIX_EPOCH);
        assert_eq!(proc.last_recv_time(), OffsetDateTime::UNIX_EPOCH);

        // Process first message
        let msg1 = seq.msg(Action::Add, 1, Side::Bid, 100, 50, true);
        proc.process_message(&msg1).unwrap();

        assert_eq!(proc.last_event_time(), msg1.event_time);
        assert_eq!(proc.last_recv_time(), msg1.recv_time);
        assert_eq!(proc.last_ts_in_delta(), msg1.ts_in_delta);

        // Process second message - timestamps should update
        let msg2 = seq.msg(Action::Add, 2, Side::Bid, 99, 30, true);
        proc.process_message(&msg2).unwrap();

        assert_eq!(proc.last_event_time(), msg2.event_time);
        assert!(proc.last_event_time() > msg1.event_time);
    }

    #[test]
    fn test_last_timestamps_tuple() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        let msg = seq.msg(Action::Add, 1, Side::Bid, 100, 50, true);
        proc.process_message(&msg).unwrap();

        let (event_time, recv_time, delta) = proc.last_timestamps();
        assert_eq!(event_time, msg.event_time);
        assert_eq!(recv_time, msg.recv_time);
        assert_eq!(delta, msg.ts_in_delta);
    }

    // --- Observer tests ---

    /// Test observer that counts how many times each callback is invoked.
    #[derive(Debug, Default)]
    struct CountingObserver {
        adds: u32,
        cancels: u32,
        modifies: u32,
        trades: u32,
        clears: u32,
        event_completes: u32,
    }

    impl MboObserver for CountingObserver {
        fn on_order_added(&mut self, _event: &OrderAddedEvent) {
            self.adds += 1;
        }
        fn on_order_cancelled(&mut self, _event: &OrderCancelledEvent) {
            self.cancels += 1;
        }
        fn on_order_modified(&mut self, _event: &OrderModifiedEvent) {
            self.modifies += 1;
        }
        fn on_trade(&mut self, _event: &TradeEvent) {
            self.trades += 1;
        }
        fn on_clear(&mut self) {
            self.clears += 1;
        }
        fn on_event_complete(
            &mut self,
            _book: &OrderBook,
            _event_time: OffsetDateTime,
            _recv_time: OffsetDateTime,
        ) {
            self.event_completes += 1;
        }
    }

    #[test]
    fn test_observer_called_for_each_action() {
        let mut proc = MboProcessor::with_observer(CountingObserver::default());
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Modify, 1, Side::Bid, 110, 60, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Cancel, 1, Side::Bid, 0, 0, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Fill, 99, Side::Bid, 100, 10, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Clear, 0, Side::Bid, 0, 0, true))
            .unwrap();

        let obs = proc.observer();
        assert_eq!(obs.adds, 1);
        assert_eq!(obs.modifies, 1);
        assert_eq!(obs.cancels, 1);
        assert_eq!(obs.trades, 2); // Trade + Fill both fire on_trade
        assert_eq!(obs.clears, 1);
    }

    #[test]
    fn test_event_complete_fires_only_on_is_last() {
        let mut proc = MboProcessor::with_observer(CountingObserver::default());
        let mut seq = TestMessageBuilder::new();

        // Non-LAST message should not fire event_complete
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, false))
            .unwrap();
        assert_eq!(proc.observer().event_completes, 0);

        // LAST message should fire event_complete
        proc.process_message(&seq.msg(Action::Add, 2, Side::Bid, 99, 30, true))
            .unwrap();
        assert_eq!(proc.observer().event_completes, 1);

        // Another non-LAST
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Bid, 100, 10, false))
            .unwrap();
        assert_eq!(proc.observer().event_completes, 1);

        // LAST again
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 10, true))
            .unwrap();
        assert_eq!(proc.observer().event_completes, 2);
    }

    #[test]
    fn test_event_complete_receives_correct_book_and_timestamps() {
        /// Observer that captures the book state at event completion.
        #[derive(Debug)]
        struct SnapshotObserver {
            best_bid_at_complete: Option<(i64, u64)>,
            captured_event_time: OffsetDateTime,
            captured_recv_time: OffsetDateTime,
        }

        impl Default for SnapshotObserver {
            fn default() -> Self {
                Self {
                    best_bid_at_complete: None,
                    captured_event_time: OffsetDateTime::UNIX_EPOCH,
                    captured_recv_time: OffsetDateTime::UNIX_EPOCH,
                }
            }
        }

        impl MboObserver for SnapshotObserver {
            fn on_event_complete(
                &mut self,
                book: &OrderBook,
                event_time: OffsetDateTime,
                recv_time: OffsetDateTime,
            ) {
                self.best_bid_at_complete = book.best_bid();
                self.captured_event_time = event_time;
                self.captured_recv_time = recv_time;
            }
        }

        let mut proc = MboProcessor::with_observer(SnapshotObserver::default());
        let mut seq = TestMessageBuilder::new();

        let msg = seq.msg(Action::Add, 1, Side::Bid, 100, 50, true);
        let expected_event_time = msg.event_time;
        let expected_recv_time = msg.recv_time;
        proc.process_message(&msg).unwrap();

        let obs = proc.observer();
        assert_eq!(obs.best_bid_at_complete, Some((100, 50)));
        assert_eq!(obs.captured_event_time, expected_event_time);
        assert_eq!(obs.captured_recv_time, expected_recv_time);
    }

    #[test]
    fn test_unit_observer_compiles_and_runs() {
        let mut proc = MboProcessor::new();
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));
    }

    #[test]
    fn test_tuple_observer_both_called() {
        let obs_a = CountingObserver::default();
        let obs_b = CountingObserver::default();
        let mut proc = MboProcessor::with_observer((obs_a, obs_b));
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Bid, 100, 10, true))
            .unwrap();

        let (a, b) = proc.observer();
        assert_eq!(a.adds, 1);
        assert_eq!(a.trades, 1);
        assert_eq!(a.event_completes, 2);
        assert_eq!(b.adds, 1);
        assert_eq!(b.trades, 1);
        assert_eq!(b.event_completes, 2);
    }

    #[test]
    fn test_into_observer_extracts_state() {
        let mut proc = MboProcessor::with_observer(CountingObserver::default());
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Add, 2, Side::Bid, 99, 30, true))
            .unwrap();

        let obs = proc.into_observer();
        assert_eq!(obs.adds, 2);
        assert_eq!(obs.event_completes, 2);
    }

    #[test]
    fn test_order_added_event_contains_level_info() {
        #[derive(Debug, Default)]
        struct AddObserver {
            last_event: Option<OrderAddedEvent>,
        }
        impl MboObserver for AddObserver {
            fn on_order_added(&mut self, event: &OrderAddedEvent) {
                self.last_event = Some(*event);
            }
        }

        let mut proc = MboProcessor::with_observer(AddObserver::default());
        let mut seq = TestMessageBuilder::new();

        // First order at price 100 — new level
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        let evt = proc.observer().last_event.unwrap();
        assert_eq!(evt.order.price, 100);
        assert_eq!(evt.order.size, 50);
        assert_eq!(evt.level_qty, 50);
        assert_eq!(evt.level_order_count, 1);
        assert!(evt.new_level);

        // Second order at same price — existing level
        proc.process_message(&seq.msg(Action::Add, 2, Side::Bid, 100, 30, true))
            .unwrap();
        let evt = proc.observer().last_event.unwrap();
        assert_eq!(evt.level_qty, 80); // 50 + 30
        assert_eq!(evt.level_order_count, 2);
        assert!(!evt.new_level);
    }

    #[test]
    fn test_order_cancelled_event_contains_level_info() {
        #[derive(Debug, Default)]
        struct CancelObserver {
            last_event: Option<OrderCancelledEvent>,
        }
        impl MboObserver for CancelObserver {
            fn on_order_cancelled(&mut self, event: &OrderCancelledEvent) {
                self.last_event = Some(*event);
            }
        }

        let mut proc = MboProcessor::with_observer(CancelObserver::default());
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&seq.msg(Action::Add, 2, Side::Bid, 100, 30, true))
            .unwrap();

        // Cancel one of two orders at this level
        proc.process_message(&seq.msg(Action::Cancel, 1, Side::Bid, 0, 0, true))
            .unwrap();
        let evt = proc.observer().last_event.unwrap();
        assert_eq!(evt.order.order_id, 1);
        assert_eq!(evt.order.size, 50);
        assert_eq!(evt.remaining_level_qty, 30);
        assert_eq!(evt.remaining_level_count, 1);
        assert!(!evt.level_removed);

        // Cancel the last order — level should be removed
        proc.process_message(&seq.msg(Action::Cancel, 2, Side::Bid, 0, 0, true))
            .unwrap();
        let evt = proc.observer().last_event.unwrap();
        assert_eq!(evt.order.order_id, 2);
        assert_eq!(evt.remaining_level_qty, 0);
        assert_eq!(evt.remaining_level_count, 0);
        assert!(evt.level_removed);
    }

    #[test]
    fn test_order_modified_event_captures_old_state() {
        #[derive(Debug, Default)]
        struct ModifyObserver {
            last_event: Option<OrderModifiedEvent>,
        }
        impl MboObserver for ModifyObserver {
            fn on_order_modified(&mut self, event: &OrderModifiedEvent) {
                self.last_event = Some(*event);
            }
        }

        let mut proc = MboProcessor::with_observer(ModifyObserver::default());
        let mut seq = TestMessageBuilder::new();

        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();

        // Modify price and size
        proc.process_message(&seq.msg(Action::Modify, 1, Side::Bid, 110, 60, true))
            .unwrap();
        let evt = proc.observer().last_event.unwrap();
        assert_eq!(evt.order.price, 110);
        assert_eq!(evt.order.size, 60);
        assert_eq!(evt.old_price, 100);
        assert_eq!(evt.old_size, 50);
        assert_eq!(evt.level_qty, 60);
        assert_eq!(evt.level_order_count, 1);
    }

    #[test]
    fn test_trade_event_aggressor_flag() {
        #[derive(Debug, Default)]
        struct TradeObserver {
            events: Vec<TradeEvent>,
        }
        impl MboObserver for TradeObserver {
            fn on_trade(&mut self, event: &TradeEvent) {
                self.events.push(*event);
            }
        }

        let mut proc = MboProcessor::with_observer(TradeObserver::default());
        let mut seq = TestMessageBuilder::new();

        // Trade action → aggressor = true
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Ask, 100, 10, false))
            .unwrap();
        // Fill action → aggressor = false
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 10, true))
            .unwrap();

        let events = &proc.observer().events;
        assert_eq!(events.len(), 2);
        assert!(events[0].aggressor);
        assert!(!events[1].aggressor);
    }

    #[test]
    fn test_trade_collector_observer() {
        use crate::orderbook::tradestream::TradeCollector;

        let mut proc = MboProcessor::with_observer(TradeCollector::new());
        let mut seq = TestMessageBuilder::new();

        // Add an order (should not produce trades)
        proc.process_message(&seq.msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.observer().trades().len(), 0);

        // Trade message should be collected (aggressor)
        proc.process_message(&seq.msg(Action::Trade, 99, Side::Ask, 100, 10, false))
            .unwrap();
        assert_eq!(proc.observer().trades().len(), 1);

        // Fill message should also be collected (passive)
        proc.process_message(&seq.msg(Action::Fill, 1, Side::Bid, 100, 20, true))
            .unwrap();
        assert_eq!(proc.observer().trades().len(), 2);

        // Verify trade data and aggressor flag
        let trades = proc.into_observer().into_trades();
        assert_eq!(trades[0].price, 100);
        assert_eq!(trades[0].size, 10);
        assert!(trades[0].aggressor);
        assert_eq!(trades[1].price, 100);
        assert_eq!(trades[1].size, 20);
        assert!(!trades[1].aggressor);
    }
}
