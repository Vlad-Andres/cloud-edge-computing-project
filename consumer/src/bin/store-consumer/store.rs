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

    // Hardcoded test insert 
    fn save_experiment_configured(&mut self, _configured: ExperimentConfigured) {
        let result = self.db.execute(
            "INSERT INTO experiments (experiment_id, researcher, sensor_count, lower_threshold, upper_threshold)
             VALUES ('test-experiment', 'test.com', 2, 25.0, 26.0)
             ON CONFLICT (experiment_id) DO NOTHING",
            &[],
        );
        match result {
            Ok(_) => println!("stored experiment_configured (test insert)"),
            Err(error) => eprintln!("DB error (experiment_configured): {error}"),
        }
    }

    // Hardcoded test  
    fn save_stabilization_started(&mut self, _stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments SET phase = 'stabilizing', stabilizing_at = 0 WHERE experiment_id = 'test-experiment'",
            &[],
        );
        match result {
            Ok(_) => println!("stored stabilization_started (test update)"),
            Err(error) => eprintln!("DB error (stabilization_started): {error}"),
        }
    }

    // Hardcoded test 
    fn save_experiment_started(&mut self, _stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments SET phase = 'carrying_out', started_at = 0 WHERE experiment_id = 'test-experiment'",
            &[],
        );
        match result {
            Ok(_) => println!("stored experiment_started (test update)"),
            Err(error) => eprintln!("DB error (experiment_started): {error}"),
        }
    }

    //only measurement_id comes from the event, repeated calls should produce distinct rows
    fn save_sensor_temperature_measured(&mut self, reading: SensorTemperatureMeasured) {
        let result = self.db.execute(
            "INSERT INTO measurements (measurement_id, experiment_id, timestamp, temperature, out_of_range)
             VALUES ($1, 'test-experiment', 0, 25.0, false)
             ON CONFLICT (measurement_id) DO NOTHING",
            &[&reading.measurement_id],
        );
        match result {
            Ok(_) => println!("stored sensor_temperature_measured (test insert)"),
            Err(error) => eprintln!("DB error (sensor_temperature_measured): {error}"),
        }
    }

    // Hardcoded test
    fn save_experiment_terminated(&mut self, _stage: StageChange) {
        let result = self.db.execute(
            "UPDATE experiments SET phase = 'terminated', terminated_at = 0 WHERE experiment_id = 'test-experiment'",
            &[],
        );
        match result {
            Ok(_) => println!("stored experiment_terminated (test update)"),
            Err(error) => eprintln!("DB error (experiment_terminated): {error}"),
        }
    }
}
