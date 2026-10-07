mod notifier;

use clap::Parser;
use consumer::kafka::{KafkaArgs, KafkaMetrics, consume};
use notifier::{Notifier, NotifierMetrics};
use prometheus::{Encoder, TextEncoder};
use tiny_http::{Header, Response, Server};
use std::sync::Arc;

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
    let notifier_metrics = Arc::new(NotifierMetrics::new());
    let kafka_metrics = KafkaMetrics::new();
    let metrics_server = Server::http("0.0.0.0:9091")
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::thread::spawn(move || {
        for request in metrics_server.incoming_requests() {
            let mut body = Vec::new();
            if let Err(error) = TextEncoder::new().encode(&prometheus::gather(), &mut body) {
                eprintln!("Metrics encoding error: {error}");
                let _ = request.respond(Response::from_string("metrics encoding failed").with_status_code(500));
                continue;
            }
            let response = Response::from_data(body).with_header(Header::from_bytes(
                b"Content-Type",
                b"text/plain; version=0.0.4",
            ).expect("valid metrics content type header"));
            let _ = request.respond(response);
        }
    });
    let mut notifier = Notifier::new(args.notifications_url, args.token, notifier_metrics);
    consume(&args.kafka, &kafka_metrics, |event| notifier.handle(event))
}
