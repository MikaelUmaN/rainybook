use time::OffsetDateTime;

use crate::{Order, Side};

/// Emitted when an order is added to the book.
#[derive(Debug, Clone, Copy)]
pub struct OrderAddedEvent {
    /// The order that was added.
    pub order: Order,
    /// Total quantity at this price level after the add.
    pub level_qty: u64,
    /// Number of orders at this price level after the add.
    pub level_order_count: usize,
    /// True if this order created a new price level.
    pub new_level: bool,
    /// Exchange event timestamp.
    pub event_time: OffsetDateTime,
    /// Server receive timestamp.
    pub recv_time: OffsetDateTime,
    /// Venue-assigned sequence number.
    pub sequence: u32,
}

/// Emitted when an order is cancelled/removed from the book.
#[derive(Debug, Clone, Copy)]
pub struct OrderCancelledEvent {
    /// The order that was cancelled.
    pub order: Order,
    /// Quantity remaining at this price level after the cancel (0 if level removed).
    pub remaining_level_qty: u64,
    /// Orders remaining at this price level after the cancel (0 if level removed).
    pub remaining_level_count: usize,
    /// True if the price level was removed (no more orders at this price).
    pub level_removed: bool,
    /// Exchange event timestamp.
    pub event_time: OffsetDateTime,
    /// Server receive timestamp.
    pub recv_time: OffsetDateTime,
    /// Venue-assigned sequence number.
    pub sequence: u32,
}

/// Emitted when an order is modified (price and/or size change).
#[derive(Debug, Clone, Copy)]
pub struct OrderModifiedEvent {
    /// The order after modification.
    pub order: Order,
    /// Price before modification.
    pub old_price: i64,
    /// Size before modification.
    pub old_size: u64,
    /// Total quantity at the new price level after modification.
    pub level_qty: u64,
    /// Number of orders at the new price level after modification.
    pub level_order_count: usize,
    /// True if the order kept its queue position (size decreased at same price).
    /// False if queue position was reset (price changed or size increased).
    pub retained_queue_position: bool,
    /// Exchange event timestamp.
    pub event_time: OffsetDateTime,
    /// Server receive timestamp.
    pub recv_time: OffsetDateTime,
    /// Venue-assigned sequence number.
    pub sequence: u32,
}

/// Emitted for trade activity — both aggressive and passive sides.
///
/// A single exchange trade produces two MBO messages: a Trade (aggressor)
/// and a Fill (resting side). Both are emitted as `TradeEvent` with the
/// `aggressor` flag distinguishing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TradeEvent {
    /// Trade price.
    pub price: i64,
    /// Trade size.
    pub size: u32,
    /// Side of the order involved in the trade.
    pub side: Side,
    /// True if this was the aggressor (incoming order), false if resting (passive fill).
    pub aggressor: bool,
    /// Exchange event timestamp.
    pub event_time: OffsetDateTime,
    /// Server receive timestamp.
    pub recv_time: OffsetDateTime,
    /// Venue-assigned sequence number.
    pub sequence: u32,
}
