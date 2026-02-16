use dbn::MboMsg;
use dbn::enums::Action as DbnAction;
use dbn::enums::Side as DbnSide;
use dbn::flags;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use strum::Display;
use thiserror::Error;
use tracing::debug;

use crate::{Order, OrderBook, OrderBookError, Side};

#[derive(Debug, Error, Clone)]
pub enum MboProcessError {
    #[error("Action {0} is not supported.")]
    UnknownAction(i8),

    #[error("Could not convert {0} to a bid/ask.")]
    SideConversionError(i8),

    #[error(transparent)]
    OrderBookError(#[from] OrderBookError),
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
        let dbn_action = msg
            .action()
            .map_err(|_| MboProcessError::UnknownAction(msg.action))?;
        let action = convert_action(dbn_action)?;

        let dbn_side = msg
            .side()
            .map_err(|_| MboProcessError::SideConversionError(msg.side))?;
        let side = convert_side(dbn_side, action)?;

        let is_last = msg.flags.raw() & flags::LAST != 0;

        Ok(MarketByOrderMessage {
            action,
            side,
            price: msg.price,
            order_id: msg.order_id,
            size: msg.size,
            is_last,
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
        }
    }
}

/// Market-By-Order processor that maintains an in-memory order book,
/// and emits desired market-by-price or other views.
///
/// The order book is only in a consistent, queryable state after a message
/// with `is_last == true` has been processed (the dbn `F_LAST` flag marks
/// the end of an exchange event).
#[derive(Debug)]
pub struct MboProcessor {
    order_book: OrderBook,
    /// Whether the last processed message had the LAST flag set.
    event_complete: bool,
}

impl Default for MboProcessor {
    fn default() -> Self {
        Self {
            order_book: OrderBook::default(),
            // Start as true so the initial (empty) state is considered consistent.
            event_complete: true,
        }
    }
}

impl MboProcessor {
    pub fn new() -> Self {
        Self::default()
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

    /// Processes an incoming MBO message and updates the order book accordingly.
    ///
    /// Only Add, Cancel, Modify, and Clear actions modify the order book.
    /// Fill and Trade are informational and do not change order sizes —
    /// actual size changes arrive as separate Modify or Cancel messages.
    pub fn process_message(
        &mut self,
        message: &MarketByOrderMessage,
    ) -> Result<(), MboProcessError> {
        self.event_complete = message.is_last;

        match message.action {
            Action::Add => {
                debug!(
                    "Adding order ID {}: side {:?}, price {}, size {}",
                    message.order_id, message.side, message.price, message.size
                );
                self.order_book.add_order(Order::from(message));
            }
            Action::Cancel => {
                debug!("Cancelling order ID {}", message.order_id);
                self.order_book.remove_order(message.order_id);
            }
            Action::Modify => {
                debug!(
                    "Modifying order ID {} to price {}, size {}",
                    message.order_id, message.price, message.size
                );
                // Modify can change price, size, or both. Remove and re-add to handle all cases.
                self.order_book.remove_order(message.order_id);
                self.order_book.add_order(Order::from(message));
            }
            Action::Fill | Action::Trade => {
                // Fill and Trade do NOT modify the order book.
                // If a trade affects a resting order's size, Databento sends
                // a separate Modify or Cancel message for that change.
                debug!(
                    "Ignoring {} action for order ID {}",
                    message.action, message.order_id
                );
            }
            Action::Clear => {
                // Order book will be rebuilt using subsequent messages.
                debug!("Clearing order book");
                self.order_book = OrderBook::new();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(
        action: Action,
        order_id: u64,
        side: Side,
        price: i64,
        size: u32,
        is_last: bool,
    ) -> MarketByOrderMessage {
        MarketByOrderMessage {
            action,
            side,
            price,
            order_id,
            size,
            is_last,
        }
    }

    #[test]
    fn test_fill_does_not_modify_book() {
        let mut proc = MboProcessor::new();
        // Add an order
        proc.process_message(&msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Fill should NOT change the order
        proc.process_message(&msg(Action::Fill, 1, Side::Bid, 100, 20, false))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Full fill should also NOT change the order
        proc.process_message(&msg(Action::Fill, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));
    }

    #[test]
    fn test_trade_does_not_modify_book() {
        let mut proc = MboProcessor::new();
        proc.process_message(&msg(Action::Add, 1, Side::Ask, 200, 30, true))
            .unwrap();
        assert_eq!(proc.order_book().best_ask(), Some((200, 30)));

        // Trade should NOT change the order
        proc.process_message(&msg(Action::Trade, 1, Side::Ask, 200, 10, true))
            .unwrap();
        assert_eq!(proc.order_book().best_ask(), Some((200, 30)));
    }

    #[test]
    fn test_event_complete_tracks_last_flag() {
        let mut proc = MboProcessor::new();
        // Initially event_complete is true
        assert!(proc.is_event_complete());

        // Non-LAST message sets event_complete to false
        proc.process_message(&msg(Action::Add, 1, Side::Bid, 100, 50, false))
            .unwrap();
        assert!(!proc.is_event_complete());

        // LAST message sets event_complete to true
        proc.process_message(&msg(Action::Add, 2, Side::Bid, 100, 30, true))
            .unwrap();
        assert!(proc.is_event_complete());

        // Trade without LAST -> incomplete
        proc.process_message(&msg(Action::Trade, 99, Side::Bid, 100, 10, false))
            .unwrap();
        assert!(!proc.is_event_complete());

        // Fill with LAST -> complete
        proc.process_message(&msg(Action::Fill, 1, Side::Bid, 100, 10, true))
            .unwrap();
        assert!(proc.is_event_complete());
    }

    #[test]
    fn test_add_cancel_modify_still_work() {
        let mut proc = MboProcessor::new();

        // Add
        proc.process_message(&msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((100, 50)));

        // Modify (changes price and size)
        proc.process_message(&msg(Action::Modify, 1, Side::Bid, 110, 60, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), Some((110, 60)));

        // Cancel
        proc.process_message(&msg(Action::Cancel, 1, Side::Bid, 0, 0, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), None);
    }

    #[test]
    fn test_clear_resets_book() {
        let mut proc = MboProcessor::new();

        proc.process_message(&msg(Action::Add, 1, Side::Bid, 100, 50, true))
            .unwrap();
        proc.process_message(&msg(Action::Add, 2, Side::Ask, 200, 30, true))
            .unwrap();
        assert!(proc.order_book().best_bid().is_some());
        assert!(proc.order_book().best_ask().is_some());

        proc.process_message(&msg(Action::Clear, 0, Side::Bid, 0, 0, true))
            .unwrap();
        assert_eq!(proc.order_book().best_bid(), None);
        assert_eq!(proc.order_book().best_ask(), None);
    }
}
