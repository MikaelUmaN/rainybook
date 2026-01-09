use dbn::MboMsg;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use polars::prelude::*;
use strum::Display;
use thiserror::Error;
use tracing::{debug, error};

use crate::{OrderBook, OrderBookError, Side};

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
}

impl TryFrom<&MboMsg> for MarketByOrderMessage {
    type Error = MboProcessError;

    fn try_from(msg: &MboMsg) -> Result<Self, Self::Error> {
        let action =
            Action::try_from(msg.action).map_err(|e| MboProcessError::UnknownAction(e.number))?;
        let side =
            Side::try_from(msg.side).map_err(|e| MboProcessError::SideConversionError(e.number))?;

        Ok(MarketByOrderMessage {
            action,
            side,
            price: msg.price,
            order_id: msg.order_id,
            size: msg.size,
        })
    }
}

/// Processes DataFrame to `MarketByOrderMessage`s.
pub fn into_mbo_messages(df: &DataFrame) -> PolarsResult<Vec<MarketByOrderMessage>> {
    let actions = df.column("action")?.i8()?;
    let sides = df.column("side")?.i8()?;
    let prices = df.column("price")?.i64()?;
    let order_ids = df.column("order_id")?.u64()?;
    let sizes = df.column("size")?.u32()?;

    let messages = actions
        .into_iter()
        .zip(sides)
        .zip(prices)
        .zip(order_ids)
        .zip(sizes)
        .filter_map(|((((a, s), p), oid), sz)| {
            let action = Action::try_from(a?).ok()?;
            let side = Side::try_from(s?).ok()?;
            Some(MarketByOrderMessage {
                action,
                side,
                price: p?,
                order_id: oid?,
                size: sz?,
            })
        })
        .collect();

    Ok(messages)
}

/// Market-By-Order processor that maintains an in-memory order book,
/// and emits desired market-by-price or other views.
#[derive(Debug, Default)]
pub struct MboProcessor {
    order_book: OrderBook,
}

impl MboProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes an incoming MBO message and updates the order book accordingly.
    pub fn process_message(
        &mut self,
        message: &MarketByOrderMessage,
    ) -> Result<(), MboProcessError> {
        debug!("Processing MBO message: {:?}", debug(message));
        match message.action {
            Action::Add => {
                debug!(
                    "Adding order ID {}: side {:?}, price {}, size {}",
                    message.order_id, message.side, message.price, message.size
                );
                self.order_book.add_order(
                    message.side,
                    message.price,
                    message.order_id,
                    message.size.into(),
                );
            }
            Action::Cancel => {
                debug!("Cancelling order ID {}", message.order_id);
                self.order_book.remove_order(message.order_id);
            }
            Action::Modify => {
                debug!(
                    "Modifying order ID {} to size {}",
                    message.order_id, message.size
                );
                self.order_book
                    .modify_order(message.order_id, message.size.into())?;
            }
            Action::Fill => {
                debug!(
                    "Filling order ID {} with size {}",
                    message.order_id, message.size
                );
                self.order_book
                    .fill_order(message.order_id, message.size.into())?;
            }
            Action::Clear => {
                // Order book will be rebuilt using subsequent messages.
                debug!("Clearing order book");
                self.order_book = OrderBook::new();
            }
            Action::Trade => {
                // TODO: we still want to keep the stream of trades.
                // Trades do not affect the order book.
                debug!("Ignoring trade action for order ID {}", message.order_id);
            }
        }
        Ok(())
    }
}
