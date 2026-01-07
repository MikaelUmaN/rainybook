use thiserror::Error;
use tracing::{debug, error};

use super::book::{OrderBook, OrderBookError, Side};

#[derive(Debug, Error, Clone)]
pub enum MboProcessError {
    #[error("Action {0} is not supported.")]
    UnknownAction(i8),

    #[error("Could not convert {0} to a bid/ask.")]
    SideConversionError(i8),

    #[error(transparent)]
    OrderBookError(#[from] OrderBookError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    Add = 1,
    Cancel = 2,
    Modify = 3,
    Fill = 4,
    Clear = 5,
    Trade = 6,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct MarketByOrderMessage {
    pub action: Action,
    pub side: Side,
    pub price: i64,
    pub order_id: u64,
    pub size: u32,
}

#[derive(Debug, Default)]
pub struct MboProcessor {
    order_book: OrderBook,
}

impl MboProcessor {
    pub fn new() -> Self {
        Self {
            order_book: OrderBook::new(),
        }
    }

    /// Processes an incoming MBO message and updates the order book accordingly.
    pub fn process_message(&mut self, message: &MarketByOrderMessage) -> Result<(), MboProcessError> {
        debug!("Processing MBO message: {:?}", debug(message));

        let action_u8 = u8::try_from(message.action).map_err(|_| MboProcessError::UnknownAction(message.action))?;
        let action = Action::try_from(action_u8).map_err(|_| MboProcessError::UnknownAction(message.action))?;

        match action {
            Action::Add => {
                debug!(
                    "Adding order ID {}: side {:?}, price {}, size {}",
                    message.order_id, message.side, message.price, message.size
                );
                self.order_book.add_order(
                    Side::try_from(
                        message
                            .side()
                            .map_err(|_| MboProcessError::SideConversionError(message.side))?,
                    )
                    .map_err(|_| MboProcessError::SideConversionError(message.side))?,
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
                debug!("Modifying order ID {} to size {}", message.order_id, message.size);
                self.order_book
                    .modify_order(message.order_id, message.size.into())?;
            }
            Action::Fill => {
                debug!("Filling order ID {} with size {}", message.order_id, message.size);
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
            _ => {
                error!("Unknown action: {}", message.action);
                return Err(MboProcessError::UnknownAction(message.action));
            }
        }
        Ok(())
    }
}
