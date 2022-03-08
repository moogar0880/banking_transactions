use clap::{App, Arg};

pub struct Args {
    pub csv_file: String
}

impl Args {
    pub fn parse() -> Self {
        let matches = App::new("bank")
            .version("0.1.0")
            .arg(Arg::with_name("csv_file")
                .takes_value(true).required(true).help("path of CSV file to read from"))
            .get_matches();

        Self {
            csv_file: matches.value_of("csv_file").unwrap_or_default().to_string(),
        }
    }
}
