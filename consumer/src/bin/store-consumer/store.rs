use consumer::events::{Event, ExperimentConfigured, SensorTemperatureMeasured, StageChange};
use postgres::Client;
use prometheus::{HistogramVec, IntCounterVec, register_histogram_vec, register_int_counter_vec};
use std::sync::Arc;

pub struct Metrics {
    pub db_requests: IntCounterVec,
    pub db_errors: IntCounterVec,
    pub db_latency: HistogramVec,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            db_requests: register_int_counter_vec!(
                "store_db_requests_total",
                "Database requests issued by the store consumer",
                &["operation"]
            ).expect("register store_db_requests_total"),
            db_errors: register_int_counter_vec!(
                "store_db_errors_total",
                "Database requests that failed in the store consumer",
                &["operation"]
            ).expect("register store_db_errors_total"),
            db_latency: register_histogram_vec!(
                "store_db_request_duration_seconds",
                "Database request duration in seconds",
                &["operation"]
            ).expect("register store_db_request_duration_seconds"),
        }
    }
}

pub struct Store {
    db: Client,
    metrics: Arc<Metrics>,
}

impl Store {
    pub fn new(db: Client, metrics: Arc<Metrics>) -> Self {
        Store { db, metrics }
    }

    pub fn save(&mut self, event: Event) {
        match event {
            Event::ExperimentConfigured(configured) => self.save_experiment_configured(configured),
            Event::StabilizationStarted(stage) => self.save_stabilization_started(stage),
            Event::ExperimentStarted(stage) => self.save_experiment_started(stage),
            Event::SensorTemperatureMeasured(reading) => self.save_sensor_temperature_measured(reading),
            Event::ExperimentTerminated(stage) => self.save_experiment_terminated(stage),
        }
    }

    fn save_experiment_configured(&mut self, configured: ExperimentConfigured) {
        let timer = self.db_timer("experiment_configured");
        let result = self.db.execute(
            "INSERT INTO experiments (experiment_id, researcher, sensor_count, lower_threshold, upper_threshold)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (experiment_id) DO NOTHING",
            &[
                &configured.experiment,
                &configured.researcher,
                &(configured.sensors.len() as i32),
                &f64::from(configured.temperature_range.lower_threshold),
                &f64::from(configured.temperature_range.upper_threshold),
            ],
        );
        match result {
            Ok(_) => println!("stored experiment_configured"),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["experiment_configured"]).inc();
                eprintln!("DB error (experiment_configured): {error}");
            }
        }
        timer.observe_duration();
    }

    fn save_stabilization_started(&mut self, stage: StageChange) {
        let timer = self.db_timer("stabilization_started");
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'stabilizing', stabilizing_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored stabilization_started"),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["stabilization_started"]).inc();
                eprintln!("DB error (stabilization_started): {error}");
            }
        }
        timer.observe_duration();
    }

    fn save_experiment_started(&mut self, stage: StageChange) {
        let timer = self.db_timer("experiment_started");
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'carrying_out', started_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored experiment_started"),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["experiment_started"]).inc();
                eprintln!("DB error (experiment_started): {error}");
            }
        }
        timer.observe_duration();
    }

    fn save_sensor_temperature_measured(&mut self, reading: SensorTemperatureMeasured) {
        let timer = self.db_timer("measurement");
        let temperature = f64::from(reading.temperature);
        let result = self.db.execute(
            "INSERT INTO measurements
                (measurement_id, experiment_id, timestamp, temperature, out_of_range)
             SELECT $1, experiment_id, $3, $4,
                    $4 < lower_threshold OR $4 > upper_threshold
             FROM experiments
             WHERE experiment_id = $2
             ON CONFLICT (measurement_id) DO NOTHING",
            &[
                &reading.measurement_id,
                &reading.experiment,
                &reading.timestamp,
                &temperature,
            ],
        );
        match result {
            Ok(1) => println!("stored sensor_temperature_measured"),
            Ok(0) => println!(
                "skipped sensor_temperature_measured (duplicate or unknown experiment: {})",
                reading.experiment
            ),
            Ok(rows) => println!(
                "stored sensor_temperature_measured ({rows} rows affected)"
            ),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["measurement"]).inc();
                eprintln!("DB error (sensor_temperature_measured): {error}");
            }
        }
        timer.observe_duration();
    }

    fn save_experiment_terminated(&mut self, stage: StageChange) {
        let timer = self.db_timer("experiment_terminated");
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'terminated', terminated_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored experiment_terminated"),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["experiment_terminated"]).inc();
                eprintln!("DB error (experiment_terminated): {error}");
            }
        }
        timer.observe_duration();
    }

    fn db_timer(&self, operation: &'static str) -> prometheus::HistogramTimer {
        self.metrics.db_requests.with_label_values(&[operation]).inc();
        self.metrics.db_latency.with_label_values(&[operation]).start_timer()
    }
}
