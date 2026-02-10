use std::error::Error;
use std::path::PathBuf;

use clap::Parser;
use dbn::{
    MboMsg,
    decode::{DecodeRecord, DynReader, dbn::Decoder},
};
use tracing::{debug, info};

use rainybook::orderbook::{MarketByOrderMessage, MboProcessor};

#[derive(Parser)]
#[command(name = "rainybook")]
#[command(version, about = "Market-by-order processor and orderbook simulator")]
#[command(
    long_about = "Process market data and maintain an in-memory orderbook.\n\n\
    Supported data formats:\n  \
    - Databento Binary Encoding (DBN): .dbn, .dbn.zst"
)]
struct Cli {
    /// Path to the market data file
    #[arg(short, long, value_name = "FILE")]
    #[arg(help = "Input data file (supports .dbn, .dbn.zst formats)")]
    #[arg(value_parser = clap::value_parser!(PathBuf))]
    data_path: PathBuf,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    info!("Using data file: {}", cli.data_path.display());

    match cli.data_path.extension() {
        Some(ext) if ext == "dbn" || ext == "zst" => {
            info!("Processing Databento Binary Encoding (DBN) file...");
        }
        _ => {
            return Err("Data file must have extension .dbn or .dbn.zst".into());
        }
    }

    let mut decoder = Decoder::new(DynReader::from_file(&cli.data_path)?)?;
    let mut processor = MboProcessor::new();

    while let Some(record) = decoder.decode_record::<MboMsg>()? {
        let message = MarketByOrderMessage::try_from(record)?;
        debug!("Processing MBO message: {:?}", debug(&message));
        processor.process_message(&message)?;
    }

    Ok(())
}
