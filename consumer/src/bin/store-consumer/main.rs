mod db;
mod store;

use clap::Parser;
use consumer::kafka::{KafkaArgs, KafkaMetrics, consume};
use db::DbArgs;
use prometheus::{Encoder, TextEncoder};
use std::sync::Arc;
use store::{Metrics, Store};
use tiny_http::{Header, Response, Server};

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
    let metrics = Arc::new(Metrics::new());
    let kafka_metrics = KafkaMetrics::new();
    let metrics_server = Server::http("0.0.0.0:9091")
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::thread::spawn(move || {
        for request in metrics_server.incoming_requests() {
            let metric_families = prometheus::gather();
            let mut body = Vec::new();
            if let Err(error) = TextEncoder::new().encode(&metric_families, &mut body) {
                eprintln!("Metrics encoding error: {error}");
                let _ = request.respond(Response::from_string("metrics encoding failed").with_status_code(500));
                continue;
            }
            let response = Response::from_data(body)
                .with_header(Header::from_bytes(
                    b"Content-Type",
                    b"text/plain; version=0.0.4",
                ).expect("valid metrics content type header"));
            let _ = request.respond(response);
        }
    });
    let mut store = Store::new(db, metrics);
    consume(&args.kafka, &kafka_metrics, |event| store.save(event))
}
