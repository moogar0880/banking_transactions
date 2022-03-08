use crate::args::Args;
use crate::engine::{read_all, Ledger};
use std::process;

mod args;
mod engine;
mod errors;

fn main() {
    let args = Args::parse();

    // let mut reader = match TransactionReader::try_from(args.csv_file.as_str()) {
    //     Ok(reader) => reader,
    //     Err(err) => {
    //         eprintln!("failed to read input file: {}", err);
    //         return;
    //     }
    // };

    let data = match read_all(args.csv_file.as_str()) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("failed to read input file: {}", err);
            process::exit(1);
        }
    };

    let mut ledger = Ledger::new();

    if let Err(err) = ledger.process_transactions(data) {
        eprintln!("failed to process transactions: {}", err);
        process::exit(1);
    }

    // for entry in data {
    //     println!("{:?}", entry);
    // }

    // println!("would have read {}", args.csv_file);
}
