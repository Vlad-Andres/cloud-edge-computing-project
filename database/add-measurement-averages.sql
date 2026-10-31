-- Adds measurement_averages to a running database and fills it from measurements.
-- Safe to run again: it only adds the measurements that are missing.
CREATE TABLE IF NOT EXISTS measurement_averages (
    experiment_id   TEXT NOT NULL REFERENCES experiments (experiment_id),
    timestamp       DOUBLE PRECISION NOT NULL,
    measurement_id  TEXT NOT NULL,
    temperature     DOUBLE PRECISION NOT NULL,
    out_of_range    BOOLEAN NOT NULL,
    PRIMARY KEY (experiment_id, timestamp, measurement_id) INCLUDE (temperature, out_of_range)
);

INSERT INTO measurement_averages (experiment_id, timestamp, measurement_id, temperature, out_of_range)
SELECT m.experiment_id, MIN(m.timestamp), m.measurement_id, AVG(m.temperature),
       AVG(m.temperature) NOT BETWEEN e.lower_threshold AND e.upper_threshold
FROM measurements m
JOIN experiments e ON e.experiment_id = m.experiment_id
GROUP BY m.experiment_id, m.measurement_id, e.lower_threshold, e.upper_threshold, e.sensor_count
HAVING COUNT(*) = e.sensor_count
ON CONFLICT DO NOTHING;

VACUUM (ANALYZE) measurement_averages;
