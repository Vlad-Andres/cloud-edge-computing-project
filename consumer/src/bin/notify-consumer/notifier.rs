use consumer::events::{Event, SensorTemperatureMeasured};
use serde::Serialize;
use std::collections::HashMap;
use std::thread;
use std::sync::Arc;
use std::time::Duration;
use prometheus::{HistogramVec, register_histogram_vec};
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
    readings: HashMap<String, (f64, usize)>,
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
}

impl NotifierMetrics {
    pub fn new() -> Self {
        Self {
            service_latency: register_histogram_vec!(
                "notify_service_latency_seconds",
                "Notification service reported latency in seconds",
                &["notification_type"]
            ).expect("register notify_service_latency_seconds"),
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
    }

    fn start_stage(&mut self, experiment: &str, carrying_out: bool) {
        if let Some(experiment) = self.experiments.get_mut(experiment) {
            experiment.carrying_out = carrying_out;
            experiment.previous_in_range = None;
        }
    }

    fn add_reading(&mut self, reading: SensorTemperatureMeasured) -> Option<Notification> {
        let experiment = self.experiments.get_mut(&reading.experiment)?;
        let (sum, count) = experiment
            .readings
            .entry(reading.measurement_id.clone())
            .or_insert((0.0, 0));
        *sum += f64::from(reading.temperature);
        *count += 1;
        if *count < experiment.sensors {
            return None;
        }
        let (sum, count) = experiment.readings.remove(&reading.measurement_id)?;
        let temperature = sum / count as f64;
        let in_range =
            experiment.lower_threshold <= temperature && temperature <= experiment.upper_threshold;
        let previous_in_range = experiment.previous_in_range.replace(in_range);
        let notification_type = match (experiment.carrying_out, in_range) {
            (false, true) if previous_in_range != Some(true) => NotificationType::Stabilized,
            (true, false) if previous_in_range != Some(false) => NotificationType::OutOfRange,
            _ => return None,
        };
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
        thread::spawn(move || {
            let notification_type = format!("{:?}", notification.notification_type);
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
        });
    }
}
