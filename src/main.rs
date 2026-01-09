use std::error::Error;
use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use dbn::{
    MboMsg,
    decode::{DecodeRecord, dbn::Decoder},
};
use polars::io::parquet::read::ParquetReader;
use polars::prelude::*;
use tracing::{debug, error, info};

use rainybook::orderbook::{MarketByOrderMessage, MboProcessor, into_mbo_messages};

#[derive(Parser)]
#[command(name = "rainybook")]
#[command(version, about = "Market-by-order processor and orderbook simulator")]
#[command(
    long_about = "Process market data and maintain an in-memory orderbook.\n\n\
    Supported data formats:\n  \
    - Databento Binary Encoding (DBN): .dbn, .dbn.zst\n  \
    - Parquet files: .parquet\n  \
    - Market-by-order (MBO) messages with actions: Add, Cancel, Modify, Fill, Clear, Trade"
)]
struct Cli {
    /// Path to the market data file
    #[arg(short, long, value_name = "FILE")]
    #[arg(help = "Input data file (supports .dbn, .dbn.zst, .parquet formats)")]
    #[arg(value_parser = clap::value_parser!(PathBuf))]
    data_path: PathBuf,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("Hello world!");

    let cli = Cli::parse();
    info!("Using data file: {}", cli.data_path.display());
    let file = File::open(&cli.data_path).expect("Failed to open parquet file");

    let messages = match cli.data_path.extension() {
        Some(ext) if ext == "dbn" || ext == "zst" => {
            info!("Processing Databento Binary Encoding (DBN) file...");
            let decoder = Decoder::new(file)?;
            // Note: currently decodes all records into memory; consistent flow with the parquet case.
            let records = decoder.decode_records::<MboMsg>()?;
            let mbo_messages = records
                .iter()
                .map(MarketByOrderMessage::try_from)
                .collect::<Result<_, _>>()?;
            Ok(mbo_messages)
        }
        Some(ext) if ext == "parquet" => {
            info!("Processing Parquet file...");
            let df = ParquetReader::new(file)
                .finish()
                .expect("Failed to parse DataFrame from parquet file");
            Ok(into_mbo_messages(&df).expect("Failed to convert DataFrame to MBO messages"))
        }
        _ => {
            error!("Data file must have extension .dbn, .dbn.zst, or .parquet");
            Err("Unsupported file format")
        }
    }?;

    let mut processor = MboProcessor::new();
    for message in &messages {
        debug!("Processing MBO message: {:?}", debug(message));
        processor
            .process_message(message)
            .expect("Failed to process MBO message");
    }

    Ok(())
}
