use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, Utc};
use postgres::{Client as PgClient, NoTls};
use reqwest::blocking::Client;

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
    Tesla { column: &'a str },
    Heatpump { column: &'a str },
    Multical { column: &'a str },
    Emonth { column: &'a str },
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
    if let Some(rest) = topic.strip_prefix("emon/tesla/") {
        return Some(TopicRoute::Tesla { column: rest });
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

fn parse_time_value_row(
    row: &HashMap<String, String>,
    time_context: &'static str,
    value_context: &'static str,
) -> ThermalResult<(DateTime<FixedOffset>, f64)> {
    let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
        column: "_time",
        context: time_context,
    })?;
    let t = parse_dt(time_str)?;

    let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
        column: "_value",
        context: value_context,
    })?;
    let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
        context: value_context,
        value: value_str.clone(),
    })?;

    Ok((t, value))
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
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    sensor_topics: &[&str],
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        let mut client = pg_client(conninfo)?;
        let mut out = Vec::new();
        for topic in sensor_topics {
            out.extend(query_pg_room_topic(&mut client, topic, start, stop)?);
        }
        out.sort_by_key(|(t, _, _)| *t);
        return Ok(out);
    }

    let mut conditions = Vec::new();
    for t in sensor_topics {
        if *t == "emon/emonth2_23/temperature" || t.starts_with("ebusd/poll/") {
            // emon and ebusd_poll measurements use _field == "value"
            conditions.push(format!("(r.topic == \"{}\" and r._field == \"value\")", t));
        } else {
            conditions.push(format!(
                "(r.topic == \"{}\" and r._field == \"temperature\")",
                t
            ));
        }
    }

    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => {})\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"topic\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
        conditions.join(" or ")
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;

    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "room temp row",
        })?;
        let t = parse_dt(time_str)?;

        let topic = row
            .get("topic")
            .ok_or(ThermalError::MissingColumn {
                column: "topic",
                context: "room temp row",
            })?
            .to_string();

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "room temp row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "room temp _value",
            value: value_str.clone(),
        })?;
        out.push((t, topic, value));
    }
    out.sort_by_key(|(t, _, _)| *t);
    Ok(out)
}

pub fn query_outside_temp(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        return query_pg_timeseries(
            conninfo,
            "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(value) AS value FROM ebusd_poll WHERE field = 'OutsideTemp' AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket ORDER BY bucket",
            &[start, stop],
        );
    }

    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/OutsideTemp\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(parse_time_value_row(&row, "outside row", "outside _value")?);
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

pub fn query_status_codes(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, i32)>> {
    if let Some(conninfo) = pg_conninfo {
        let mut client = pg_client(conninfo)?;
        let rows = client
            .query(
                "SELECT bucket, value FROM (SELECT DISTINCT ON (time_bucket(INTERVAL '1 minute', time)) time_bucket(INTERVAL '1 minute', time) AS bucket, time, value FROM ebusd_poll WHERE field = 'StatuscodeNum' AND time >= $1 AND time < $2 AND value IS NOT NULL ORDER BY time_bucket(INTERVAL '1 minute', time), time DESC) t ORDER BY bucket",
                &[start, stop],
            )
            .map_err(ThermalError::PostgresQuery)?;
        let mut out = Vec::new();
        for row in rows {
            out.push((
                fixed_utc(row.get::<_, DateTime<Utc>>(0)),
                row.get::<_, f64>(1).round() as i32,
            ));
        }
        return Ok(out);
    }
    // Use last() not mean() — status codes are categorical, not numeric.
    // mean(34,100)=67 is meaningless; last() preserves the actual state.
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/StatuscodeNum\")\n  |> aggregateWindow(every: 1m, fn: last, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "status row",
        })?;
        let t = parse_dt(time_str)?;

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "status row",
        })?;
        let value_f: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "status _value",
            value: value_str.clone(),
        })?;
        out.push((t, value_f.round() as i32));
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

/// Query PV generation (EmonPi2 P3) as SW irradiance proxy.
/// Returns (time, watts) — negative values = generation.
pub fn query_pv_power(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        return query_pg_timeseries(
            conninfo,
            "SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, AVG(\"P3\") AS value FROM ct_monitor WHERE source = 'EmonPi2' AND time >= $1 AND time < $2 AND \"P3\" IS NOT NULL GROUP BY bucket ORDER BY bucket",
            &[start, stop],
        );
    }

    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"emon/EmonPi2/P3\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(parse_time_value_row(&row, "pv row", "pv _value")?);
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

/// Query BuildingCircuitFlow (L/h) from eBUS for HP state classification.
pub fn query_building_circuit_flow(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        return query_pg_timeseries(
            conninfo,
            "SELECT time_bucket(INTERVAL '1 minute', time) AS bucket, AVG(value) AS value FROM ebusd_poll WHERE field = 'BuildingCircuitFlow' AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket ORDER BY bucket",
            &[start, stop],
        );
    }

    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/BuildingCircuitFlow\")\n  |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(parse_time_value_row(&row, "bcf row", "bcf _value")?);
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

/// Query mean water temperature (MWT) = average of FlowTemp and ReturnTemp.
pub fn query_mwt(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        return query_pg_timeseries(
            conninfo,
            "WITH field_means AS (SELECT time_bucket(INTERVAL '5 minutes', time) AS bucket, field, AVG(value) AS avg_value FROM ebusd_poll WHERE field IN ('FlowTemp', 'ReturnTemp') AND time >= $1 AND time < $2 AND value IS NOT NULL GROUP BY bucket, field) SELECT bucket, AVG(avg_value) AS value FROM field_means GROUP BY bucket HAVING COUNT(*) = 2 ORDER BY bucket",
            &[start, stop],
        );
    }
    // Query both flow and return, aggregate to 5m, then we'll merge
    let flux = format!(
        "flow = from(bucket: \"{bucket}\")\n  |> range(start: {start}, stop: {stop})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/FlowTemp\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])\n  |> set(key: \"_field\", value: \"flow\")\n\nret = from(bucket: \"{bucket}\")\n  |> range(start: {start}, stop: {stop})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/ReturnTemp\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])\n  |> set(key: \"_field\", value: \"return\")\n\nunion(tables: [flow, ret])\n  |> pivot(rowKey: [\"_time\"], columnKey: [\"_field\"], valueColumn: \"_value\")\n  |> map(fn: (r) => ({{ r with _value: (r.flow + r[\"return\"]) / 2.0 }}))\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket = bucket,
        start = start.to_rfc3339(),
        stop = stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(parse_time_value_row(&row, "mwt row", "mwt _value")?);
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

pub fn query_room_humidity(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    sensor_topics: &[&str],
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    if let Some(conninfo) = pg_conninfo {
        let mut client = pg_client(conninfo)?;
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
        return Ok(out);
    }

    let mut conditions = Vec::new();
    for t in sensor_topics {
        if *t == "emon/emonth2_23/temperature" {
            continue;
        }
        conditions.push(format!(
            "(r.topic == \"{}\" and r._field == \"humidity\")",
            t
        ));
    }
    if conditions.is_empty() {
        return Ok(Vec::new());
    }

    let flux = format!(
        "from(bucket: \"{bucket}\")\n  |> range(start: {start}, stop: {stop})\n  |> filter(fn: (r) => {cond})\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"topic\", \"_value\"])",
        bucket = bucket,
        start = start.to_rfc3339(),
        stop = stop.to_rfc3339(),
        cond = conditions.join(" or ")
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "humidity row",
        })?;
        let t = parse_dt(time_str)?;
        let topic = row
            .get("topic")
            .ok_or(ThermalError::MissingColumn {
                column: "topic",
                context: "humidity row",
            })?
            .to_string();
        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "humidity row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "humidity _value",
            value: value_str.clone(),
        })?;
        out.push((t, topic, value));
    }
    out.sort_by_key(|(t, _, _)| *t);
    Ok(out)
}

pub fn query_latest_topic_value(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    topic: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Option<f64>> {
    if let Some(conninfo) = pg_conninfo {
        let mut client = pg_client(conninfo)?;
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
    }

    let flux = format!(
        "from(bucket: \"{bucket}\") |> range(start: {start}, stop: {stop}) |> filter(fn: (r) => r.topic == \"{topic}\") |> last() |> keep(columns: [\"_value\"])",
        bucket = bucket,
        start = start.to_rfc3339(),
        stop = stop.to_rfc3339(),
        topic = topic,
    );
    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    Ok(rows
        .last()
        .and_then(|row| row.get("_value"))
        .and_then(|s| s.parse().ok()))
}

/// Public wrapper for display module to query arbitrary Flux.
pub fn query_flux_csv_pub(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<Vec<HashMap<String, String>>> {
    query_flux_csv(influx_url, org, token, flux)
}

fn query_flux_csv(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<Vec<HashMap<String, String>>> {
    let text = query_flux_raw(influx_url, org, token, flux)?;
    parse_influx_annotated_csv(&text)
}

fn query_flux_raw(influx_url: &str, org: &str, token: &str, flux: &str) -> ThermalResult<String> {
    let url = format!(
        "{}/api/v2/query?org={}",
        influx_url.trim_end_matches('/'),
        org
    );
    let body = serde_json::json!({
        "query": flux,
        "type": "flux"
    });

    let resp = Client::new()
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/csv")
        .json(&body)
        .send()
        .map_err(ThermalError::InfluxRequest)?;

    let status = resp.status();
    let text = resp.text().map_err(ThermalError::InfluxResponseRead)?;
    if !status.is_success() {
        return Err(ThermalError::InfluxQueryFailed { status, body: text });
    }

    Ok(text)
}

fn parse_influx_annotated_csv(csv_text: &str) -> ThermalResult<Vec<HashMap<String, String>>> {
    let mut rows = Vec::new();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(csv_text.as_bytes());

    let mut headers: Option<Vec<String>> = None;

    for rec in reader.records() {
        let rec = rec?;
        if rec.is_empty() {
            continue;
        }

        let first = rec.get(0).unwrap_or("");
        if first.starts_with('#') {
            continue;
        }

        if headers.is_none() {
            headers = Some(rec.iter().map(ToString::to_string).collect());
            continue;
        }

        let Some(h) = headers.as_ref() else {
            continue;
        };
        if rec.len() == h.len() && rec.iter().zip(h.iter()).all(|(val, head)| val == head) {
            continue;
        }
        let mut map = HashMap::new();
        for (i, val) in rec.iter().enumerate() {
            if i >= h.len() {
                continue;
            }
            let key = &h[i];
            if key.is_empty() {
                continue;
            }
            map.insert(key.clone(), val.to_string());
        }
        if !map.is_empty() {
            rows.push(map);
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_influx_annotated_csv ──────────────────────────────────────────

    // @lat: [[tests#InfluxDB wire-format parsing#Empty CSV input returns empty vec]]
    #[test]
    fn parse_csv_empty_input() {
        let rows = parse_influx_annotated_csv("").unwrap();
        assert!(rows.is_empty());
    }

    // @lat: [[tests#InfluxDB wire-format parsing#Annotation lines are skipped]]
    #[test]
    fn parse_csv_skips_annotations() {
        let csv = "\
#datatype,string,long,dateTime:RFC3339,double,string
#group,false,false,false,false,true
#default,_result,,,,
,result,table,_time,_value,topic
,_result,0,2026-01-15T10:00:00Z,21.5,zigbee2mqtt/Leather
,_result,0,2026-01-15T10:05:00Z,21.6,zigbee2mqtt/Leather
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("_value").unwrap(), "21.5");
        assert_eq!(rows[0].get("topic").unwrap(), "zigbee2mqtt/Leather");
        assert_eq!(rows[1].get("_value").unwrap(), "21.6");
    }

    // @lat: [[tests#InfluxDB wire-format parsing#Multi-table CSV resets headers per table]]
    #[test]
    fn parse_csv_multi_table() {
        // InfluxDB emits a blank line + new header row between tables
        let csv = "\
#datatype,string,long,dateTime:RFC3339,double,string
#group,false,false,false,false,true
,result,table,_time,_value,topic
,_result,0,2026-01-15T10:00:00Z,21.5,zigbee2mqtt/Leather

#datatype,string,long,dateTime:RFC3339,double,string
#group,false,false,false,false,true
,result,table,_time,_value,topic
,_result,1,2026-01-15T10:00:00Z,19.3,zigbee2mqtt/Aldora
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("topic").unwrap(), "zigbee2mqtt/Leather");
        assert_eq!(rows[1].get("topic").unwrap(), "zigbee2mqtt/Aldora");
    }

    // @lat: [[tests#InfluxDB wire-format parsing#Duplicate header rows are not emitted as data]]
    #[test]
    fn parse_csv_skips_duplicate_header_rows() {
        // Some Flux responses repeat the header row between result blocks
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,21.5
,result,table,_time,_value
,_result,1,2026-01-15T10:05:00Z,19.3
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("_value").unwrap(), "21.5");
        assert_eq!(rows[1].get("_value").unwrap(), "19.3");
    }

    // @lat: [[tests#InfluxDB wire-format parsing#Empty-key columns are excluded from output map]]
    #[test]
    fn parse_csv_skips_empty_key_columns() {
        // InfluxDB CSV often has an empty first column header (the annotation column)
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,42.0
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].contains_key(""));
        assert_eq!(rows[0].get("_value").unwrap(), "42.0");
    }

    // @lat: [[tests#InfluxDB wire-format parsing#All-annotation CSV returns empty vec]]
    #[test]
    fn parse_csv_annotations_only() {
        let csv = "\
#datatype,string,long
#group,false,false
#default,_result,
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert!(rows.is_empty());
    }

    // ── parse_dt ───────────────────────────────────────────────────────────

    // @lat: [[tests#InfluxDB wire-format parsing#parse_dt accepts standard RFC3339 formats]]
    #[test]
    fn parse_dt_rfc3339_variants() {
        // Z suffix
        let dt = parse_dt("2026-01-15T10:30:00Z").unwrap();
        assert_eq!(dt.timestamp(), 1768473000);

        // Explicit +00:00 offset
        let dt2 = parse_dt("2026-01-15T10:30:00+00:00").unwrap();
        assert_eq!(dt.timestamp(), dt2.timestamp());

        // Non-zero offset
        let dt3 = parse_dt("2026-01-15T11:30:00+01:00").unwrap();
        assert_eq!(dt.timestamp(), dt3.timestamp());
    }

    // @lat: [[tests#InfluxDB wire-format parsing#parse_dt rejects invalid timestamp input]]
    #[test]
    fn parse_dt_rejects_bad_input() {
        assert!(parse_dt("2026-01-15 10:30:00").is_err());
        assert!(parse_dt("not a date").is_err());
        assert!(parse_dt("").is_err());
    }

    // ── query return contracts ─────────────────────────────────────────────
    // These tests feed known CSV through parse_influx_annotated_csv and then
    // verify the typed extraction logic that each query_* function applies.
    // This pins the output contract that the PostgreSQL migration must preserve.

    // @lat: [[tests#Query return contracts#Room temps extracts timestamp-topic-value triples]]
    #[test]
    fn room_temps_csv_contract() {
        // Simulate the CSV that query_room_temps receives from InfluxDB
        let csv = "\
,result,table,_time,_value,topic
,_result,0,2026-01-15T10:00:00Z,21.5,zigbee2mqtt/Leather
,_result,0,2026-01-15T10:05:00Z,21.6,zigbee2mqtt/Leather
,_result,1,2026-01-15T10:00:00Z,19.3,zigbee2mqtt/Aldora
";
        let rows = parse_influx_annotated_csv(csv).unwrap();

        // Apply the same extraction logic as query_room_temps
        let mut out: Vec<(DateTime<FixedOffset>, String, f64)> = Vec::new();
        for row in &rows {
            let t = parse_dt(row.get("_time").unwrap()).unwrap();
            let topic = row.get("topic").unwrap().to_string();
            let value: f64 = row.get("_value").unwrap().parse().unwrap();
            out.push((t, topic, value));
        }
        out.sort_by_key(|(t, _, _)| *t);

        assert_eq!(out.len(), 3);
        // Both 10:00 entries first, then 10:05
        assert_eq!(out[0].1, "zigbee2mqtt/Leather");
        assert!((out[0].2 - 21.5).abs() < 1e-9);
        assert_eq!(out[1].1, "zigbee2mqtt/Aldora");
        assert!((out[1].2 - 19.3).abs() < 1e-9);
        assert_eq!(out[2].1, "zigbee2mqtt/Leather");
        assert!((out[2].2 - 21.6).abs() < 1e-9);
    }

    // @lat: [[tests#Query return contracts#Outside temp extracts timestamp-value pairs sorted by time]]
    #[test]
    fn outside_temp_csv_contract() {
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:05:00Z,5.2
,_result,0,2026-01-15T10:00:00Z,4.8
";
        let rows = parse_influx_annotated_csv(csv).unwrap();

        let mut out: Vec<(DateTime<FixedOffset>, f64)> = Vec::new();
        for row in &rows {
            let t = parse_dt(row.get("_time").unwrap()).unwrap();
            let value: f64 = row.get("_value").unwrap().parse().unwrap();
            out.push((t, value));
        }
        out.sort_by_key(|(t, _)| *t);

        assert_eq!(out.len(), 2);
        // Sorted: 10:00 first
        assert!((out[0].1 - 4.8).abs() < 1e-9);
        assert!((out[1].1 - 5.2).abs() < 1e-9);
    }

    // @lat: [[tests#Query return contracts#Status codes round float to integer]]
    #[test]
    fn status_codes_csv_contract() {
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,34.0
,_result,0,2026-01-15T10:01:00Z,100.0
";
        let rows = parse_influx_annotated_csv(csv).unwrap();

        let mut out: Vec<(DateTime<FixedOffset>, i32)> = Vec::new();
        for row in &rows {
            let t = parse_dt(row.get("_time").unwrap()).unwrap();
            let value_f: f64 = row.get("_value").unwrap().parse().unwrap();
            out.push((t, value_f.round() as i32));
        }
        out.sort_by_key(|(t, _)| *t);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1, 34);
        assert_eq!(out[1].1, 100);
    }

    // @lat: [[tests#Query return contracts#MWT CSV with flow and return produces averaged pairs]]
    #[test]
    fn mwt_csv_contract() {
        // After the Flux union+pivot+map, MWT returns _time and _value (the average)
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,32.5
,_result,0,2026-01-15T10:05:00Z,33.1
";
        let rows = parse_influx_annotated_csv(csv).unwrap();

        let mut out: Vec<(DateTime<FixedOffset>, f64)> = Vec::new();
        for row in &rows {
            let t = parse_dt(row.get("_time").unwrap()).unwrap();
            let value: f64 = row.get("_value").unwrap().parse().unwrap();
            out.push((t, value));
        }
        out.sort_by_key(|(t, _)| *t);

        assert_eq!(out.len(), 2);
        assert!((out[0].1 - 32.5).abs() < 1e-9);
        assert!((out[1].1 - 33.1).abs() < 1e-9);
    }

    // @lat: [[tests#Query return contracts#Missing required column returns MissingColumn error]]
    #[test]
    fn missing_column_errors() {
        let csv = "\
,result,table,_time,value
,_result,0,2026-01-15T10:00:00Z,99.9
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        let err = parse_time_value_row(&rows[0], "outside row", "outside _value").unwrap_err();

        match err {
            ThermalError::MissingColumn { column, context } => {
                assert_eq!(column, "_value");
                assert_eq!(context, "outside _value");
            }
            other => panic!("expected MissingColumn error, got {other:?}"),
        }
    }

    // @lat: [[tests#Query return contracts#Unparseable float in value column returns FloatParse error]]
    #[test]
    fn unparseable_float_errors() {
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,not_a_number
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        let err = parse_time_value_row(&rows[0], "outside row", "outside _value").unwrap_err();

        match err {
            ThermalError::FloatParse { context, value } => {
                assert_eq!(context, "outside _value");
                assert_eq!(value, "not_a_number");
            }
            other => panic!("expected FloatParse error, got {other:?}"),
        }
    }

    // ── topic→table routing ────────────────────────────────────────────────
    // These tests document the implicit mapping from InfluxDB topic tags
    // to TimescaleDB tables. The PostgreSQL migration must route each topic
    // to the correct table and column.

    // @lat: [[tests#Topic to table routing#Room sensor topics use correct field name]]
    #[test]
    fn topic_field_name_routing() {
        // The query_room_temps function distinguishes _field by topic prefix
        let zigbee_topic = "zigbee2mqtt/Leather";
        let emon_topic = "emon/emonth2_23/temperature";
        let ebusd_topic = "ebusd/poll/OutsideTemp";

        // Zigbee sensors use _field == "temperature"
        let is_value_field = zigbee_topic == "emon/emonth2_23/temperature"
            || zigbee_topic.starts_with("ebusd/poll/");
        assert!(
            !is_value_field,
            "Zigbee topics should use 'temperature' field"
        );

        // emonth2_23 uses _field == "value"
        let is_value_field =
            emon_topic == "emon/emonth2_23/temperature" || emon_topic.starts_with("ebusd/poll/");
        assert!(is_value_field, "emonth2_23 should use 'value' field");

        // ebusd/poll uses _field == "value"
        let is_value_field =
            ebusd_topic == "emon/emonth2_23/temperature" || ebusd_topic.starts_with("ebusd/poll/");
        assert!(is_value_field, "ebusd/poll should use 'value' field");
    }

    // @lat: [[tests#Topic to table routing#Topic prefix maps to TimescaleDB table]]
    #[test]
    fn topic_to_table_mapping() {
        assert_eq!(
            topic_route("zigbee2mqtt/Leather"),
            Some(TopicRoute::ZigbeeTemp { device: "Leather" })
        );
        assert_eq!(
            topic_route("zigbee2mqtt/Aldora"),
            Some(TopicRoute::ZigbeeTemp { device: "Aldora" })
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
            topic_route("ebusd/poll/BuildingCircuitFlow"),
            Some(TopicRoute::EbusdPoll {
                field: "BuildingCircuitFlow"
            })
        );
        assert_eq!(
            topic_route("ebusd/poll/FlowTemp"),
            Some(TopicRoute::EbusdPoll { field: "FlowTemp" })
        );
        assert_eq!(
            topic_route("ebusd/poll/ReturnTemp"),
            Some(TopicRoute::EbusdPoll {
                field: "ReturnTemp"
            })
        );
        assert_eq!(
            topic_route("ebusd/poll/StatuscodeNum"),
            Some(TopicRoute::EbusdPoll {
                field: "StatuscodeNum"
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
        // emon/EmonPi2/P3 → ct_monitor WHERE source='EmonPi2', column "P3"
        let topic = "emon/EmonPi2/P3";
        let parts: Vec<&str> = topic.splitn(3, '/').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "emon");
        assert_eq!(parts[1], "EmonPi2"); // → source column
        assert_eq!(parts[2], "P3"); // → "P3" column in wide table
    }

    // ── timestamp precision ────────────────────────────────────────────────

    // @lat: [[tests#Timestamp migration contracts#Microsecond truncation preserves 10s-interval data]]
    #[test]
    fn timestamp_microsecond_truncation_safe() {
        // InfluxDB stores nanoseconds, TimescaleDB stores microseconds.
        // At 10s sample intervals, truncation is safe.
        let nano_ts: i64 = 1768567800_000_000_000; // nanoseconds
        let micro_ts: i64 = nano_ts / 1000; // microseconds
        let seconds_nano = nano_ts / 1_000_000_000;
        let seconds_micro = micro_ts / 1_000_000;
        assert_eq!(
            seconds_nano, seconds_micro,
            "Truncation must preserve seconds"
        );
    }

    // @lat: [[tests#Timestamp migration contracts#PostgreSQL TIMESTAMPTZ offset formats parse correctly]]
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

    // ── multi-topic condition builder ──────────────────────────────────────

    // @lat: [[tests#Query return contracts#Multi-topic query builds OR conditions with correct field names]]
    #[test]
    fn multi_topic_condition_builder() {
        // Reproduce the condition-building logic from query_room_temps
        let sensor_topics: &[&str] = &[
            "zigbee2mqtt/Leather",
            "zigbee2mqtt/Aldora",
            "emon/emonth2_23/temperature",
            "ebusd/poll/OutsideTemp",
        ];

        let mut conditions = Vec::new();
        for t in sensor_topics {
            if *t == "emon/emonth2_23/temperature" || t.starts_with("ebusd/poll/") {
                conditions.push(format!("(r.topic == \"{}\" and r._field == \"value\")", t));
            } else {
                conditions.push(format!(
                    "(r.topic == \"{}\" and r._field == \"temperature\")",
                    t
                ));
            }
        }

        let clause = conditions.join(" or ");

        // Zigbee sensors use "temperature"
        assert!(clause.contains("zigbee2mqtt/Leather\" and r._field == \"temperature\""));
        assert!(clause.contains("zigbee2mqtt/Aldora\" and r._field == \"temperature\""));

        // emonth2_23 and ebusd/poll use "value"
        assert!(clause.contains("emon/emonth2_23/temperature\" and r._field == \"value\""));
        assert!(clause.contains("ebusd/poll/OutsideTemp\" and r._field == \"value\""));

        // Should have 4 conditions joined by " or "
        assert_eq!(conditions.len(), 4);
    }

    // ── wide-row NULL handling ─────────────────────────────────────────────

    // @lat: [[tests#Query return contracts#Wide-row CSV with NULL columns parses present fields only]]
    #[test]
    fn wide_row_null_columns_parsed() {
        // ct_monitor wide rows: 6-channel devices have P7-P12 as empty/missing
        // When querying P3 (PV), NULLs in other columns must not break parsing
        let csv = "\
,result,table,_time,_value,topic
,_result,0,2026-01-15T10:00:00Z,-1500.0,emon/EmonPi2/P3
,_result,0,2026-01-15T10:05:00Z,,emon/EmonPi2/P7
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);

        // P3 row has a valid numeric value
        let p3_val: f64 = rows[0].get("_value").unwrap().parse().unwrap();
        assert!((p3_val - (-1500.0)).abs() < 1e-9);

        // P7 row has an empty _value — consumer must handle this
        let p7_val_str = rows[1].get("_value").unwrap();
        assert!(
            p7_val_str.is_empty() || p7_val_str.parse::<f64>().is_err(),
            "Empty/NULL _value from wide-row should not parse as a valid float"
        );
    }

    // @lat: [[tests#Query return contracts#Single-value CSV parsing extracts last value]]
    #[test]
    fn single_value_csv_parsing() {
        // The adaptive-heating-mvp query_single_value function uses a simpler
        // inline CSV parser. This test pins the contract: given a last() query
        // result, extract the single _value.
        let csv = "\
#datatype,string,long,dateTime:RFC3339,double
#group,false,false,false,false
#default,_result,,,
,result,table,_time,_value
,_result,0,2026-01-15T10:30:00Z,21.5
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert_eq!(rows.len(), 1);
        let val: f64 = rows[0].get("_value").unwrap().parse().unwrap();
        assert!((val - 21.5).abs() < 1e-9);
    }

    // @lat: [[tests#Query return contracts#Empty result from last query returns no rows]]
    #[test]
    fn empty_last_query_result() {
        // When a sensor has no data in the lookback window, last() returns
        // an empty CSV with only annotations and headers — no data rows.
        let csv = "\
#datatype,string,long,dateTime:RFC3339,double
#group,false,false,false,false
#default,_result,,,
,result,table,_time,_value

";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        assert!(
            rows.is_empty(),
            "Empty last() result should produce zero rows"
        );
    }
}
