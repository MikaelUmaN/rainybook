use rainybook::{
    MboProcessor, MbpLogSink, MbpObserver,
    generators::OrderGenerator,
    orderbook::mbo::{Action, MarketByOrderMessage},
};
use time::{Duration, OffsetDateTime};

fn main() {
    tracing_subscriber::fmt::init();

    // Sample the order book once per simulated minute.
    let observer = MbpObserver::with_sink(Duration::minutes(1), MbpLogSink);
    let mut proc = MboProcessor::with_observer(observer);
    let mut generator = OrderGenerator::default_seeded(42);

    // Simulate 150 Add events, one per simulated second (~2:30 total).
    //
    // The first event establishes the sampling baseline (no snapshot emitted).
    // Subsequent events fire when recv_time - baseline ≥ 1 minute:
    //
    //   i = 0        → baseline set, delta = 0 < 1 min, no log
    //   i = 60  (+1min) → delta = 60s ≥ 1 min → log statement #1
    //   i = 120 (+2min) → delta = 60s ≥ 1 min → log statement #2
    //   i = 149 (+2:29) → delta = 29s < 1 min → no third log
    let base_time = OffsetDateTime::now_utc();
    for i in 0..150_i64 {
        let recv_time = base_time + Duration::seconds(i);
        let event_time = recv_time - Duration::microseconds(500);
        let order = generator.next_order();
        let msg = MarketByOrderMessage {
            action: Action::Add,
            side: order.side,
            price: order.price,
            order_id: order.order_id,
            size: order.size as u32,
            is_last: true,
            sequence: i as u32,
            event_time,
            recv_time,
            ts_in_delta: Duration::ZERO,
        };
        proc.process_message(&msg).unwrap();
    }
}
