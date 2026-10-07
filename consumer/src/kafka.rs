use crate::events::Event;
use clap::Args;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::message::Headers;
use rdkafka::Message;
use prometheus::{IntCounter, register_int_counter};
use std::error::Error;
use std::time::Duration;

const TUTORIAL_BROKERS: &str =
    "kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092";

#[derive(Args)]
pub struct KafkaArgs {
    topic: String,
    consumer_group: String,
    #[arg(short, long, default_value = TUTORIAL_BROKERS)]
    brokers: String,
}

pub struct KafkaMetrics {
    pub messages: IntCounter,
    pub decode_errors: IntCounter,
}

impl KafkaMetrics {
    pub fn new() -> Self {
        Self {
            messages: register_int_counter!(
                "kafka_messages_consumed_total",
                "Kafka messages successfully decoded and passed to the consumer"
            ).expect("register kafka_messages_consumed_total"),
            decode_errors: register_int_counter!(
                "kafka_decode_errors_total",
                "Kafka messages that could not be decoded"
            ).expect("register kafka_decode_errors_total"),
        }
    }
}

pub fn consume(
    args: &KafkaArgs,
    metrics: &KafkaMetrics,
    mut handle: impl FnMut(Event),
) -> Result<(), Box<dyn Error>> {
    let consumer: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", &args.brokers)
        .set("group.id", &args.consumer_group)
        .set("auto.offset.reset", "latest")
        .set("enable.auto.commit", "true")
        .set("security.protocol", "ssl")
        .set("ssl.ca.location", "auth/ca.crt")
        .set("ssl.keystore.location", "auth/kafka.keystore.pkcs12")
        .set("ssl.keystore.password", "cc2023")
        .set("ssl.endpoint.identification.algorithm", "none")
        .create()?;

    consumer.subscribe(&[&args.topic])?;

    println!("Listening to topic: {}", args.topic);
    println!("Consumer group: {}", args.consumer_group);
    println!("Brokers: {}", args.brokers);

    loop {
        match consumer.poll(Duration::from_secs(1)) {
            Some(Ok(message)) => {
                let record_name = message
                    .headers()
                    .and_then(|headers| headers.iter().find(|header| header.key == "record_name"))
                    .and_then(|header| header.value)
                    .map(|value| String::from_utf8_lossy(value).into_owned())
                    .unwrap_or_default();

                match Event::decode(&record_name, message.payload().unwrap_or_default()) {
                    Ok(event) => {
                        metrics.messages.inc();
                        handle(event);
                    }
                    Err(error) => {
                        metrics.decode_errors.inc();
                        eprintln!("Decode error ({record_name}): {error}");
                    }
                }
            }
            Some(Err(error)) => eprintln!("Consumer error: {error}"),
            None => {}
        }
    }
}
