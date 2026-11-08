use consumer::events::{Event, ExperimentConfigured, SensorTemperatureMeasured, StageChange};
use consumer::kafka::{HANDLE_BUCKETS, Handler};
use postgres::{Client, Statement};
use prometheus::{HistogramVec, IntCounterVec, register_histogram_vec, register_int_counter_vec};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Readings are written in batches: one statement and one commit for up to
// MAX_BATCH readings instead of one each. A batch is written once it is full,
// once its first reading waited MAX_WAIT, when Kafka has nothing new, and before
// any other event, so the database still sees the events in Kafka's order.
const MAX_BATCH: usize = 500;
const MAX_WAIT: Duration = Duration::from_millis(50);

// Stores a batch of sensor readings. The reading that completes a measurement also
// stores its average for the REST API, also when its other readings are in the same
// batch. It's one statement, so a replayed event can't count a sensor twice.
const INSERT_READINGS: &str = "WITH input AS (
         SELECT DISTINCT ON (measurement_id, sensor_id) *
         FROM UNNEST($1::text[], $2::text[], $3::text[], $4::float8[], $5::float8[])
             WITH ORDINALITY AS t(measurement_id, sensor_id, experiment_id, timestamp, temperature, ord)
         ORDER BY measurement_id, sensor_id, ord
     ), reading AS (
         INSERT INTO measurements
            (measurement_id, sensor_id, experiment_id, timestamp, temperature)
         SELECT input.measurement_id, input.sensor_id, input.experiment_id, input.timestamp, input.temperature
         FROM input
         JOIN experiments ON experiments.experiment_id = input.experiment_id
         ON CONFLICT (measurement_id, sensor_id) DO NOTHING
         RETURNING measurement_id, sensor_id
     ), batch AS (
         -- per measurement: its new readings, and the timestamp of the last one
         SELECT input.measurement_id, input.experiment_id,
                (ARRAY_AGG(input.timestamp ORDER BY input.ord DESC))[1] AS timestamp,
                SUM(input.temperature) AS temperature_sum, COUNT(*) AS readings
         FROM reading
         JOIN input ON input.measurement_id = reading.measurement_id
                   AND input.sensor_id = reading.sensor_id
         GROUP BY input.measurement_id, input.experiment_id
     ), average AS (
         INSERT INTO measurement_averages
            (experiment_id, timestamp, measurement_id, temperature, out_of_range)
         SELECT e.experiment_id, b.timestamp, b.measurement_id, m.temperature,
                m.temperature NOT BETWEEN e.lower_threshold AND e.upper_threshold
         FROM batch b
         JOIN experiments e ON e.experiment_id = b.experiment_id
         CROSS JOIN LATERAL (
             -- the statement can't see its own readings in measurements yet
             SELECT (COALESCE(SUM(temperature), 0) + b.temperature_sum) / (COUNT(*) + b.readings) AS temperature,
                    COUNT(*) + b.readings AS sensors_reported
             FROM measurements
             WHERE measurement_id = b.measurement_id
         ) m
         WHERE m.sensors_reported = e.sensor_count
     )
     SELECT COUNT(*) FROM reading";

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
                &["operation"],
                HANDLE_BUCKETS.to_vec()
            ).expect("register store_db_request_duration_seconds"),
        }
    }
}

pub struct Store {
    db: Client,
    metrics: Arc<Metrics>,
    insert_readings: Statement,
    readings: Vec<SensorTemperatureMeasured>,
    batch_started: Instant,
}

impl Store {
    pub fn new(mut db: Client, metrics: Arc<Metrics>) -> Result<Self, postgres::Error> {
        let insert_readings = db.prepare(INSERT_READINGS)?;
        Ok(Store {
            db,
            metrics,
            insert_readings,
            readings: Vec::with_capacity(MAX_BATCH),
            batch_started: Instant::now(),
        })
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

    fn add_reading(&mut self, reading: SensorTemperatureMeasured) {
        if self.readings.is_empty() {
            self.batch_started = Instant::now();
        }
        self.readings.push(reading);
        if self.readings.len() >= MAX_BATCH || self.batch_started.elapsed() >= MAX_WAIT {
            self.save_readings();
        }
    }

    fn save_readings(&mut self) {
        if self.readings.is_empty() {
            return;
        }
        let timer = self.db_timer("measurement_batch");
        let measurement_ids: Vec<&str> = self.readings.iter().map(|r| r.measurement_id.as_str()).collect();
        let sensors: Vec<&str> = self.readings.iter().map(|r| r.sensor.as_str()).collect();
        let experiments: Vec<&str> = self.readings.iter().map(|r| r.experiment.as_str()).collect();
        let timestamps: Vec<f64> = self.readings.iter().map(|r| r.timestamp).collect();
        let temperatures: Vec<f64> = self.readings.iter().map(|r| f64::from(r.temperature)).collect();
        let result = self.db.query_one(
            &self.insert_readings,
            &[&measurement_ids, &sensors, &experiments, &timestamps, &temperatures],
        );
        let count = self.readings.len();
        match result.map(|row| row.get::<_, i64>(0)) {
            Ok(stored) if stored as usize == count => {}
            Ok(stored) => println!(
                "skipped {} of {count} sensor_temperature_measured (duplicate or unknown experiment)",
                count - stored as usize
            ),
            Err(error) => {
                self.metrics.db_errors.with_label_values(&["measurement_batch"]).inc();
                eprintln!("DB error ({count} sensor_temperature_measured): {error}");
            }
        }
        timer.observe_duration();
        self.readings.clear();
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

impl Handler for Store {
    fn handle(&mut self, event: Event) {
        if !matches!(event, Event::SensorTemperatureMeasured(_)) {
            self.save_readings();
        }
        match event {
            Event::ExperimentConfigured(configured) => self.save_experiment_configured(configured),
            Event::StabilizationStarted(stage) => self.save_stabilization_started(stage),
            Event::ExperimentStarted(stage) => self.save_experiment_started(stage),
            Event::SensorTemperatureMeasured(reading) => self.add_reading(reading),
            Event::ExperimentTerminated(stage) => self.save_experiment_terminated(stage),
        }
    }

    fn idle(&mut self) {
        self.save_readings();
    }
}
