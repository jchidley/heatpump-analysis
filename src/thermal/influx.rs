use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use reqwest::blocking::Client;

use super::error::{ThermalError, ThermalResult};

pub fn parse_dt(s: &str) -> ThermalResult<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|source| ThermalError::DateTimeParse {
        value: s.to_string(),
        source,
    })
}

pub fn query_room_temps(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    sensor_topics: &[&str],
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String, f64)>> {
    let mut conditions = Vec::new();
    for t in sensor_topics {
        if *t == "emon/emonth2_23/temperature" {
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
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/OutsideTemp\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "outside row",
        })?;
        let t = parse_dt(time_str)?;

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "outside row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "outside _value",
            value: value_str.clone(),
        })?;
        out.push((t, value));
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

pub fn query_status_codes(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, i32)>> {
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
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"emon/EmonPi2/P3\")\n  |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "pv row",
        })?;
        let t = parse_dt(time_str)?;

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "pv row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "pv _value",
            value: value_str.clone(),
        })?;
        out.push((t, value));
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
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"ebusd/poll/BuildingCircuitFlow\")\n  |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        bucket,
        start.to_rfc3339(),
        stop.to_rfc3339(),
    );

    let rows = query_flux_csv(influx_url, org, token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "bcf row",
        })?;
        let t = parse_dt(time_str)?;

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "bcf row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "bcf _value",
            value: value_str.clone(),
        })?;
        out.push((t, value));
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
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
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
        let time_str = row.get("_time").ok_or(ThermalError::MissingColumn {
            column: "_time",
            context: "mwt row",
        })?;
        let t = parse_dt(time_str)?;

        let value_str = row.get("_value").ok_or(ThermalError::MissingColumn {
            column: "_value",
            context: "mwt row",
        })?;
        let value: f64 = value_str.parse().map_err(|_| ThermalError::FloatParse {
            context: "mwt _value",
            value: value_str.clone(),
        })?;
        out.push((t, value));
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

fn query_flux_csv(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<Vec<HashMap<String, String>>> {
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

    parse_influx_annotated_csv(&text)
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

        let mut map = HashMap::new();
        let Some(h) = headers.as_ref() else {
            continue;
        };
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
