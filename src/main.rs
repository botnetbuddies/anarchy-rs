mod cli;
mod engine;
mod fmt;
mod hash;
mod names;
mod output;
mod util;

fn main() {
    std::process::exit(cli::run());
}
