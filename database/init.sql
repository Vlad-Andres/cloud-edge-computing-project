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
    measurement_id  TEXT PRIMARY KEY,
    experiment_id   TEXT NOT NULL REFERENCES experiments (experiment_id),
    timestamp       DOUBLE PRECISION NOT NULL,
    temperature     DOUBLE PRECISION NOT NULL,
    out_of_range    BOOLEAN NOT NULL
);

CREATE INDEX measurements_experiment_time_idx ON measurements (experiment_id, timestamp);
CREATE INDEX measurements_experiment_oor_idx ON measurements (experiment_id, out_of_range);
