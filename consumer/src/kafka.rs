use crate::events::Event;
use clap::Args;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::message::{Headers, Timestamp};
use rdkafka::Message;
use prometheus::{
    HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, register_histogram_vec,
    register_int_counter, register_int_counter_vec, register_int_gauge_vec,
};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TUTORIAL_BROKERS: &str =
    "kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092";

/// Buckets for time spent handling one event (e.g. one DB round-trip).
pub const HANDLE_BUCKETS: &[f64] = &[
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];
/// Buckets for event age; large values show up when a backlog builds.
pub const END_TO_END_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];
const WATERMARK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Args)]
pub struct KafkaArgs {
    topic: String,
    consumer_group: String,
    #[arg(short, long, default_value = TUTORIAL_BROKERS)]
    brokers: String,
}

pub struct KafkaMetrics {
    pub messages: IntCounter,
    pub messages_by_event: IntCounterVec,
    pub decode_errors: IntCounter,
    pub end_to_end: HistogramVec,
    pub handle_duration: HistogramVec,
    pub consumed_offset: IntGaugeVec,
    pub high_watermark: IntGaugeVec,
}

impl KafkaMetrics {
    pub fn new() -> Self {
        Self {
            messages: register_int_counter!(
                "kafka_messages_consumed_total",
                "Kafka messages successfully decoded and passed to the consumer"
            ).expect("register kafka_messages_consumed_total"),
            messages_by_event: register_int_counter_vec!(
                "kafka_events_consumed_total",
                "Kafka events successfully decoded by event type",
                &["event_type"]
            ).expect("register kafka_events_consumed_total"),
            decode_errors: register_int_counter!(
                "kafka_decode_errors_total",
                "Kafka messages that could not be decoded"
            ).expect("register kafka_decode_errors_total"),
            end_to_end: register_histogram_vec!(
                "kafka_event_end_to_end_seconds",
                "Time from the Kafka message timestamp until the consumer finished handling the event",
                &["event_type"],
                END_TO_END_BUCKETS.to_vec()
            ).expect("register kafka_event_end_to_end_seconds"),
            handle_duration: register_histogram_vec!(
                "kafka_event_handle_duration_seconds",
                "Time the consumer spent handling one decoded event",
                &["event_type"],
                HANDLE_BUCKETS.to_vec()
            ).expect("register kafka_event_handle_duration_seconds"),
            consumed_offset: register_int_gauge_vec!(
                "kafka_partition_consumed_offset",
                "Offset after the last message processed by this consumer, per partition",
                &["partition"]
            ).expect("register kafka_partition_consumed_offset"),
            high_watermark: register_int_gauge_vec!(
                "kafka_partition_high_watermark",
                "Latest offset available in Kafka, per partition",
                &["partition"]
            ).expect("register kafka_partition_high_watermark"),
        }
    }
}

pub fn consume(
    args: &KafkaArgs,
    metrics: &KafkaMetrics,
    mut handle: impl FnMut(Event),
) -> Result<(), Box<dyn Error>> {
    let mut config = ClientConfig::new();
    config
        .set("bootstrap.servers", &args.brokers)
        .set("group.id", &args.consumer_group)
        .set("auto.offset.reset", "latest")
        .set("enable.auto.commit", "true")
        // On a rebalance only the partitions that move are paused, not all of them
        .set("partition.assignment.strategy", "cooperative-sticky")
        .set("security.protocol", "ssl")
        .set("ssl.ca.location", "auth/ca.crt")
        .set("ssl.keystore.location", "auth/kafka.keystore.pkcs12")
        .set("ssl.keystore.password", "cc2023")
        .set("ssl.endpoint.identification.algorithm", "none");
    let consumer: BaseConsumer = config.create()?;

    consumer.subscribe(&[&args.topic])?;
    spawn_watermark_poller(config, args.topic.clone(), metrics.high_watermark.clone());

    println!("Listening to topic: {}", args.topic);
    println!("Consumer group: {}", args.consumer_group);
    println!("Brokers: {}", args.brokers);

    // On SIGTERM (scale-down, rollout) finish the current message and return:
    // dropping the consumer commits the offsets and leaves the group, so the
    // partitions are handed over at once instead of after session.timeout.ms.
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))?;

    while !stop.load(Ordering::Relaxed) {
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
                        metrics
                            .messages_by_event
                            .with_label_values(&[&record_name])
                            .inc();
                        let started = Instant::now();
                        handle(event);
                        metrics
                            .handle_duration
                            .with_label_values(&[&record_name])
                            .observe(started.elapsed().as_secs_f64());
                        if let Some(age) = seconds_since(message.timestamp()) {
                            metrics
                                .end_to_end
                                .with_label_values(&[&record_name])
                                .observe(age);
                        }
                    }
                    Err(error) => {
                        metrics.decode_errors.inc();
                        eprintln!("Decode error ({record_name}): {error}");
                    }
                }
                metrics
                    .consumed_offset
                    .with_label_values(&[&message.partition().to_string()])
                    .set(message.offset() + 1);
            }
            Some(Err(error)) => eprintln!("Consumer error: {error}"),
            None => {}
        }
    }
    println!("SIGTERM received, leaving the consumer group");
    Ok(())
}

fn seconds_since(timestamp: Timestamp) -> Option<f64> {
    let produced_ms = timestamp.to_millis()?;
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((now_ms - produced_ms).max(0) as f64 / 1000.0)
}

/// Polls the newest offset of every partition on a separate client, so the
/// main consume loop is never blocked by these broker round-trips.
fn spawn_watermark_poller(config: ClientConfig, topic: String, high_watermark: IntGaugeVec) {
    std::thread::spawn(move || {
        let client: BaseConsumer = match config.create() {
            Ok(client) => client,
            Err(error) => {
                eprintln!("Watermark client error: {error}");
                return;
            }
        };
        loop {
            match client.fetch_metadata(Some(topic.as_str()), WATERMARK_INTERVAL) {
                Ok(metadata) => {
                    for partition in metadata.topics().iter().flat_map(|entry| entry.partitions()) {
                        match client.fetch_watermarks(&topic, partition.id(), WATERMARK_INTERVAL) {
                            Ok((_, high)) => high_watermark
                                .with_label_values(&[&partition.id().to_string()])
                                .set(high),
                            Err(error) => eprintln!(
                                "Watermark error (partition {}): {error}",
                                partition.id()
                            ),
                        }
                    }
                }
                Err(error) => eprintln!("Metadata error: {error}"),
            }
            std::thread::sleep(WATERMARK_INTERVAL);
        }
    });
}
