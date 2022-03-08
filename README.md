# banking_transactions

A simple toy payments engine, written in rust

## Overview

This payment engine reads a series of transactions from an input CSV file,
updates a ledger of client accounts, reconciles disputes and chargebacks, and
then outputs the state of clients accounts as a CSV to stdout.

## Quick Start

Run Tests
```shell
cargo test
```

Run Linter
```shell
cargo clippy
```

Verify compilation
```shell
cargo check
```

Run Application
```shell
cargo run -- data/transactions_basic.csv > output.csv
```

Run Benchmarks
```shell
cargo bench
```

Release Build
```shell
cargo build --release
```

## Sample Data

Sample input data can be found in the [data  directory](./data). The smaller 
datasets were all written by hand, the two larger datasets were generated using
a simple python script that kept the transaction ids incrementing.

These csv files are used by the [benchmark tests](./benches/transaction_benches.rs),
whereas the unit tests for the transaction engine all have their own 
hand-crafted transactions that are used to verify specific pieces of behavior.

## Safety

No unsafe code is used in this project and all errors are properly handled. 
Each operation that can be performed by this utility come with their own set of
error types, defined in [errors.rs](./src/errors.rs), so that the exact cause of
any issues is made available to upstream callers.

## Efficiency

The Ledger struct ingests data using `io::BufReader`, albeit indirectly, so it 
processes a stream of CSV records as they're read, allowing for some level of 
efficiency. However, the Ledger then maintains all of it's state in memory 
which is less than ideal for production-like workloads. For handling concurrent
requests at the scale of a production system a proper database would ideally
be leveraged to mitigate this issue.
