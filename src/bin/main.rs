use banking_transactions::args::Args;
use banking_transactions::engine::Ledger;
use std::path::PathBuf;
use std::process;

fn main() {
    let args = Args::parse();

    let ledger = match Ledger::try_from(PathBuf::from(args.csv_file)) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("failed to process input file: {}", err);
            process::exit(1);
        }
    };

    let output = match ledger.generate_account_statements() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("failed to generate output report: {}", err);
            process::exit(1);
        }
    };
    println!("{}", output);
}
