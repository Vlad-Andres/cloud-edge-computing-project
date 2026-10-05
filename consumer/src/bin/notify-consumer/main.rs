mod notifier;

use clap::Parser;
use consumer::kafka::{KafkaArgs, consume};
use notifier::Notifier;

const NOTIFICATIONS_URL: &str = "https://notifications.cec.dlandau.nl/api/notify";

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    kafka: KafkaArgs,
    #[arg(long, env = "NOTIFICATIONS_TOKEN")]
    token: String,
    #[arg(long, default_value = NOTIFICATIONS_URL)]
    notifications_url: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut notifier = Notifier::new(args.notifications_url, args.token);
    consume(&args.kafka, |event| notifier.handle(event))
}
