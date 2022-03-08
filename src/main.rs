use crate::args::Args;

mod args;
mod engine;

fn main() {
    let args = Args::parse();

    println!("would have read {}", args.csv_file);
}
