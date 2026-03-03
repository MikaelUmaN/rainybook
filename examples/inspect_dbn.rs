//! Streaming inspection of a DBN / DBN.ZST file.
//!
//! Demonstrates that we can read metadata and stream records from a dbn store
//! without loading the entire file into memory.
//!
//! Usage:
//!   cargo run --example inspect_dbn -- --file path/to/data.dbn.zst
//!   cargo run --example inspect_dbn -- --file path/to/data.dbn.zst --records 5
//!   cargo run --example inspect_dbn -- --file path/to/data.dbn.zst --schema mbp-10

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use clap::Parser;
use dbn::{
    BboMsg, CbboMsg, Cmbp1Msg, ImbalanceMsg, InstrumentDefMsg, MboMsg, Mbp1Msg, Mbp10Msg, OhlcvMsg,
    Schema, StatMsg, StatusMsg, TradeMsg,
    decode::{DbnMetadata, DecodeRecord, DynReader, dbn::Decoder},
};

use rainybook::MarketByOrderMessage;

#[derive(Parser)]
#[command(name = "inspect_dbn")]
#[command(about = "Inspect a DBN/DBN.ZST file by streaming metadata and the first N records")]
struct Cli {
    /// Path to a .dbn or .dbn.zst file
    #[arg(short, long, value_name = "FILE")]
    #[arg(value_parser = clap::value_parser!(PathBuf))]
    file: PathBuf,

    /// Number of records to decode and display
    #[arg(short, long, default_value_t = 10)]
    records: usize,

    /// Schema to use for decoding (e.g. mbo, mbp-1, mbp-10, trades, tbbo, ohlcv-1s, etc.)
    /// If not specified, uses the schema from the file metadata.
    #[arg(long)]
    schema: Option<String>,
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    match bytes {
        b if b >= GB => format!("{:.2} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.2} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.2} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

/// Decode and print up to `max` records of the given type.
/// Returns `(count, exhausted)`.
macro_rules! decode_and_print {
    ($decoder:expr, $record_type:ty, $max:expr) => {{
        let mut count = 0usize;
        let mut exhausted = false;
        while count < $max {
            match $decoder.decode_record::<$record_type>()? {
                Some(record) => {
                    count += 1;
                    println!("  [{count}] {record:?}");
                }
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        (count, exhausted)
    }};
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // --- File info ---
    let file_meta = fs::metadata(&cli.file)?;
    let file_size = file_meta.len();

    println!("=== File Info ===");
    println!("  Path:  {}", cli.file.display());
    println!("  Size:  {} ({} bytes)", format_bytes(file_size), file_size);
    println!();

    // --- Decode metadata (does NOT load records into memory) ---
    let mut decoder = Decoder::new(DynReader::from_file(&cli.file)?)?;
    let metadata = decoder.metadata();

    println!("=== DBN Metadata ===");
    println!("  Version:    {}", metadata.version);
    println!("  Dataset:    {}", metadata.dataset);
    println!(
        "  Schema:     {}",
        metadata
            .schema
            .map_or_else(|| "None".to_string(), |s| format!("{s:?}"))
    );
    println!(
        "  Stype In:   {}",
        metadata
            .stype_in
            .map_or_else(|| "None".to_string(), |s| format!("{s:?}"))
    );
    println!("  Stype Out:  {:?}", metadata.stype_out);
    println!("  Start:      {}", metadata.start());
    println!(
        "  End:        {}",
        metadata
            .end()
            .map_or_else(|| "None".to_string(), |dt| dt.to_string())
    );
    println!(
        "  Limit:      {}",
        metadata
            .limit
            .map_or_else(|| "None".to_string(), |l| l.to_string())
    );
    println!("  Ts Out:     {}", metadata.ts_out);
    println!("  Symbol Len: {} bytes", metadata.symbol_cstr_len);

    if !metadata.symbols.is_empty() {
        println!("  Symbols:    [{}]", metadata.symbols.join(", "));
    } else {
        println!("  Symbols:    (none)");
    }
    if !metadata.partial.is_empty() {
        println!("  Partial:    [{}]", metadata.partial.join(", "));
    }
    if !metadata.not_found.is_empty() {
        println!("  Not Found:  [{}]", metadata.not_found.join(", "));
    }
    if !metadata.mappings.is_empty() {
        println!("  Mappings:   {} entries", metadata.mappings.len());
    }

    // Extract schema before mutable borrow of decoder
    let file_schema = metadata.schema;
    println!();

    // --- Determine schema ---
    let schema = match &cli.schema {
        Some(s) => s.parse::<Schema>()?,
        None => file_schema.ok_or("No schema in file metadata; please specify --schema")?,
    };

    // --- Stream first N records ---
    println!("=== First {} Record(s) [schema: {schema}] ===", cli.records);

    // Future proof against new schemas.
    #[allow(unreachable_patterns)]
    let (count, exhausted) = match schema {
        Schema::Mbo => {
            let mut count = 0usize;
            let mut exhausted = false;
            while count < cli.records {
                match decoder.decode_record::<MboMsg>()? {
                    Some(record) => {
                        count += 1;
                        match MarketByOrderMessage::try_from(record) {
                            Ok(msg) => println!("  [{count}] {msg:?}"),
                            Err(e) => println!("  [{count}] (conversion error: {e})"),
                        }
                    }
                    None => {
                        exhausted = true;
                        break;
                    }
                }
            }
            (count, exhausted)
        }
        Schema::Mbp1 | Schema::Tbbo => decode_and_print!(decoder, Mbp1Msg, cli.records),
        Schema::Mbp10 => decode_and_print!(decoder, Mbp10Msg, cli.records),
        Schema::Trades => decode_and_print!(decoder, TradeMsg, cli.records),
        Schema::Ohlcv1S
        | Schema::Ohlcv1M
        | Schema::Ohlcv1H
        | Schema::Ohlcv1D
        | Schema::OhlcvEod => decode_and_print!(decoder, OhlcvMsg, cli.records),
        Schema::Definition => decode_and_print!(decoder, InstrumentDefMsg, cli.records),
        Schema::Statistics => decode_and_print!(decoder, StatMsg, cli.records),
        Schema::Status => decode_and_print!(decoder, StatusMsg, cli.records),
        Schema::Imbalance => decode_and_print!(decoder, ImbalanceMsg, cli.records),
        Schema::Cmbp1 | Schema::Tcbbo => decode_and_print!(decoder, Cmbp1Msg, cli.records),
        Schema::Cbbo1S | Schema::Cbbo1M => decode_and_print!(decoder, CbboMsg, cli.records),
        Schema::Bbo1S | Schema::Bbo1M => decode_and_print!(decoder, BboMsg, cli.records),
        _ => {
            println!("  Unknown message schema encountered: {schema:?}");
            (0, false)
        }
    };

    println!();
    if exhausted {
        println!("Stream exhausted after {count} record(s) (file fully read).");
    } else if count > 0 {
        println!(
            "Displayed {count} record(s). Stream still has more data (not loaded into memory)."
        );
    }

    Ok(())
}
