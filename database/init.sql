CREATE TABLE experiments (
    experiment_id    TEXT PRIMARY KEY,
    researcher       TEXT NOT NULL,
    sensor_count     INTEGER NOT NULL,
    lower_threshold  DOUBLE PRECISION NOT NULL,
    upper_threshold  DOUBLE PRECISION NOT NULL,
    phase            TEXT NOT NULL DEFAULT 'configured',
    -- 'configured, stabilizing, carrying_out, terminated
    configured_at    DOUBLE PRECISION,
    stabilizing_at   DOUBLE PRECISION,
    started_at       DOUBLE PRECISION,
    terminated_at    DOUBLE PRECISION
);

CREATE TABLE measurements (
    measurement_id  TEXT NOT NULL,
    sensor_id       TEXT NOT NULL,
    experiment_id   TEXT NOT NULL REFERENCES experiments (experiment_id),
    timestamp       DOUBLE PRECISION NOT NULL,
    temperature     DOUBLE PRECISION NOT NULL,
    -- one row per sensor reading
    PRIMARY KEY (measurement_id, sensor_id)
);

-- for the foreign key: deleting an experiment checks that no measurements are left
CREATE INDEX measurements_experiment_idx ON measurements (experiment_id);

CREATE TABLE measurement_averages (
    experiment_id   TEXT NOT NULL REFERENCES experiments (experiment_id),
    timestamp       DOUBLE PRECISION NOT NULL,
    measurement_id  TEXT NOT NULL,
    temperature     DOUBLE PRECISION NOT NULL,
    out_of_range    BOOLEAN NOT NULL,
    -- one row per measurement whose sensors have all reported; the REST API reads only
    -- this table, in key order and from the index alone
    PRIMARY KEY (experiment_id, timestamp, measurement_id) INCLUDE (temperature, out_of_range)
);
