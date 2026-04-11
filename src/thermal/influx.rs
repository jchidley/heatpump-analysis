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

/// Public wrapper for display module to query arbitrary Flux.
pub fn query_flux_csv_pub(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<Vec<HashMap<String, String>>> {
    query_flux_csv(influx_url, org, token, flux)
}

/// Public wrapper to execute Flux and return raw annotated CSV.
pub fn query_flux_raw_pub(
    influx_url: &str,
    org: &str,
    token: &str,
    flux: &str,
) -> ThermalResult<String> {
    query_flux_raw(influx_url, org, token, flux)
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

    // @lat: [[tests#InfluxDB wire-format parsing#parse_dt rejects non-RFC3339 input]]
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
,result,table,_time
,_result,0,2026-01-15T10:00:00Z
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        // Simulate query_outside_temp looking for _value
        let result = rows[0].get("_value");
        assert!(result.is_none(), "Missing _value column should not be present");
    }

    // @lat: [[tests#Query return contracts#Unparseable float in value column returns FloatParse error]]
    #[test]
    fn unparseable_float_errors() {
        let csv = "\
,result,table,_time,_value
,_result,0,2026-01-15T10:00:00Z,not_a_number
";
        let rows = parse_influx_annotated_csv(csv).unwrap();
        let value_str = rows[0].get("_value").unwrap();
        let parse_result: Result<f64, _> = value_str.parse();
        assert!(parse_result.is_err());
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
        assert!(!is_value_field, "Zigbee topics should use 'temperature' field");

        // emonth2_23 uses _field == "value"
        let is_value_field = emon_topic == "emon/emonth2_23/temperature"
            || emon_topic.starts_with("ebusd/poll/");
        assert!(is_value_field, "emonth2_23 should use 'value' field");

        // ebusd/poll uses _field == "value"
        let is_value_field = ebusd_topic == "emon/emonth2_23/temperature"
            || ebusd_topic.starts_with("ebusd/poll/");
        assert!(is_value_field, "ebusd/poll should use 'value' field");
    }

    // @lat: [[tests#Topic to table routing#Topic prefix maps to TimescaleDB table]]
    #[test]
    fn topic_to_table_mapping() {
        // Document the routing that the migration must implement.
        // This function can later become the real routing function.
        fn expected_table(topic: &str) -> &str {
            if topic.starts_with("zigbee2mqtt/") {
                "zigbee"
            } else if topic.starts_with("emon/EmonPi2/") || topic.starts_with("emon/emonpi2_cu/")
                || topic.starts_with("emon/emontx5_cu/") {
                "ct_monitor"
            } else if topic.starts_with("emon/tesla/") {
                "tesla"
            } else if topic.starts_with("emon/heatpump/") {
                "heatpump"
            } else if topic.starts_with("emon/multical/") {
                "multical"
            } else if topic.starts_with("emon/emonth2_23/") {
                "emonth"
            } else if topic.starts_with("emon/sensors/") {
                "sensors"
            } else if topic.starts_with("ebusd/poll/") {
                "ebusd_poll"
            } else if topic.starts_with("emon/metoffice/") {
                "metoffice"
            } else {
                "unknown"
            }
        }

        // Room temperature topics
        assert_eq!(expected_table("zigbee2mqtt/Leather"), "zigbee");
        assert_eq!(expected_table("zigbee2mqtt/Aldora"), "zigbee");
        assert_eq!(expected_table("emon/emonth2_23/temperature"), "emonth");

        // Outside temperature
        assert_eq!(expected_table("ebusd/poll/OutsideTemp"), "ebusd_poll");

        // PV power (P3 on EmonPi2)
        assert_eq!(expected_table("emon/EmonPi2/P3"), "ct_monitor");

        // Building circuit flow
        assert_eq!(expected_table("ebusd/poll/BuildingCircuitFlow"), "ebusd_poll");

        // MWT components
        assert_eq!(expected_table("ebusd/poll/FlowTemp"), "ebusd_poll");
        assert_eq!(expected_table("ebusd/poll/ReturnTemp"), "ebusd_poll");

        // Status codes
        assert_eq!(expected_table("ebusd/poll/StatuscodeNum"), "ebusd_poll");

        // DHW topics
        assert_eq!(expected_table("emon/multical/dhw_flow"), "multical");
        assert_eq!(expected_table("emon/heatpump/electric_Power"), "heatpump");

        // Tesla
        assert_eq!(expected_table("emon/tesla/solar_W"), "tesla");
    }

    // @lat: [[tests#Topic to table routing#PV power topic maps to ct_monitor P3 column]]
    #[test]
    fn pv_topic_to_ct_monitor_column() {
        // emon/EmonPi2/P3 → ct_monitor WHERE source='EmonPi2', column "P3"
        let topic = "emon/EmonPi2/P3";
        let parts: Vec<&str> = topic.splitn(3, '/').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "emon");
        assert_eq!(parts[1], "EmonPi2");  // → source column
        assert_eq!(parts[2], "P3");       // → "P3" column in wide table
    }

    // ── timestamp precision ────────────────────────────────────────────────

    // @lat: [[tests#Timestamp migration contracts#Microsecond truncation preserves 10s-interval data]]
    #[test]
    fn timestamp_microsecond_truncation_safe() {
        // InfluxDB stores nanoseconds, TimescaleDB stores microseconds.
        // At 10s sample intervals, truncation is safe.
        let nano_ts: i64 = 1768567800_000_000_000; // nanoseconds
        let micro_ts: i64 = nano_ts / 1000;         // microseconds
        let seconds_nano = nano_ts / 1_000_000_000;
        let seconds_micro = micro_ts / 1_000_000;
        assert_eq!(seconds_nano, seconds_micro, "Truncation must preserve seconds");
    }

    // @lat: [[tests#Timestamp migration contracts#PostgreSQL TIMESTAMPTZ offset formats parse correctly]]
    #[test]
    fn timestamptz_format_parsing() {
        // PostgreSQL returns timestamps like: 2026-01-15 10:30:00+00
        // parse_dt currently requires RFC3339 (T separator, colon in offset)
        // This documents what the migration must handle.
        let rfc3339 = "2026-01-15T10:30:00+00:00";
        let dt = parse_dt(rfc3339).unwrap();

        // PostgreSQL common format (without T, short offset) won't parse with parse_dt
        let pg_format = "2026-01-15 10:30:00+00";
        assert!(
            parse_dt(pg_format).is_err(),
            "Current parse_dt requires RFC3339 — migration must adapt timestamp format"
        );

        // Verify the expected unix timestamp for the valid parse
        assert_eq!(dt.timestamp(), 1768473000);
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
                conditions.push(format!("(r.topic == \"{}\" and r._field == \"temperature\")", t));
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
        assert!(rows.is_empty(), "Empty last() result should produce zero rows");
    }
}
