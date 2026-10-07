use consumer::events::{Event, ExperimentConfigured, SensorTemperatureMeasured, StageChange};
use postgres::Client;

pub struct Store {
    db: Client,
}

impl Store {
    pub fn new(db: Client) -> Self {
        Store { db }
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
            Err(error) => eprintln!("DB error (experiment_configured): {error}"),
        }
    }

    fn save_stabilization_started(&mut self, stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'stabilizing', stabilizing_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored stabilization_started"),
            Err(error) => eprintln!("DB error (stabilization_started): {error}"),
        }
    }

    fn save_experiment_started(&mut self, stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'carrying_out', started_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored experiment_started"),
            Err(error) => eprintln!("DB error (experiment_started): {error}"),
        }
    }

    fn save_sensor_temperature_measured(&mut self, reading: SensorTemperatureMeasured) {
        let thresholds = self.db.query_one(
            "SELECT lower_threshold, upper_threshold
             FROM experiments
             WHERE experiment_id = $1",
            &[&reading.experiment],
        );
        let thresholds = match thresholds {
            Ok(row) => (row.get::<_, f64>(0), row.get::<_, f64>(1)),
            Err(error) => {
                eprintln!(
                    "DB error (lookup experiment {} for measurement {}): {error}",
                    reading.experiment, reading.measurement_id
                );
                return;
            }
        };
        let temperature = f64::from(reading.temperature);
        let out_of_range = temperature < thresholds.0 || temperature > thresholds.1;
        let result = self.db.execute(
            "INSERT INTO measurements
                (measurement_id, experiment_id, timestamp, temperature, out_of_range)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (measurement_id) DO NOTHING",
            &[
                &reading.measurement_id,
                &reading.experiment,
                &reading.timestamp,
                &temperature,
                &out_of_range,
            ],
        );
        match result {
            Ok(_) => println!("stored sensor_temperature_measured"),
            Err(error) => eprintln!("DB error (sensor_temperature_measured): {error}"),
        }
    }

    fn save_experiment_terminated(&mut self, stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments
             SET phase = 'terminated', terminated_at = $1
             WHERE experiment_id = $2",
            &[&stage.timestamp, &stage.experiment],
        );
        match result {
            Ok(_) => println!("stored experiment_terminated"),
            Err(error) => eprintln!("DB error (experiment_terminated): {error}"),
        }
    }
}
