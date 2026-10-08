use consumer::events::{Event, SensorTemperatureMeasured};
use consumer::kafka::END_TO_END_BUCKETS;
use serde::Serialize;
use std::collections::HashMap;
use std::thread;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use prometheus::{
    Histogram, HistogramVec, IntGauge, register_histogram, register_histogram_vec,
    register_int_gauge,
};
use ureq::Agent;

#[derive(Debug, Serialize)]
enum NotificationType {
    Stabilized,
    OutOfRange,
}

#[derive(Serialize)]
struct Notification {
    notification_type: NotificationType,
    researcher: String,
    experiment_id: String,
    measurement_id: String,
    cipher_data: String,
}

struct Experiment {
    researcher: String,
    sensors: usize,
    lower_threshold: f64,
    upper_threshold: f64,
    carrying_out: bool,
    previous_in_range: Option<bool>,
    /// measurement_id -> (sum of temperatures, sensors seen, when the first sensor arrived)
    readings: HashMap<String, (f64, usize, Instant)>,
}

pub struct Notifier {
    agent: Agent,
    url: String,
    token: String,
    experiments: HashMap<String, Experiment>,
    metrics: Arc<NotifierMetrics>,
}

pub struct NotifierMetrics {
    pub service_latency: HistogramVec,
    pub request_duration: HistogramVec,
    pub inflight_requests: IntGauge,
    pub experiments: IntGauge,
    pub pending_measurements: IntGauge,
    pub hold_duration: Histogram,
    pub measurement_age: Histogram,
}

impl NotifierMetrics {
    pub fn new() -> Self {
        Self {
            service_latency: register_histogram_vec!(
                "notify_service_latency_seconds",
                "Notification service reported latency in seconds",
                &["notification_type"]
            ).expect("register notify_service_latency_seconds"),
            request_duration: register_histogram_vec!(
                "notify_request_duration_seconds",
                "Round-trip time of one POST to the notification service, including failed ones",
                &["notification_type"],
                END_TO_END_BUCKETS.to_vec()
            ).expect("register notify_request_duration_seconds"),
            inflight_requests: register_int_gauge!(
                "notify_inflight_requests",
                "Notifications sent to the notification service that have not been answered yet"
            ).expect("register notify_inflight_requests"),
            experiments: register_int_gauge!(
                "notify_experiments_tracked",
                "Experiments kept in memory (configured and not terminated yet)"
            ).expect("register notify_experiments_tracked"),
            pending_measurements: register_int_gauge!(
                "notify_pending_measurements",
                "Measurements kept in memory while waiting for their remaining sensor readings"
            ).expect("register notify_pending_measurements"),
            hold_duration: register_histogram!(
                "notify_measurement_hold_seconds",
                "Time a measurement waits in memory, from its first to its last sensor reading",
                END_TO_END_BUCKETS.to_vec()
            ).expect("register notify_measurement_hold_seconds"),
            measurement_age: register_histogram!(
                "notify_measurement_age_seconds",
                "Age of the measurement (now minus its timestamp) when its notification is sent",
                END_TO_END_BUCKETS.to_vec()
            ).expect("register notify_measurement_age_seconds"),
        }
    }
}

impl Notifier {
    pub fn new(url: String, token: String, metrics: Arc<NotifierMetrics>) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        Notifier {
            agent,
            url,
            token,
            experiments: HashMap::new(),
            metrics,
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::ExperimentConfigured(configured) => {
                self.experiments.insert(
                    configured.experiment,
                    Experiment {
                        researcher: configured.researcher,
                        sensors: configured.sensors.len(),
                        lower_threshold: configured.temperature_range.lower_threshold.into(),
                        upper_threshold: configured.temperature_range.upper_threshold.into(),
                        carrying_out: false,
                        previous_in_range: None,
                        readings: HashMap::new(),
                    },
                );
            }
            Event::StabilizationStarted(stage) => self.start_stage(&stage.experiment, false),
            Event::ExperimentStarted(stage) => self.start_stage(&stage.experiment, true),
            Event::SensorTemperatureMeasured(reading) => {
                if let Some(notification) = self.add_reading(reading) {
                    self.send(notification);
                }
            }
            Event::ExperimentTerminated(stage) => {
                self.experiments.remove(&stage.experiment);
            }
        }
        self.metrics.experiments.set(self.experiments.len() as i64);
        self.metrics.pending_measurements.set(
            self.experiments
                .values()
                .map(|experiment| experiment.readings.len() as i64)
                .sum(),
        );
    }

    fn start_stage(&mut self, experiment: &str, carrying_out: bool) {
        if let Some(experiment) = self.experiments.get_mut(experiment) {
            experiment.carrying_out = carrying_out;
            experiment.previous_in_range = None;
        }
    }

    fn add_reading(&mut self, reading: SensorTemperatureMeasured) -> Option<Notification> {
        let experiment = self.experiments.get_mut(&reading.experiment)?;
        let (sum, count, _) = experiment
            .readings
            .entry(reading.measurement_id.clone())
            .or_insert((0.0, 0, Instant::now()));
        *sum += f64::from(reading.temperature);
        *count += 1;
        if *count < experiment.sensors {
            return None;
        }
        let (sum, count, first_seen) = experiment.readings.remove(&reading.measurement_id)?;
        self.metrics.hold_duration.observe(first_seen.elapsed().as_secs_f64());
        let temperature = sum / count as f64;
        let in_range =
            experiment.lower_threshold <= temperature && temperature <= experiment.upper_threshold;
        let previous_in_range = experiment.previous_in_range.replace(in_range);
        let notification_type = match (experiment.carrying_out, in_range) {
            (false, true) if previous_in_range != Some(true) => NotificationType::Stabilized,
            (true, false) if previous_in_range != Some(false) => NotificationType::OutOfRange,
            _ => return None,
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |now| now.as_secs_f64());
        self.metrics.measurement_age.observe((now - reading.timestamp).max(0.0));
        Some(Notification {
            notification_type,
            researcher: experiment.researcher.clone(),
            experiment_id: reading.experiment,
            measurement_id: reading.measurement_id,
            cipher_data: reading.measurement_hash,
        })
    }

    fn send(&self, notification: Notification) {
        let (agent, url, token, metrics) = (
            self.agent.clone(),
            self.url.clone(),
            self.token.clone(),
            Arc::clone(&self.metrics),
        );
        metrics.inflight_requests.inc();
        thread::spawn(move || {
            let notification_type = format!("{:?}", notification.notification_type);
            let started = Instant::now();
            match agent.post(&url).query("token", &token).send_json(&notification) {
                Ok(mut response) => {
                    let status = response.status();
                    let body = response.body_mut().read_to_string().unwrap_or_default();
                    println!(
                        "{:?} {} -> {} {}",
                        notification.notification_type,
                        notification.measurement_id,
                        status,
                        body
                    );
                    match body.trim().parse::<f64>() {
                        Ok(latency) => metrics
                            .service_latency
                            .with_label_values(&[&notification_type])
                            .observe(latency),
                        Err(error) => eprintln!(
                            "Invalid notification latency ({}, body `{body}`): {error}",
                            notification.measurement_id
                        ),
                    }
                }
                Err(error) => eprintln!("Notify error ({}): {error}", notification.measurement_id),
            }
            metrics
                .request_duration
                .with_label_values(&[&notification_type])
                .observe(started.elapsed().as_secs_f64());
            metrics.inflight_requests.dec();
        });
    }
}
