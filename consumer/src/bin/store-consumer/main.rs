mod store;

use clap::Parser;
use consumer::kafka::{KafkaArgs, consume};
use store::Store;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    kafka: KafkaArgs,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut store = Store::new();
    consume(&args.kafka, |event| store.save(&event))
}
