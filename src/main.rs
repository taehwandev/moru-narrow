use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(err) = moru_narrow::run_cli(&args) {
        eprintln!("{err}");
        process::exit(1);
    }
}
