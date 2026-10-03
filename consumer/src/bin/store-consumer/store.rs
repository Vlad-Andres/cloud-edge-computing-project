use consumer::events::{Event, ExperimentConfigured, SensorTemperatureMeasured, StageChange};

pub struct Store;

impl Store {
    // TODO: connect to Postgres with POSTGRES_* from .env once the database is ready
    pub fn new() -> Self {
        Store
    }

    pub fn save(&mut self, event: &Event) {
        match event {
            Event::ExperimentConfigured(configured) => self.save_experiment_configured(configured),
            Event::StabilizationStarted(stage) => self.save_stabilization_started(stage),
            Event::ExperimentStarted(stage) => self.save_experiment_started(stage),
            Event::SensorTemperatureMeasured(reading) => self.save_sensor_temperature_measured(reading),
            Event::ExperimentTerminated(stage) => self.save_experiment_terminated(stage),
        }
    }

    // TODO: implement once the table schema is known
    fn save_experiment_configured(&mut self, configured: &ExperimentConfigured) {
        println!("experiment_configured {configured:?}");
    }

    // TODO: implement once the table schema is known
    fn save_stabilization_started(&mut self, stage: &StageChange) {
        println!("stabilization_started {stage:?}");
    }

    // TODO: implement once the table schema is known
    fn save_experiment_started(&mut self, stage: &StageChange) {
        println!("experiment_started {stage:?}");
    }

    // TODO: implement once the table schema is known
    fn save_sensor_temperature_measured(&mut self, reading: &SensorTemperatureMeasured) {
        println!("sensor_temperature_measured {reading:?}");
    }

    // TODO: implement once the table schema is known
    fn save_experiment_terminated(&mut self, stage: &StageChange) {
        println!("experiment_terminated {stage:?}");
    }
}
