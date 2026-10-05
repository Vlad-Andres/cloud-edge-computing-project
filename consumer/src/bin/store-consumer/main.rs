mod db;
mod store;

use clap::Parser;
use consumer::kafka::{KafkaArgs, consume};
use db::DbArgs;
use store::Store;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    kafka: KafkaArgs,
    #[command(flatten)]
    db: DbArgs,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let db = db::connect(&args.db)?;
    let mut store = Store::new(db);
    consume(&args.kafka, |event| store.save(event))
}
