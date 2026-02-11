//! Streaming inspection of a DBN / DBN.ZST file.
//!
//! Demonstrates that we can read metadata and stream records from a dbn store
//! without loading the entire file into memory.
//!
//! Usage:
//!   cargo run --example inspect_dbn -- --file path/to/data.dbn.zst
//!   cargo run --example inspect_dbn -- --file path/to/data.dbn.zst --records 5

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use clap::Parser;
use dbn::{
    MboMsg,
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
    println!();

    // --- Stream first N records ---
    println!("=== First {} Record(s) ===", cli.records);

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

    println!();
    if exhausted {
        println!("Stream exhausted after {count} record(s) (file fully read).");
    } else {
        println!(
            "Displayed {count} record(s). Stream still has more data (not loaded into memory)."
        );
    }

    Ok(())
}
