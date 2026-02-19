//! Validate MBO-built order book against Databento MBP-10 snapshots.
//!
//! Processes MBO messages to build an order book, then compares the resulting
//! top-10 price levels against Databento's ground-truth MBP-10 snapshots.
//!
//! Matching strategy:
//! 1. **Snapshot phase**: All SNAPSHOT-flagged MBO records (initial book state)
//!    are processed without advancing MBP-10. After the phase ends, the single
//!    MBP-10 SNAPSHOT record is read and compared.
//! 2. **MBP-10-driven matching**: For each MBP-10 record, advance the MBO
//!    stream until we reach the matching (ts_event, sequence). Process ALL MBO
//!    records along the way (including deep-book events that don't appear in
//!    MBP-10). When multiple MBO records share the target (ts_event, sequence),
//!    process all of them before comparing.
//!
//! Usage:
//!   cargo run --release --example validate_mbp -- \
//!     --mbo-file path/to/mbo.dbn.zst \
//!     --mbp-file path/to/mbp_10.dbn.zst

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use dbn::{
    BidAskPair, FlagSet, MboMsg, Mbp10Msg, UNDEF_PRICE,
    decode::{DbnMetadata, DecodeRecord, DynReader, dbn::Decoder},
    flags,
};

use rainybook::{Action, MarketByOrderMessage, MarketByPrice, MboProcessor, OrderLevelSummary};

#[derive(Parser)]
#[command(name = "validate_mbp")]
#[command(about = "Validate MBO-built order book against Databento MBP-10 snapshots")]
struct Cli {
    /// Path to the MBO .dbn.zst file
    #[arg(long, value_name = "FILE")]
    #[arg(value_parser = clap::value_parser!(PathBuf))]
    mbo_file: PathBuf,

    /// Path to the MBP-10 .dbn.zst file
    #[arg(long, value_name = "FILE")]
    #[arg(value_parser = clap::value_parser!(PathBuf))]
    mbp_file: PathBuf,

    /// Maximum number of MBP-10 snapshots to validate (0 = unlimited / exhaust file)
    #[arg(long, default_value_t = 0)]
    max_checks: usize,

    /// Print detailed level comparison on mismatch
    #[arg(long, default_value_t = true)]
    verbose_mismatch: bool,

    /// Print progress every N snapshots
    #[arg(long, default_value_t = 100_000)]
    progress_interval: usize,

    /// Maximum number of verbose mismatches to print
    #[arg(long, default_value_t = 10)]
    max_verbose: usize,
}

// ---------------------------------------------------------------------------
// Statistics tracking
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MboStats {
    action_counts: HashMap<Action, u64>,
    conversion_errors: u64,
    processing_errors: u64,
    snapshot_flagged: u64,
    clear_events: u64,
    total_messages: u64,
}

impl MboStats {
    fn record_action(&mut self, action: Action) {
        *self.action_counts.entry(action).or_insert(0) += 1;
    }
}

impl fmt::Display for MboStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  Total MBO messages:    {}", self.total_messages)?;
        for action in &[
            Action::Add,
            Action::Cancel,
            Action::Modify,
            Action::Fill,
            Action::Clear,
            Action::Trade,
        ] {
            writeln!(
                f,
                "    {:>8}: {:>12}",
                format!("{action}"),
                self.action_counts.get(action).unwrap_or(&0)
            )?;
        }
        writeln!(f, "  SNAPSHOT-flagged msgs:  {}", self.snapshot_flagged)?;
        writeln!(f, "  Clear (book reset) events: {}", self.clear_events)?;
        writeln!(f, "  Conversion errors:     {}", self.conversion_errors)?;
        writeln!(f, "  Processing errors:     {}", self.processing_errors)
    }
}

#[derive(Default)]
struct ValidationStats {
    snapshots_checked: u64,
    full_matches: u64,
    partial_mismatches: u64,
    first_mismatch_snapshot: Option<u64>,
    level_bid_mismatches: u64,
    level_ask_mismatches: u64,
    mbp_action_counts: HashMap<char, u64>,
    mbp_snapshot_flagged: u64,
    ts_seq_misalignments: u64,
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

fn format_price(price: i64) -> String {
    if price == UNDEF_PRICE {
        return "UNDEF".to_string();
    }
    let dollars = price / 1_000_000_000;
    let frac = (price % 1_000_000_000).unsigned_abs();
    format!("{dollars}.{frac:09}")
}

struct LevelComparison {
    level: usize,
    bid_match: bool,
    ask_match: bool,
    our_bid: Option<OrderLevelSummary>,
    our_ask: Option<OrderLevelSummary>,
    expected: BidAskPair,
}

fn compare_levels(
    our_bids: &[OrderLevelSummary],
    our_asks: &[OrderLevelSummary],
    mbp10_levels: &[BidAskPair; 10],
) -> Vec<LevelComparison> {
    (0..10)
        .map(|i| {
            let expected = mbp10_levels[i].clone();
            let our_bid = our_bids.get(i).copied();
            let our_ask = our_asks.get(i).copied();

            let bid_match = match our_bid {
                Some(b) => {
                    b.price == expected.bid_px
                        && b.total_quantity == expected.bid_sz as u64
                        && b.order_count == expected.bid_ct as usize
                }
                None => expected.bid_px == UNDEF_PRICE && expected.bid_sz == 0,
            };

            let ask_match = match our_ask {
                Some(a) => {
                    a.price == expected.ask_px
                        && a.total_quantity == expected.ask_sz as u64
                        && a.order_count == expected.ask_ct as usize
                }
                None => expected.ask_px == UNDEF_PRICE && expected.ask_sz == 0,
            };

            LevelComparison {
                level: i + 1,
                bid_match,
                ask_match,
                our_bid,
                our_ask,
                expected,
            }
        })
        .collect()
}

fn print_comparison(comparisons: &[LevelComparison]) {
    println!(
        "  {:<3} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | Status",
        "Lvl",
        "Our Bid Px",
        "Sz",
        "Ct",
        "Exp Bid Px",
        "Sz",
        "Ct",
        "Our Ask Px",
        "Sz",
        "Ct",
        "Exp Ask Px",
        "Sz",
        "Ct"
    );
    println!("  {}", "-".repeat(155));

    for c in comparisons {
        let (our_bid_px, our_bid_sz, our_bid_ct) = match c.our_bid {
            Some(b) => (
                format_price(b.price),
                b.total_quantity.to_string(),
                b.order_count.to_string(),
            ),
            None => ("EMPTY".to_string(), "-".to_string(), "-".to_string()),
        };
        let (our_ask_px, our_ask_sz, our_ask_ct) = match c.our_ask {
            Some(a) => (
                format_price(a.price),
                a.total_quantity.to_string(),
                a.order_count.to_string(),
            ),
            None => ("EMPTY".to_string(), "-".to_string(), "-".to_string()),
        };

        let status = match (c.bid_match, c.ask_match) {
            (true, true) => "OK",
            (false, true) => "BID MISS",
            (true, false) => "ASK MISS",
            (false, false) => "BOTH MISS",
        };

        println!(
            "  {:<3} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | {:>14} {:>7} {:>5} | {}",
            c.level,
            our_bid_px,
            our_bid_sz,
            our_bid_ct,
            format_price(c.expected.bid_px),
            c.expected.bid_sz,
            c.expected.bid_ct,
            our_ask_px,
            our_ask_sz,
            our_ask_ct,
            format_price(c.expected.ask_px),
            c.expected.ask_sz,
            c.expected.ask_ct,
            status,
        );
    }
}

fn is_snapshot(flag_set: FlagSet) -> bool {
    flag_set.raw() & flags::SNAPSHOT != 0
}

// ---------------------------------------------------------------------------
// Core comparison: compare our book against MBP-10 record
// ---------------------------------------------------------------------------

fn do_comparison(
    processor: &MboProcessor,
    mbp_record: &Mbp10Msg,
    mbo_record: Option<&MboMsg>,
    mbo_stats: &MboStats,
    val_stats: &mut ValidationStats,
    cli: &Cli,
) {
    val_stats.snapshots_checked += 1;

    let mbp_action = mbp_record.action as u8 as char;
    *val_stats.mbp_action_counts.entry(mbp_action).or_insert(0) += 1;
    if is_snapshot(mbp_record.flags) {
        val_stats.mbp_snapshot_flagged += 1;
    }

    // Sanity check: verify (ts_event, sequence) alignment
    if let Some(mbo) = mbo_record
        && (mbo.hd.ts_event != mbp_record.hd.ts_event || mbo.sequence != mbp_record.sequence)
    {
        val_stats.ts_seq_misalignments += 1;
        if val_stats.ts_seq_misalignments <= 5 {
            eprintln!(
                "  MISALIGNMENT at snapshot #{}: MBO(ts={}, seq={}, action={}) vs MBP(ts={}, seq={}, action='{}')",
                val_stats.snapshots_checked,
                mbo.hd.ts_event,
                mbo.sequence,
                mbo.action as u8 as char,
                mbp_record.hd.ts_event,
                mbp_record.sequence,
                mbp_action,
            );
        }
    }

    let mbp = MarketByPrice::from_top_n(processor.order_book(), 10);
    let top_bids = mbp.top_n_bids(10);
    let top_asks = mbp.top_n_asks(10);

    let comparisons = compare_levels(&top_bids, &top_asks, &mbp_record.levels);
    let all_match = comparisons.iter().all(|c| c.bid_match && c.ask_match);

    if all_match {
        val_stats.full_matches += 1;
    } else {
        val_stats.partial_mismatches += 1;
        if val_stats.first_mismatch_snapshot.is_none() {
            val_stats.first_mismatch_snapshot = Some(val_stats.snapshots_checked);
        }
        let bid_misses = comparisons.iter().filter(|c| !c.bid_match).count() as u64;
        let ask_misses = comparisons.iter().filter(|c| !c.ask_match).count() as u64;
        val_stats.level_bid_mismatches += bid_misses;
        val_stats.level_ask_mismatches += ask_misses;

        if cli.verbose_mismatch && val_stats.partial_mismatches <= cli.max_verbose as u64 {
            let mbo_info = mbo_record
                .map(|m| {
                    format!(
                        "MBO: ts={}, seq={}, action='{}'",
                        m.hd.ts_event, m.sequence, m.action as u8 as char
                    )
                })
                .unwrap_or_else(|| "MBO: (snapshot phase)".to_string());

            println!(
                "\n  MISMATCH at snapshot #{} ({} | MBP: ts={}, seq={}, action='{}', snapshot={})",
                val_stats.snapshots_checked,
                mbo_info,
                mbp_record.hd.ts_event,
                mbp_record.sequence,
                mbp_action,
                is_snapshot(mbp_record.flags),
            );
            println!(
                "  MBO messages processed: {}, book: {} bid levels, {} ask levels",
                mbo_stats.total_messages,
                mbp.bids.len(),
                mbp.asks.len()
            );
            print_comparison(&comparisons);
            println!();
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let start_time = Instant::now();

    // --- Open decoders ---
    let mut mbp_decoder = Decoder::new(DynReader::from_file(&cli.mbp_file)?)?;
    let mbp_metadata = mbp_decoder.metadata();
    println!("=== File Metadata ===");
    println!(
        "MBP-10: schema={:?}, dataset={}, symbols={:?}",
        mbp_metadata.schema, mbp_metadata.dataset, mbp_metadata.symbols
    );

    let mut mbo_decoder = Decoder::new(DynReader::from_file(&cli.mbo_file)?)?;
    let mbo_metadata = mbo_decoder.metadata();
    println!(
        "MBO:    schema={:?}, dataset={}, symbols={:?}",
        mbo_metadata.schema, mbo_metadata.dataset, mbo_metadata.symbols
    );
    println!();

    let mut processor = MboProcessor::new();
    let mut mbo_stats = MboStats::default();
    let mut val_stats = ValidationStats::default();
    let max_checks = if cli.max_checks == 0 {
        u64::MAX
    } else {
        cli.max_checks as u64
    };

    let mut mbo_exhausted = false;
    // Buffer for MBO records that overshoot the current MBP-10 target.
    // These belong to the next event and must not be processed until then.
    let mut pending_mbo: Option<MboMsg> = None;

    println!("=== Streaming Validation (MBP-10-driven) ===");

    // Helper: process one MBO record and update stats
    fn process_mbo_record(record: &MboMsg, processor: &mut MboProcessor, mbo_stats: &mut MboStats) {
        mbo_stats.total_messages += 1;
        if is_snapshot(record.flags) {
            mbo_stats.snapshot_flagged += 1;
        }
        match MarketByOrderMessage::try_from(record) {
            Ok(msg) => {
                mbo_stats.record_action(msg.action);
                if msg.action == Action::Clear {
                    mbo_stats.clear_events += 1;
                }
                if let Err(e) = processor.process_message(&msg) {
                    mbo_stats.processing_errors += 1;
                    if mbo_stats.processing_errors <= 5 {
                        eprintln!(
                            "  Processing error at msg #{} (action={}, order_id={}, size={}): {}",
                            mbo_stats.total_messages, msg.action, msg.order_id, msg.size, e
                        );
                    }
                }
            }
            Err(e) => {
                mbo_stats.conversion_errors += 1;
                if mbo_stats.conversion_errors <= 5 {
                    eprintln!(
                        "  Conversion error at msg #{}: {} (action={}, side={})",
                        mbo_stats.total_messages, e, record.action, record.side
                    );
                }
            }
        }
    }

    // Uniform MBP-10-driven loop.
    // For each MBP-10 record (including the SNAPSHOT), advance the MBO stream
    // until we match its (ts_event, sequence), processing all MBO records along
    // the way. The MBP-10 SNAPSHOT's (ts, seq) matches the last MBO SNAPSHOT
    // record, so the bootstrap happens naturally.
    //
    // IMPORTANT: When events at the target (ts, seq) have NO LAST flag (e.g.
    // Trade events with pattern T(-) → F(-)), the next MBO record will have a
    // *different* (ts, seq). We must detect this "overshoot" BEFORE processing
    // the record and buffer it for the next iteration.
    while val_stats.snapshots_checked < max_checks {
        // Read next MBP-10 target
        let mbp_record = match mbp_decoder.decode_record::<Mbp10Msg>()? {
            Some(r) => r.clone(),
            None => break,
        };

        let target_ts = mbp_record.hd.ts_event;
        let target_seq = mbp_record.sequence;
        let mbp_action = mbp_record.action as u8 as char;
        let mbp_is_snapshot = is_snapshot(mbp_record.flags);

        // Advance MBO stream until we find the specific MBO record that
        // corresponds to this MBP-10 record.
        //
        // Matching strategy:
        // - SNAPSHOT MBP-10: process all MBO records at (ts, seq) until LAST
        //   (handles the 1324+ snapshot Adds that share the same ts/seq)
        // - Non-snapshot MBP-10: match by (ts, seq) AND action type.
        //   Within a multi-message event (e.g. T→F→C at the same ts/seq),
        //   each MBP-10 record corresponds to exactly one MBO record with
        //   the same action. Intermediate records (like Fill) that don't
        //   generate MBP-10 are processed but don't trigger comparison.
        if !mbo_exhausted {
            loop {
                // Get next MBO record: from buffer or from decoder
                let record = if let Some(buffered) = pending_mbo.take() {
                    buffered
                } else {
                    match mbo_decoder.decode_record::<MboMsg>()? {
                        Some(r) => r.clone(),
                        None => {
                            mbo_exhausted = true;
                            break;
                        }
                    }
                };

                let ts = record.hd.ts_event;
                let seq = record.sequence;

                // Check for overshoot BEFORE processing — this record belongs
                // to a later event and must be buffered for the next iteration.
                if ts > target_ts || (ts == target_ts && seq > target_seq) {
                    pending_mbo = Some(record);
                    break;
                }

                process_mbo_record(&record, &mut processor, &mut mbo_stats);

                // Check if this MBO record is the one corresponding to the MBP-10
                if ts == target_ts && seq == target_seq {
                    if mbp_action == 'T' && !mbp_is_snapshot {
                        // Trade events can have multiple MBP-10 'T' records at
                        // the same (ts, seq), each corresponding to one MBO 'T'
                        // message. Match by action so that intermediate Fill
                        // records are processed but don't trigger comparison.
                        let mbo_action = record.action as u8 as char;
                        if mbo_action == 'T' {
                            break;
                        }
                    } else {
                        // For A/C/M and snapshots: there is one MBP-10 record
                        // per (ts, seq) event group. Process all MBO records at
                        // this (ts, seq) until we see the LAST flag.
                        if record.flags.raw() & flags::LAST != 0 {
                            break;
                        }
                    }
                }
            }
        }

        // Log snapshot phase
        if is_snapshot(mbp_record.flags) {
            println!(
                "  Snapshot: {} MBO records processed ({} SNAPSHOT-flagged)",
                mbo_stats.total_messages, mbo_stats.snapshot_flagged
            );
        }

        // Compare our book state with the MBP-10 target
        do_comparison(
            &processor,
            &mbp_record,
            None,
            &mbo_stats,
            &mut val_stats,
            &cli,
        );

        // Log snapshot result
        if is_snapshot(mbp_record.flags) {
            let status = if val_stats.full_matches > 0 {
                "MATCH"
            } else {
                "MISMATCH"
            };
            println!("  Snapshot comparison: {status}");
        }

        // Progress
        if (val_stats.snapshots_checked as usize).is_multiple_of(cli.progress_interval) {
            let elapsed = start_time.elapsed().as_secs_f64();
            let rate = val_stats.snapshots_checked as f64 / elapsed;
            println!(
                "  Progress: {} snapshots checked ({} match, {} mismatch) | {} MBO msgs | {:.0} snaps/sec | {:.1}s elapsed",
                val_stats.snapshots_checked,
                val_stats.full_matches,
                val_stats.partial_mismatches,
                mbo_stats.total_messages,
                rate,
                elapsed
            );
        }
    }

    let elapsed = start_time.elapsed();

    // --- Summary ---
    println!();
    println!("============================================================");
    println!("=== VALIDATION SUMMARY ===");
    println!("============================================================");
    println!();
    println!("--- MBO Message Statistics ---");
    print!("{mbo_stats}");
    println!();

    println!("--- MBP-10 Record Statistics ---");
    println!("  Total snapshots checked: {}", val_stats.snapshots_checked);
    println!(
        "  SNAPSHOT-flagged MBP-10 records: {}",
        val_stats.mbp_snapshot_flagged
    );
    println!("  MBP-10 action distribution:");
    let mut mbp_actions: Vec<_> = val_stats.mbp_action_counts.iter().collect();
    mbp_actions.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    for (action, count) in &mbp_actions {
        println!("    '{}': {}", action, count);
    }
    println!(
        "  (ts_event, sequence) misalignments: {}",
        val_stats.ts_seq_misalignments
    );
    println!();

    println!("--- Validation Results ---");
    println!("  Full matches (10/10):  {}", val_stats.full_matches);
    println!(
        "  Snapshots with mismatch: {}",
        val_stats.partial_mismatches
    );
    if val_stats.partial_mismatches > 0 {
        println!(
            "  First mismatch at snapshot #{}",
            val_stats.first_mismatch_snapshot.unwrap_or(0)
        );
        println!(
            "  Total bid level mismatches: {}",
            val_stats.level_bid_mismatches
        );
        println!(
            "  Total ask level mismatches: {}",
            val_stats.level_ask_mismatches
        );
    }
    let match_pct = if val_stats.snapshots_checked > 0 {
        val_stats.full_matches as f64 / val_stats.snapshots_checked as f64 * 100.0
    } else {
        0.0
    };
    println!(
        "  Match rate: {:.4}% ({}/{})",
        match_pct, val_stats.full_matches, val_stats.snapshots_checked
    );
    println!();

    println!("--- Coverage Assessment ---");
    let has_add = mbo_stats.action_counts.contains_key(&Action::Add);
    let has_cancel = mbo_stats.action_counts.contains_key(&Action::Cancel);
    let has_modify = mbo_stats.action_counts.contains_key(&Action::Modify);
    let has_fill = mbo_stats.action_counts.contains_key(&Action::Fill);
    let has_clear = mbo_stats.action_counts.contains_key(&Action::Clear);
    let has_trade = mbo_stats.action_counts.contains_key(&Action::Trade);

    println!(
        "  Add:    {} ({})",
        if has_add { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Add).unwrap_or(&0)
    );
    println!(
        "  Cancel: {} ({})",
        if has_cancel { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Cancel).unwrap_or(&0)
    );
    println!(
        "  Modify: {} ({})",
        if has_modify { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Modify).unwrap_or(&0)
    );
    println!(
        "  Fill:   {} ({})",
        if has_fill { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Fill).unwrap_or(&0)
    );
    println!(
        "  Clear:  {} ({})",
        if has_clear { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Clear).unwrap_or(&0)
    );
    println!(
        "  Trade:  {} ({})",
        if has_trade { "EXERCISED" } else { "NOT SEEN" },
        mbo_stats.action_counts.get(&Action::Trade).unwrap_or(&0)
    );
    println!(
        "  Snapshot initialization (SNAPSHOT-flagged MBO): {} messages",
        mbo_stats.snapshot_flagged
    );
    println!("  Book resets (Clear events): {}", mbo_stats.clear_events);
    println!();

    println!(
        "Elapsed: {:.2}s ({:.0} MBO msgs/sec, {:.0} snapshots/sec)",
        elapsed.as_secs_f64(),
        mbo_stats.total_messages as f64 / elapsed.as_secs_f64(),
        val_stats.snapshots_checked as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
