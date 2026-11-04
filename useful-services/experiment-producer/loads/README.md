# Load schedules

JSON does not support comments, so the descriptions for each schedule are kept here instead of being inserted into the `.json` files.

## `baseline.json`

Small check load: 2 sensors, 25 ms interval per sensor.

Expected rate: `2 * (1000 / 25) = 80 events/s`.

## `ramp.json`

Three experiments start at 0, 30, and 75 seconds. Their individual rates are 80, 64, and 48 events/s. They do not overlap, so the peak is 80 events/s.

## `burst.json`

Four experiments start at 0, 15, 30, and 45 seconds. Each produces 40 events/s. During overlaps, the combined rate is 80 events/s and briefly reaches 120 events/s from 45 to 55 seconds.

## `over-100.json`

One sustained load: 20 sensors, 100 ms interval per sensor.

Expected rate: `20 * (1000 / 100) = 200 events/s`, which is above the provider's 100 messages/s reference.

## `parallel.json`

32 experiments start together, each with 2 sensors at a 100 ms interval: `32 * 2 * (1000 / 100) = 640 events/s` for about 3 minutes (1800 samples).

Each experiment's events go to one partition (the key is the experiment ID), so the single-experiment profiles above only ever keep one consumer busy. 32 experiments spread over almost all 16 partitions, which is what autoscaling the consumers needs to make a difference.

## Formula

`sample_rate` is the delay between samples in milliseconds. It is not the rate in messages/second.

```text
rate per sensor = 1000 / sample_rate
rate per experiment = num_sensors * (1000 / sample_rate)
combined rate = sum of the rates of all experiments active at that time
```

Example:

```text
num_sensors = 20
sample_rate = 100 ms

rate = 20 * (1000 / 100)
     = 20 * 10
     = 200 events/second
```

The producer also emits configuration and lifecycle events, so the total Kafka message rate can be slightly higher than the sensor-event calculation. Grafana may display a smoother or lower rate because of scrape intervals and consumer processing time.
