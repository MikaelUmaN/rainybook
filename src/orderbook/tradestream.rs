use thiserror::Error;

use crate::{MarketByOrderMessage, Side};

/// Represents a trade event in the order book.
/// Either from an aggressing trade or from a passive fill.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Trade {
    pub event_time: u64,
    pub recv_time: u64,
    pub price: i64,
    pub size: u32,
    pub side: Side,
    pub aggressor: bool,
    pub sequence: u32,
}

#[derive(Debug, Error, Clone)]
pub enum TradeProcessError {
    #[error("Action {0} is not supported.")]
    UnknownAction(i8),

    #[error("Could not convert {0} to a bid/ask.")]
    SideConversionError(i8),

    #[error("Record type from flag bits {0} is not supported. Only MBO records are supported.")]
    UnsupportedRecordType(u8),
}

impl TryFrom<&MarketByOrderMessage> for Trade {
    type Error = TradeProcessError;

    fn try_from(msg: &MarketByOrderMessage) -> Result<Self, Self::Error> {
        // TODO: Determine aggressor side based on action (Trade vs Fill)
        // For now, assume all trades are aggressor=true
        Ok(Trade {
            event_time: msg.event_time,
            recv_time: msg.recv_time,
            price: msg.price,
            size: msg.size,
            side: msg.side,
            aggressor: true,
            sequence: msg.sequence,
        })
    }
}