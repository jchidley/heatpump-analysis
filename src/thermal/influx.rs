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
            conditions.push(format!("(r.topic == \"{}\" and r._field == \"temperature\")", t));
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

fn query_flux_csv(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<Vec<HashMap<String, String>>> {
    let url = format!("{}/api/v2/query?org={}", influx_url.trim_end_matches('/'), org);
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
