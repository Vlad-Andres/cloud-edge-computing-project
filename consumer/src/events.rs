use apache_avro::{Reader, from_value};
use serde::Deserialize;
use std::error::Error;

#[derive(Debug, Deserialize)]
pub struct ExperimentConfigured {
    pub experiment: String,
    pub researcher: String,
    pub sensors: Vec<String>,
    pub temperature_range: TemperatureRange,
}

#[derive(Debug, Deserialize)]
pub struct TemperatureRange {
    pub upper_threshold: f32,
    pub lower_threshold: f32,
}

#[derive(Debug, Deserialize)]
pub struct StageChange {
    pub experiment: String,
    pub timestamp: f64,
}

#[derive(Debug, Deserialize)]
pub struct SensorTemperatureMeasured {
    pub experiment: String,
    pub sensor: String,
    pub measurement_id: String,
    pub timestamp: f64,
    pub temperature: f32,
    pub measurement_hash: String,
}

#[derive(Debug)]
pub enum Event {
    ExperimentConfigured(ExperimentConfigured),
    StabilizationStarted(StageChange),
    ExperimentStarted(StageChange),
    SensorTemperatureMeasured(SensorTemperatureMeasured),
    ExperimentTerminated(StageChange),
}

impl Event {
    pub fn decode(record_name: &str, payload: &[u8]) -> Result<Event, Box<dyn Error>> {
        let value = Reader::new(payload)?.next().ok_or("empty payload")??;
        Ok(match record_name {
            "experiment_configured" => Event::ExperimentConfigured(from_value(&value)?),
            "stabilization_started" => Event::StabilizationStarted(from_value(&value)?),
            "experiment_started" => Event::ExperimentStarted(from_value(&value)?),
            "sensor_temperature_measured" => Event::SensorTemperatureMeasured(from_value(&value)?),
            "experiment_terminated" => Event::ExperimentTerminated(from_value(&value)?),
            _ => return Err(format!("unknown record_name `{record_name}`").into()),
        })
    }
}