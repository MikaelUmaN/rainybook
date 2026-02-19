use crate::orderbook::events::TradeEvent;
use crate::orderbook::mbo::MboObserver;

/// Observer that collects trades from Trade and Fill actions.
///
/// Both aggressive (Trade) and passive (Fill) sides are collected into
/// a single `Vec<TradeEvent>`. The `aggressor` field on each event
/// distinguishes them.
///
/// Use with `MboProcessor::with_observer(TradeCollector::new())`, then
/// retrieve results via `processor.observer().trades()` or
/// `processor.into_observer().into_trades()`.
#[derive(Debug, Default)]
pub struct TradeCollector {
    trades: Vec<TradeEvent>,
}

impl TradeCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trades(&self) -> &[TradeEvent] {
        &self.trades
    }

    pub fn into_trades(self) -> Vec<TradeEvent> {
        self.trades
    }
}

impl MboObserver for TradeCollector {
    fn on_trade(&mut self, event: &TradeEvent) {
        self.trades.push(*event);
    }
}
