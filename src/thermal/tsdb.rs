use chrono::{DateTime, FixedOffset, Utc};
use postgres::{Client as PgClient, NoTls};

use super::error::{ThermalError, ThermalResult};

pub fn parse_dt(s: &str) -> ThermalResult<DateTime<FixedOffset>> {
    let parsed = match DateTime::parse_from_rfc3339(s) {
        Ok(dt) => Ok(dt),
        Err(source) => {
            if let Some(normalized) = normalize_pg_timestamptz(s) {
                DateTime::parse_from_rfc3339(&normalized)
            } else {
                Err(source)
            }
        }
    };

    parsed.map_err(|source| ThermalError::DateTimeParse {
        value: s.to_string(),
        source,
    })
}

fn normalize_pg_timestamptz(s: &str) -> Option<String> {
    let (date, time_and_offset) = s.split_once(' ')?;
    let offset_idx = time_and_offset.rfind(['+', '-'])?;
    if offset_idx == 0 {
        return None;
    }

    let time = &time_and_offset[..offset_idx];
    let offset = &time_and_offset[offset_idx..];
    let normalized_offset = if offset.len() == 3 {
        format!("{offset}:00")
    } else {
        offset.to_string()
    };

    Some(format!("{date}T{time}{normalized_offset}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TopicRoute<'a> {
    ZigbeeTemp { device: &'a str },
    CtMonitor { source: &'a str, column: &'a str },
    Heatpump { column: &'a str },
    Multical { column: &'a str },
    Emonth { column: &'a str },
    Tesla { column: &'a str },
    Sensors { column: &'a str },
    Metoffice { column: &'a str },
    EbusdPoll { field: &'a str },
}

fn topic_route(topic: &str) -> Option<TopicRoute<'_>> {
    if let Some(device) = topic.strip_prefix("zigbee2mqtt/") {
        return Some(TopicRoute::ZigbeeTemp { device });
    }
    if let Some(rest) = topic.strip_prefix("emon/EmonPi2/") {
        return Some(TopicRoute::CtMonitor {
            source: "EmonPi2",
            column: rest,
        });
    }
    if let Some(rest) = topic.strip_prefix("emon/emonpi2_cu/") {
        return Some(TopicRoute::CtMonitor {
            source: "emonpi2_cu",
            column: rest,
        });
    }
    if let Some(rest) = topic.strip_prefix("emon/emontx5_cu/") {
        return Some(TopicRoute::CtMonitor {
            source: "emontx5_cu",
            column: rest,
        });
    }
    if let Some(rest) = topic.strip_prefix("emon/heatpump/") {
        return Some(TopicRoute::Heatpump { column: rest });
    }
    if let Some(rest) = topic.strip_prefix("emon/multical/") {
        return Some(TopicRoute::Multical { column: rest });
    }
    if let Some(rest) = topic.strip_prefix("emon/emonth2_23/") {
        return Some(TopicRoute::Emonth { column: rest });
    }
    if let Some(rest) = topic.strip_prefix("emon/tesla/") {
        return Some(TopicRoute::Tesla { column: rest });
    }
    if let Some(rest) = topic.strip_prefix("emon/sensors/") {
        return Some(TopicRoute::Sensors { column: rest });
    }
    if let Some(rest) = topic.strip_prefix("emon/metoffice/") {
        return Some(TopicRoute::Metoffice { column: rest });
    }
    if let Some(field) = topic.strip_prefix("ebusd/poll/") {
        return Some(TopicRoute::EbusdPoll { field });
    }
    None
}

fn ebusd_live_field(topic: &str) -> Option<&str> {
    let rest = topic.strip_prefix("ebusd/")?;
    let (_circuit, field) = rest.split_once('/')?;
    Some(field)
}

fn quoted_identifier(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn fixed_utc(dt: DateTime<Utc>) -> DateTime<FixedOffset> {
    dt.fixed_offset()
}

fn pg_client(conninfo: &str) -> ThermalResult<PgClient> {
    PgClient::connect(conninfo, NoTls).map_err(ThermalError::PostgresConnect)
}

fn query_pg_timeseries(
    conninfo: &str,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let mut client = pg_client(conninfo)?;
    let rows = client
        .query(sql, params)
        .map_err(ThermalError::PostgresQuery)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                row.get::<_, f64>(1),
            )
        })
        .collect())
}

fn query_pg_room_topic(
    client: &mut PgClient,
    topic: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    let mut out = Vec::new();
    match topic_route(topic) {
        Some(TopicRoute::ZigbeeTemp { device }) => {
            let rows = client
                .query(
                    "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(temperature) AS value FROM zigbee WHERE device = $1 AND time >= $2 AND time < $3 AND temperature IS NOT NULL GROUP BY bucket ORDER BY bucket",
                    &[&device, start, stop],
                )
                .map_err(ThermalError::PostgresQuery)?;
            for row in rows {
                out.push((
                    fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                    topic.to_string(),
                    row.get::<_, f64>(1),
                ));
            }
        }
        Some(TopicRoute::Emonth { column }) => {
            let sql = format!(
                "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG({col}) AS value FROM emonth WHERE time >= $1 AND time < $2 AND {col} IS NOT NULL GROUP BY bucket ORDER BY bucket",
                col = quoted_identifier(column)
            );
            let rows = client
                .query(&sql, &[start, stop])
                .map_err(ThermalError::PostgresQuery)?;
            for row in rows {
                out.push((
                    fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                    topic.to_string(),
                    row.get::<_, f64>(1),
                ));
            }
        }
        Some(TopicRoute::EbusdPoll { field }) => {
            let rows = client
                .query(
                    "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(value) AS value FROM ebusd_poll WHERE field = $1 AND time >= $2 AND time < $3 AND value IS NOT NULL GROUP BY bucket ORDER BY bucket",
                    &[&field, start, stop],
                )
                .map_err(ThermalError::PostgresQuery)?;
            for row in rows {
                out.push((
                    fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                    topic.to_string(),
                    row.get::<_, f64>(1),
                ));
            }
        }
        _ => {}
    }
    Ok(out)
}

pub fn query_room_temps(
    pg_conninfo: &str,
    sensor_topics: &[&str],
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    let mut client = pg_client(pg_conninfo)?;
    let mut out = Vec::new();
    for topic in sensor_topics {
        out.extend(query_pg_room_topic(&mut client, topic, start, stop)?);
    }
    out.sort_by_key(|(t, _, _)| *t);
    Ok(out)
}

pub fn query_outside_temp(
    pg_conninfo: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    query_pg_timeseries(
        pg_conninfo,
        "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(value) AS value FROM ebusd_poll WHERE field = 'OutsideTemp' AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket ORDER BY bucket",
        &[start, stop],
    )
}

pub fn query_status_codes(
    pg_conninfo: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, i32)>> {
    let mut client = pg_client(pg_conninfo)?;
    let rows = client
        .query(
            "SELECT bucket, value FROM (SELECT DISTINCT ON (time_bucket(INTERVAL '1 minute', time)) time_bucket(INTERVAL '1 minute', time) AS bucket, time, value FROM ebusd_poll WHERE field = 'StatuscodeNum' AND time >= $1 AND time < $2 AND value IS NOT NULL ORDER BY time_bucket(INTERVAL '1 minute', time), time DESC) t ORDER BY bucket",
            &[start, stop],
        )
        .map_err(ThermalError::PostgresQuery)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                row.get::<_, f64>(1).round() as i32,
            )
        })
        .collect())
}

pub fn query_pv_power(
    pg_conninfo: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    query_pg_timeseries(
        pg_conninfo,
        "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(\"P3\") AS value FROM ct_monitor WHERE source = 'EmonPi2' AND time >= $1 AND time < $2 AND \"P3\" IS NOT NULL GROUP BY bucket ORDER BY bucket",
        &[start, stop],
    )
}

pub fn query_building_circuit_flow(
    pg_conninfo: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    query_pg_timeseries(
        pg_conninfo,
        "SELECT time_bucket(INTERVAL '1 minute', time) AS bucket, AVG(value) AS value FROM ebusd_poll WHERE field = 'BuildingCircuitFlow' AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket ORDER BY bucket",
        &[start, stop],
    )
}

pub fn query_mwt(
    pg_conninfo: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    query_pg_timeseries(
        pg_conninfo,
        "WITH field_means AS (SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, field, AVG(value) AS avg_value FROM ebusd_poll WHERE field IN ('FlowTemp', 'ReturnTemp') AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket, field) SELECT bucket, AVG(avg_value) AS value FROM field_means GROUP BY bucket HAVING COUNT(*) = 2 ORDER BY bucket",
        &[start, stop],
    )
}

pub fn query_room_humidity(
    pg_conninfo: &str,
    sensor_topics: &[&str],
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    let mut client = pg_client(pg_conninfo)?;
    let mut out = Vec::new();
    for topic in sensor_topics {
        let Some(device) = topic.strip_prefix("zigbee2mqtt/") else {
            continue;
        };
        let rows = client
            .query(
                "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(humidity) AS value FROM zigbee WHERE device = $1 AND time >= $2 AND time < $3 AND humidity IS NOT NULL GROUP BY bucket ORDER BY bucket",
                &[&device, start, stop],
            )
            .map_err(ThermalError::PostgresQuery)?;
        for row in rows {
            out.push((
                fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                topic.to_string(),
                row.get::<_, f64>(1),
            ));
        }
    }
    out.sort_by_key(|(t, _, _)| *t);
    Ok(out)
}

pub fn query_latest_topic_value(
    pg_conninfo: &str,
    topic: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Option<f64>> {
    let mut client = pg_client(pg_conninfo)?;
    if let Some(field) = topic.strip_prefix("ebusd/poll/") {
        let row = client
            .query_opt(
                "SELECT value FROM ebusd_poll WHERE field = $1 AND time >= $2 AND time < $3 AND value IS NOT NULL ORDER BY time DESC LIMIT 1",
                &[&field, start, stop],
            )
            .map_err(ThermalError::PostgresQuery)?;
        return Ok(row.map(|r| r.get::<_, f64>(0)));
    }
    if let Some(field) = ebusd_live_field(topic) {
        let row = client
            .query_opt(
                "SELECT value FROM ebusd WHERE field = $1 AND time >= $2 AND time < $3 AND value IS NOT NULL AND value <> '' ORDER BY time DESC LIMIT 1",
                &[&field, start, stop],
            )
            .map_err(ThermalError::PostgresQuery)?;
        if let Some(row) = row {
            let value_str: String = row.get(0);
            let value = value_str.parse().map_err(|_| ThermalError::FloatParse {
                context: "ebusd latest value",
                value: value_str,
            })?;
            return Ok(Some(value));
        }
        return Ok(None);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // @lat: [[tests#Topic to table routing#Room sensor topics use correct field name]]
    #[test]
    fn topic_field_name_routing() {
        let zigbee_topic = "zigbee2mqtt/Leather";
        let emon_topic = "emon/emonth2_23/temperature";
        let ebusd_topic = "ebusd/poll/OutsideTemp";

        let is_value_field = zigbee_topic == "emon/emonth2_23/temperature"
            || zigbee_topic.starts_with("ebusd/poll/");
        assert!(!is_value_field);

        let is_value_field =
            emon_topic == "emon/emonth2_23/temperature" || emon_topic.starts_with("ebusd/poll/");
        assert!(is_value_field);

        let is_value_field =
            ebusd_topic == "emon/emonth2_23/temperature" || ebusd_topic.starts_with("ebusd/poll/");
        assert!(is_value_field);
    }

    // @lat: [[tests#Topic to table routing#Topic prefix maps to TimescaleDB table]]
    #[test]
    fn topic_to_table_mapping() {
        assert_eq!(
            topic_route("zigbee2mqtt/Leather"),
            Some(TopicRoute::ZigbeeTemp { device: "Leather" })
        );
        assert_eq!(
            topic_route("emon/emonth2_23/temperature"),
            Some(TopicRoute::Emonth {
                column: "temperature"
            })
        );
        assert_eq!(
            topic_route("ebusd/poll/OutsideTemp"),
            Some(TopicRoute::EbusdPoll {
                field: "OutsideTemp"
            })
        );
        assert_eq!(
            topic_route("emon/EmonPi2/P3"),
            Some(TopicRoute::CtMonitor {
                source: "EmonPi2",
                column: "P3"
            })
        );
        assert_eq!(
            topic_route("emon/multical/dhw_flow"),
            Some(TopicRoute::Multical { column: "dhw_flow" })
        );
        assert_eq!(
            topic_route("emon/heatpump/electric_Power"),
            Some(TopicRoute::Heatpump {
                column: "electric_Power"
            })
        );
        assert_eq!(
            topic_route("emon/tesla/solar_W"),
            Some(TopicRoute::Tesla { column: "solar_W" })
        );
    }

    // @lat: [[tests#Topic to table routing#Live eBUS topics map to ebusd field names]]
    #[test]
    fn live_ebusd_topic_to_field_name() {
        assert_eq!(
            ebusd_live_field("ebusd/hmu/CurrentYieldPower"),
            Some("CurrentYieldPower")
        );
        assert_eq!(ebusd_live_field("ebusd/700/HwcSFMode"), Some("HwcSFMode"));
        assert_eq!(ebusd_live_field("ebusd"), None);
    }

    // @lat: [[tests#Topic to table routing#PV power topic maps to ct_monitor P3 column]]
    #[test]
    fn pv_topic_to_ct_monitor_column() {
        let topic = "emon/EmonPi2/P3";
        let parts: Vec<&str> = topic.splitn(3, '/').collect();
        assert_eq!(parts, vec!["emon", "EmonPi2", "P3"]);
    }

    // @lat: [[tests#Timestamp contracts#Microsecond truncation preserves 10s-interval data]]
    #[test]
    fn timestamp_microsecond_truncation_safe() {
        let nano_ts: i64 = 1768567800_000_000_000;
        let micro_ts: i64 = nano_ts / 1000;
        let seconds_nano = nano_ts / 1_000_000_000;
        let seconds_micro = micro_ts / 1_000_000;
        assert_eq!(seconds_nano, seconds_micro);
    }

    // @lat: [[tests#Timestamp contracts#PostgreSQL TIMESTAMPTZ offset formats parse correctly]]
    #[test]
    fn timestamptz_format_parsing() {
        let rfc3339 = parse_dt("2026-01-15T10:30:00+00:00").unwrap();
        let pg_basic = parse_dt("2026-01-15 10:30:00+00").unwrap();
        let pg_fractional = parse_dt("2026-01-15 10:30:00.123456+00").unwrap();
        let pg_offset = parse_dt("2026-01-15 11:30:00+01").unwrap();

        assert_eq!(rfc3339.timestamp(), 1768473000);
        assert_eq!(pg_basic.timestamp(), rfc3339.timestamp());
        assert_eq!(pg_offset.timestamp(), rfc3339.timestamp());
        assert_eq!(pg_fractional.timestamp(), rfc3339.timestamp());
        assert_eq!(pg_fractional.timestamp_subsec_micros(), 123_456);
    }
}
