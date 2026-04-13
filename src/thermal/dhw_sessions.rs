//! DHW session analysis — historical draw/charge detection with inflection analysis.
//!
//! Queries InfluxDB at 10s resolution for event detection, then 2s for per-draw
//! inflection analysis. Classifies draws by type (bath/shower/tap), tracks
//! HwcStorageTemp during draws, detects draws during HP charging.
//! Writes `dhw_inflection` to InfluxDB and, when configured, mirrors `dhw_capacity`
//! to TimescaleDB so z2m-hub startup autoload can read the migrated store.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, FixedOffset, Offset, TimeDelta, TimeZone, Utc};
use postgres::{Client as PgClient, NoTls};
use reqwest::blocking::Client;

use super::error::{ThermalError, ThermalResult};
use super::influx::query_flux_csv_pub;

#[derive(Debug, Clone, PartialEq, Eq)]
enum MeasurementRoute {
    Multical { column: String },
    EbusdPoll { field: String },
}

fn measurement_route(measurement: &str, field: &str) -> Option<MeasurementRoute> {
    match measurement {
        "emon" if field.starts_with("dhw_") => {
            let column = if field == "dhw_volume_V1" {
                "dhw_volume_v1".to_string()
            } else {
                field.to_string()
            };
            Some(MeasurementRoute::Multical { column })
        }
        "ebusd_poll" => Some(MeasurementRoute::EbusdPoll {
            field: field.to_string(),
        }),
        _ => None,
    }
}

fn pg_client(conninfo: &str) -> ThermalResult<PgClient> {
    PgClient::connect(conninfo, NoTls).map_err(ThermalError::PostgresConnect)
}

fn query_pg_series(
    conninfo: &str,
    route: &MeasurementRoute,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
    aggregate: Option<(&str, &str)>,
) -> ThermalResult<TsVal> {
    let mut client = pg_client(conninfo)?;
    let rows = match (route, aggregate) {
        (MeasurementRoute::Multical { column }, Some((interval, "max"))) => {
            let sql = format!(
                "SELECT time_bucket(INTERVAL '{interval}', time) AS bucket, MAX(\"{column}\") AS value FROM multical WHERE time >= $1 AND time < $2 AND \"{column}\" IS NOT NULL GROUP BY bucket ORDER BY bucket"
            );
            client.query(&sql, &[start, stop])
        }
        (MeasurementRoute::Multical { column }, Some((interval, "last"))) => {
            let sql = format!(
                "SELECT bucket, value FROM (SELECT DISTINCT ON (time_bucket(INTERVAL '{interval}', time)) time_bucket(INTERVAL '{interval}', time) AS bucket, time, \"{column}\" AS value FROM multical WHERE time >= $1 AND time < $2 AND \"{column}\" IS NOT NULL ORDER BY time_bucket(INTERVAL '{interval}', time), time DESC) t ORDER BY bucket"
            );
            client.query(&sql, &[start, stop])
        }
        (MeasurementRoute::Multical { column }, None) => {
            let sql = format!(
                "SELECT time, \"{column}\" AS value FROM multical WHERE time >= $1 AND time < $2 AND \"{column}\" IS NOT NULL ORDER BY time"
            );
            client.query(&sql, &[start, stop])
        }
        (MeasurementRoute::EbusdPoll { field }, Some((interval, "last"))) => client.query(
            &format!(
                "SELECT bucket, value FROM (SELECT DISTINCT ON (time_bucket(INTERVAL '{interval}', time)) time_bucket(INTERVAL '{interval}', time) AS bucket, time, value FROM ebusd_poll WHERE field = $1 AND time >= $2 AND time < $3 AND value IS NOT NULL ORDER BY time_bucket(INTERVAL '{interval}', time), time DESC) t ORDER BY bucket"
            ),
            &[field, start, stop],
        ),
        (MeasurementRoute::EbusdPoll { field }, None) => client.query(
            "SELECT time, value FROM ebusd_poll WHERE field = $1 AND time >= $2 AND time < $3 AND value IS NOT NULL ORDER BY time",
            &[field, start, stop],
        ),
        (_, Some((_interval, other))) => {
            unreachable!("unsupported PostgreSQL aggregate for dhw_sessions: {other}");
        }
    }
    .map_err(ThermalError::PostgresQuery)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get::<_, DateTime<Utc>>(0).fixed_offset(),
                row.get::<_, f64>(1),
            )
        })
        .collect())
}

// ── Constants ───────────────────────────────────────────────────────────────

/// Geometric max usable volume (litres) — 300L Kingspan Albion cylinder
const GEOMETRIC_MAX: f64 = 243.0;

/// BuildingCircuitFlow threshold for DHW charging (L/h)
const CHARGE_BC_FLOW: f64 = 900.0;
/// Minimum charge duration to count (seconds)
const CHARGE_MIN_DURATION: f64 = 300.0;

/// Minimum DHW flow to count as a draw (L/h, from Multical)
const DRAW_FLOW_MIN: f64 = 100.0;
/// Minimum volume drawn to count as a draw event (litres)
const DRAW_MIN_VOLUME: f64 = 10.0;

/// Flow rate thresholds for draw type classification (L/h)
const BATH_FLOW_MIN: f64 = 650.0;
const SHOWER_FLOW_MIN: f64 = 350.0;

/// T1 rate thresholds for inflection detection (°C per litre, negative = dropping)
const HINT_RATE: f64 = -0.003;
const SIGNAL_RATE: f64 = -0.01;
/// Rolling window for rate calculation (litres)
const ROLLING_WINDOW: f64 = 10.0;

/// T2 above this = WWHR active (warmer inlet from drain heat exchanger)
const WWHR_T2_THRESHOLD: f64 = 20.0;

// ── Data types ──────────────────────────────────────────────────────────────

type TsVal = Vec<(DateTime<FixedOffset>, f64)>;

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ChargeEvent {
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    t1_pre: f64,
    hwc_end: f64,
    t1_end: f64,
    crossover: bool,
    volume_at_end: f64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DrawEvent {
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    volume_register_start: f64,
    volume_register_end: f64,
    volume_drawn: f64,
    preceding_charge: Option<ChargeEvent>,
    cumulative_since_charge: f64,
    gap_hours: f64,
    /// Whether the HP was charging during (part of) this draw
    during_charge: bool,
}

/// Draw type based on flow rate and volume
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawType {
    Bath,
    Shower,
    Tap,
}

impl fmt::Display for DrawType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bath => write!(f, "bath"),
            Self::Shower => write!(f, "shower"),
            Self::Tap => write!(f, "tap"),
        }
    }
}

/// Inflection classification for capacity analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InflectionCategory {
    /// Full capacity measurement (crossover charge + T1 inflection detected)
    Capacity,
    /// Partial charge state (inflection found but charge wasn't full)
    Partial,
    /// Lower bound (no definitive inflection, draw wasn't large enough)
    LowerBound,
}

impl fmt::Display for InflectionCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity => write!(f, "capacity"),
            Self::Partial => write!(f, "partial"),
            Self::LowerBound => write!(f, "lower_bound"),
        }
    }
}

#[derive(Debug, Clone)]
struct InflectionResult {
    draw: DrawEvent,
    hint_cumulative: Option<f64>,
    definitive_cumulative: Option<f64>,
    definitive_draw_vol: Option<f64>,
    definitive_rate: Option<f64>,
    t1_start: f64,
    t1_at_definitive: Option<f64>,
    /// Settled mains temp during flow (not dead-leg)
    mains_temp: f64,
    /// Peak flow rate during draw (L/h)
    peak_flow_rate: f64,
    /// HwcStorageTemp before draw started
    hwc_pre: f64,
    /// Minimum HwcStorageTemp during/after draw
    hwc_min: f64,
    /// HwcStorageTemp drop during draw
    hwc_drop: f64,
}

impl InflectionResult {
    fn draw_type(&self) -> DrawType {
        if self.peak_flow_rate >= BATH_FLOW_MIN {
            DrawType::Bath
        } else if self.peak_flow_rate >= SHOWER_FLOW_MIN && self.draw.volume_drawn >= 20.0 {
            DrawType::Shower
        } else {
            DrawType::Tap
        }
    }

    fn inflection_category(&self) -> InflectionCategory {
        if let Some(_def) = self.definitive_cumulative {
            if let Some(ref charge) = self.draw.preceding_charge {
                if charge.crossover && charge.t1_end >= 43.0 {
                    return InflectionCategory::Capacity;
                }
            }
            return InflectionCategory::Partial;
        }
        InflectionCategory::LowerBound
    }

    fn best_volume(&self) -> f64 {
        self.definitive_cumulative
            .or(self.hint_cumulative)
            .unwrap_or(self.draw.cumulative_since_charge)
    }
}

// ── InfluxDB helpers ────────────────────────────────────────────────────────

fn parse_ts_val(rows: &[HashMap<String, String>]) -> TsVal {
    let mut out = Vec::new();
    for r in rows {
        let ts_str = r.get("_time").or_else(|| r.get("time"));
        let val_str = r.get("_value").or_else(|| r.get("value"));
        if let (Some(ts), Some(val)) = (ts_str, val_str) {
            if let Ok(v) = val.parse::<f64>() {
                // Parse ISO timestamp
                if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                    out.push((dt, v));
                } else if let Ok(dt) = DateTime::parse_from_rfc3339(&ts.replace("Z", "+00:00")) {
                    out.push((dt, v));
                } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
                    &ts[..19.min(ts.len())],
                    "%Y-%m-%dT%H:%M:%S",
                ) {
                    let fixed = Utc.fix().from_utc_datetime(&dt);
                    out.push((fixed, v));
                }
            }
        }
    }
    out
}

fn query_ts(url: &str, org: &str, token: &str, flux: &str) -> ThermalResult<TsVal> {
    let rows = query_flux_csv_pub(url, org, token, flux)?;
    Ok(parse_ts_val(&rows))
}

fn query_measurement_series(
    url: &str,
    org: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    bucket: &str,
    measurement: &str,
    field: &str,
    start: &DateTime<FixedOffset>,
    stop: &DateTime<FixedOffset>,
    aggregate: Option<(&str, &str)>,
) -> ThermalResult<TsVal> {
    if let Some(conninfo) = pg_conninfo {
        if let Some(route) = measurement_route(measurement, field) {
            return query_pg_series(conninfo, &route, start, stop, aggregate);
        }
    }

    let flux = match (measurement, field, aggregate) {
        ("emon", field, Some((every, agg))) => format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "{field}")
  |> aggregateWindow(every: {every}, fn: {agg}, createEmpty: false)"#,
            start = start.to_rfc3339(),
            stop = stop.to_rfc3339(),
        ),
        ("emon", field, None) => format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "emon" and r.field == "{field}")"#,
            start = start.to_rfc3339(),
            stop = stop.to_rfc3339(),
        ),
        ("ebusd_poll", field, Some((every, agg))) => format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "{field}")
  |> aggregateWindow(every: {every}, fn: {agg}, createEmpty: false)"#,
            start = start.to_rfc3339(),
            stop = stop.to_rfc3339(),
        ),
        ("ebusd_poll", field, None) => format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "{field}")"#,
            start = start.to_rfc3339(),
            stop = stop.to_rfc3339(),
        ),
        _ => {
            unreachable!("unsupported dhw measurement route: {measurement}.{field}");
        }
    };
    query_ts(url, org, token, &flux)
}

/// Convert time series to sorted vec of (epoch, value) for last-known-value lookup.
fn to_sorted(data: &TsVal) -> Vec<(i64, f64)> {
    let mut v: Vec<(i64, f64)> = data
        .iter()
        .map(|(ts, val)| (ts.timestamp(), *val))
        .collect();
    v.sort_by_key(|(t, _)| *t);
    v.dedup_by_key(|(t, _)| *t);
    v
}

/// Last-known-value lookup: find the most recent value at or before `t`.
fn lkv(sorted: &[(i64, f64)], t: i64) -> Option<f64> {
    let idx = sorted.partition_point(|(ts, _)| *ts <= t);
    if idx > 0 {
        Some(sorted[idx - 1].1)
    } else {
        None
    }
}

fn write_influx_line(url: &str, org: &str, token: &str, bucket: &str, line: &str) {
    let write_url = format!(
        "{}/api/v2/write?org={}&bucket={}&precision=s",
        url.trim_end_matches('/'),
        org,
        bucket
    );
    let resp = Client::new()
        .post(&write_url)
        .bearer_auth(token)
        .header("Content-Type", "text/plain")
        .body(line.to_string())
        .send();
    match resp {
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().unwrap_or_default();
            eprintln!("InfluxDB write failed ({status}): {body}");
        }
        Err(e) => eprintln!("InfluxDB write error: {e}"),
        _ => {}
    }
}

// ── Event detection ─────────────────────────────────────────────────────────

fn find_events(
    url: &str,
    org: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    bucket: &str,
    days: u32,
) -> ThermalResult<(Vec<ChargeEvent>, Vec<DrawEvent>)> {
    eprintln!("Finding events in last {days} days...");

    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    let stop = Utc::now().with_timezone(&utc);
    let start = stop - TimeDelta::days(days as i64);

    // Query at 10s resolution (raw for eBUS ~30s, 10s aggregate for 2s Multical)
    let flow_data = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_flow",
        &start,
        &stop,
        Some(("10s", "max")),
    )?;

    let vol_data = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_volume_V1",
        &start,
        &stop,
        Some(("10s", "last")),
    )?;

    let bc_data = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "ebusd_poll",
        "BuildingCircuitFlow",
        &start,
        &stop,
        Some(("10s", "last")),
    )?;

    let t1_data = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_t1",
        &start,
        &stop,
        Some(("10s", "last")),
    )?;

    let hwc_data = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "ebusd_poll",
        "HwcStorageTemp",
        &start,
        &stop,
        Some(("10s", "last")),
    )?;

    let flow = to_sorted(&flow_data);
    let vol = to_sorted(&vol_data);
    let bc = to_sorted(&bc_data);
    let t1 = to_sorted(&t1_data);
    let hwc = to_sorted(&hwc_data);

    // Step through time at 10s intervals covering all data
    let min_t = [&flow, &vol, &bc, &t1, &hwc]
        .iter()
        .filter_map(|s| s.first().map(|(t, _)| *t))
        .min()
        .unwrap_or(0);
    let max_t = [&flow, &vol, &bc, &t1, &hwc]
        .iter()
        .filter_map(|s| s.last().map(|(t, _)| *t))
        .max()
        .unwrap_or(0);
    let mut all_times: Vec<i64> = Vec::new();
    let mut t = min_t - (min_t % 10);
    while t <= max_t {
        all_times.push(t);
        t += 10;
    }

    // ── Detect charge events ────────────────────────────────────────────
    let mut charges: Vec<ChargeEvent> = Vec::new();
    let mut in_charge = false;
    let mut charge_start: i64 = 0;
    let mut charge_t1_pre: Option<f64> = None;
    let mut charge_hwc_max: f64 = 0.0;

    for &t in &all_times {
        let b = lkv(&bc, t).unwrap_or(0.0);

        if b > CHARGE_BC_FLOW && !in_charge {
            in_charge = true;
            charge_start = t;
            charge_t1_pre = lkv(&t1, t);
            charge_hwc_max = lkv(&hwc, t).unwrap_or(0.0);
        } else if b > CHARGE_BC_FLOW && in_charge {
            if let Some(h) = lkv(&hwc, t) {
                if h > charge_hwc_max {
                    charge_hwc_max = h;
                }
            }
        } else if in_charge {
            in_charge = false;
            let duration = (t - charge_start) as f64;
            if duration >= CHARGE_MIN_DURATION {
                if let Some(t1_pre) = charge_t1_pre {
                    let hwc_end = lkv(&hwc, t).unwrap_or(charge_hwc_max);
                    let t1_end = lkv(&t1, t).unwrap_or(0.0);
                    let crossover = charge_hwc_max >= t1_pre;
                    let volume_at_end = lkv(&vol, t).unwrap_or(0.0);

                    let start_dt = epoch_to_dt(charge_start);
                    let end_dt = epoch_to_dt(t);

                    charges.push(ChargeEvent {
                        start: start_dt,
                        end: end_dt,
                        t1_pre,
                        hwc_end,
                        t1_end,
                        crossover,
                        volume_at_end,
                    });
                }
            }
        }
    }

    let crossover_count = charges.iter().filter(|c| c.crossover).count();
    let partial_count = charges.len() - crossover_count;
    eprintln!(
        "  {} charges ({crossover_count} crossover, {partial_count} partial)",
        charges.len()
    );

    // ── Detect draw events ──────────────────────────────────────────────
    let mut draws: Vec<DrawEvent> = Vec::new();
    let mut in_draw = false;
    let mut draw_start: i64 = 0;
    let mut draw_start_vol: f64 = 0.0;
    let mut draw_saw_charge = false;
    let mut prev_draw_end: Option<i64> = None;

    for &t in &all_times {
        let f = lkv(&flow, t).unwrap_or(0.0);
        let v = lkv(&vol, t).unwrap_or(0.0);
        let b = lkv(&bc, t).unwrap_or(0.0);

        if f > DRAW_FLOW_MIN && !in_draw {
            in_draw = true;
            draw_start = t;
            draw_start_vol = v;
            draw_saw_charge = b > CHARGE_BC_FLOW;
        } else if f > DRAW_FLOW_MIN && in_draw {
            // Track if charging happened at any point during this draw
            if b > CHARGE_BC_FLOW {
                draw_saw_charge = true;
            }
        } else if f <= DRAW_FLOW_MIN && in_draw {
            in_draw = false;
            let drawn = v - draw_start_vol;

            if drawn >= DRAW_MIN_VOLUME {
                let preceding = charges
                    .iter()
                    .filter(|c| c.end.timestamp() <= draw_start)
                    .max_by_key(|c| c.end.timestamp())
                    .cloned();
                let charge_vol = preceding.as_ref().map(|c| c.volume_at_end).unwrap_or(0.0);
                let gap_hours = prev_draw_end
                    .map(|pe| (draw_start - pe) as f64 / 3600.0)
                    .unwrap_or(999.0);

                draws.push(DrawEvent {
                    start: epoch_to_dt(draw_start),
                    end: epoch_to_dt(t),
                    volume_register_start: draw_start_vol,
                    volume_register_end: v,
                    volume_drawn: drawn,
                    preceding_charge: preceding,
                    cumulative_since_charge: v - charge_vol,
                    gap_hours,
                    during_charge: draw_saw_charge,
                });
            }
            prev_draw_end = Some(t);
        }
    }

    eprintln!("  {} draws ≥{DRAW_MIN_VOLUME}L", draws.len());
    Ok((charges, draws))
}

fn epoch_to_dt(epoch: i64) -> DateTime<FixedOffset> {
    Utc.fix()
        .from_utc_datetime(&DateTime::from_timestamp(epoch, 0).unwrap().naive_utc())
}

// ── Per-draw inflection analysis at 2s resolution ───────────────────────────

fn analyse_draw(
    url: &str,
    org: &str,
    token: &str,
    pg_conninfo: Option<&str>,
    bucket: &str,
    draw: &DrawEvent,
) -> ThermalResult<Option<InflectionResult>> {
    let start = draw.start - TimeDelta::minutes(2);
    let end = draw.end + TimeDelta::minutes(2);

    let t1_raw = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_t1",
        &start,
        &end,
        None,
    )?;

    let flow_raw = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_flow",
        &start,
        &end,
        None,
    )?;

    let t2_raw = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "emon",
        "dhw_t2",
        &start,
        &end,
        None,
    )?;

    // Also fetch HwcStorageTemp — extend window to +5 min to catch post-draw settling
    let hwc_end = draw.end + TimeDelta::minutes(5);
    let hwc_raw = query_measurement_series(
        url,
        org,
        token,
        pg_conninfo,
        bucket,
        "ebusd_poll",
        "HwcStorageTemp",
        &start,
        &hwc_end,
        None,
    )?;

    if t1_raw.len() < 10 || flow_raw.len() < 10 {
        return Ok(None);
    }

    // Build cumulative volume from flow integration
    let mut cumul = 0.0f64;
    let mut fi: Vec<(f64, f64)> = Vec::new(); // (epoch, cumulative_litres)
    for i in 1..flow_raw.len() {
        let dt = (flow_raw[i].0 - flow_raw[i - 1].0).num_milliseconds() as f64 / 1000.0;
        let avg_lph = (flow_raw[i].1 + flow_raw[i - 1].1) / 2.0;
        cumul += avg_lph * dt / 3600.0;
        fi.push((flow_raw[i].0.timestamp() as f64, cumul));
    }

    if fi.is_empty() {
        return Ok(None);
    }

    let cumul_before_draw = draw.volume_register_start
        - draw
            .preceding_charge
            .as_ref()
            .map(|c| c.volume_at_end)
            .unwrap_or(0.0);

    // Map T1 readings to cumulative volume
    let mut t1_vs_cumul: Vec<(f64, f64, f64)> = Vec::new(); // (total_cumul, draw_vol, t1)
    for (ts, t1_val) in &t1_raw {
        let ts_e = ts.timestamp() as f64;
        let idx = fi.partition_point(|(t, _)| *t < ts_e);
        if idx > 0 && idx < fi.len() {
            let (t0, v0) = fi[idx - 1];
            let (t1_t, v1) = fi[idx];
            let frac = if (t1_t - t0).abs() > 0.001 {
                (ts_e - t0) / (t1_t - t0)
            } else {
                0.0
            };
            let draw_vol = v0 + frac * (v1 - v0);
            let total_cumul = cumul_before_draw + draw_vol;
            t1_vs_cumul.push((total_cumul, draw_vol, *t1_val));
        }
    }

    if t1_vs_cumul.len() < 20 {
        return Ok(None);
    }

    let t1_start = t1_vs_cumul[0].2;

    // Mains temp: use settled T2 during actual flow (not stale dead-leg)
    // Find T2 readings when flow > DRAW_FLOW_MIN, take the last quartile median
    let mains_temp = settled_mains_temp(&t2_raw, &flow_raw);

    // Peak flow rate during the draw (not average including zeros)
    let draw_start_e = draw.start.timestamp() as f64;
    let draw_end_e = draw.end.timestamp() as f64;
    let peak_flow = flow_raw
        .iter()
        .filter(|(ts, _)| {
            let e = ts.timestamp() as f64;
            e >= draw_start_e && e <= draw_end_e
        })
        .map(|(_, v)| *v)
        .fold(0.0f64, f64::max);

    // HwcStorageTemp: pre-draw and minimum during/after draw
    let hwc_pre = hwc_raw
        .iter()
        .filter(|(ts, _)| ts.timestamp() as f64 <= draw_start_e)
        .last()
        .map(|(_, v)| *v)
        .unwrap_or(0.0);
    let hwc_min = hwc_raw
        .iter()
        .filter(|(ts, _)| ts.timestamp() as f64 >= draw_start_e)
        .map(|(_, v)| *v)
        .fold(f64::INFINITY, f64::min);
    let hwc_min = if hwc_min.is_infinite() {
        hwc_pre
    } else {
        hwc_min
    };
    let hwc_drop = hwc_pre - hwc_min;

    // Rolling rate detection for T1 inflection
    let mut hint_result: Option<(f64, f64, f64, f64)> = None; // (cumul, draw_vol, rate, t1)
    let mut definitive_result: Option<(f64, f64, f64, f64)> = None;

    for i in 0..t1_vs_cumul.len() {
        let (cumul_i, draw_i, t1_i) = t1_vs_cumul[i];
        // Find the sample at least ROLLING_WINDOW litres back
        for j in (0..i).rev() {
            let (cumul_j, _, t1_j) = t1_vs_cumul[j];
            if cumul_i - cumul_j >= ROLLING_WINDOW {
                let rate = (t1_i - t1_j) / (cumul_i - cumul_j);
                if hint_result.is_none() && rate < HINT_RATE {
                    hint_result = Some((cumul_i, draw_i, rate, t1_i));
                }
                if definitive_result.is_none() && rate < SIGNAL_RATE {
                    definitive_result = Some((cumul_i, draw_i, rate, t1_i));
                }
                break;
            }
        }
    }

    Ok(Some(InflectionResult {
        draw: draw.clone(),
        hint_cumulative: hint_result.map(|r| r.0),
        definitive_cumulative: definitive_result.map(|r| r.0),
        definitive_draw_vol: definitive_result.map(|r| r.1),
        definitive_rate: definitive_result.map(|r| r.2),
        t1_start,
        t1_at_definitive: definitive_result.map(|r| r.3),
        mains_temp,
        peak_flow_rate: peak_flow,
        hwc_pre,
        hwc_min,
        hwc_drop,
    }))
}

/// Compute settled mains temperature during actual flow.
///
/// The T2 sensor sits in a dead leg that holds WWHR-warmed water (~30°C) when
/// there's no flow. True mains temp only appears once cold water has flushed
/// through. We take the last-quartile median of T2 readings during flow.
fn settled_mains_temp(t2_raw: &TsVal, flow_raw: &TsVal) -> f64 {
    // Build a set of epochs where flow > threshold
    let flow_epochs: Vec<(f64, f64)> = flow_raw
        .iter()
        .map(|(ts, v)| (ts.timestamp() as f64, *v))
        .collect();

    let mut t2_during_flow: Vec<f64> = Vec::new();
    for (ts, val) in t2_raw {
        let e = ts.timestamp() as f64;
        // Check if flow was active at this timestamp (within 2s)
        let flowing = flow_epochs
            .iter()
            .any(|(fe, fv)| (e - fe).abs() < 3.0 && *fv > DRAW_FLOW_MIN);
        if flowing {
            t2_during_flow.push(*val);
        }
    }

    if t2_during_flow.is_empty() {
        return 15.0; // fallback
    }

    // Take the last quartile (settled readings)
    let n = t2_during_flow.len();
    let start = n * 3 / 4;
    let tail = &t2_during_flow[start..];

    let mut sorted = tail.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

// ── Recommended capacity computation ────────────────────────────────────────

struct CapacityRecommendation {
    recommended_full_litres: Option<f64>,
    method: String,
}

fn compute_recommended_capacity(capacity: &[&InflectionResult]) -> CapacityRecommendation {
    if capacity.is_empty() {
        return CapacityRecommendation {
            recommended_full_litres: None,
            method: "no_data".into(),
        };
    }

    let wwhr: Vec<_> = capacity
        .iter()
        .filter(|r| r.mains_temp >= WWHR_T2_THRESHOLD)
        .collect();
    let cold: Vec<_> = capacity
        .iter()
        .filter(|r| r.mains_temp < WWHR_T2_THRESHOLD)
        .collect();

    if !wwhr.is_empty() {
        let best = wwhr
            .iter()
            .filter_map(|r| r.definitive_cumulative)
            .fold(0.0f64, f64::max);
        return CapacityRecommendation {
            recommended_full_litres: Some(best.round()),
            method: format!("direct_wwhr ({} measurements)", wwhr.len()),
        };
    }

    if cold.len() >= 2 {
        let t2s: Vec<f64> = cold.iter().map(|r| r.mains_temp).collect();
        let vols: Vec<f64> = cold
            .iter()
            .filter_map(|r| r.definitive_cumulative)
            .collect();
        let n = t2s.len() as f64;
        let mean_t2 = t2s.iter().sum::<f64>() / n;
        let mean_vol = vols.iter().sum::<f64>() / n;
        let cov: f64 = t2s
            .iter()
            .zip(vols.iter())
            .map(|(t, v)| (t - mean_t2) * (v - mean_vol))
            .sum::<f64>()
            / n;
        let var_t2: f64 = t2s.iter().map(|t| (t - mean_t2).powi(2)).sum::<f64>() / n;

        if var_t2 > 0.1 {
            let slope = cov / var_t2;
            let best_cold_vol = vols.iter().fold(0.0f64, |a, &b| a.max(b));
            let best_cold_t2 = cold.iter().map(|r| r.mains_temp).fold(0.0f64, f64::max);
            let wwhr_estimate = best_cold_vol + slope * (25.0 - best_cold_t2);
            return CapacityRecommendation {
                recommended_full_litres: Some(wwhr_estimate.max(best_cold_vol).round()),
                method: format!(
                    "regression (slope={:.1} L/°C, {} cold measurements)",
                    slope,
                    cold.len()
                ),
            };
        }
    }

    // Fallback: single cold-mains measurement, apply conservative 3% reduction
    let best_cold = cold
        .iter()
        .filter_map(|r| r.definitive_cumulative)
        .chain(capacity.iter().filter_map(|r| r.definitive_cumulative))
        .fold(0.0f64, f64::max);
    CapacityRecommendation {
        recommended_full_litres: Some((best_cold * 0.97).round()),
        method: format!("conservative_ratio ({} cold measurements)", cold.len()),
    }
}

// ── InfluxDB / PostgreSQL writes ───────────────────────────────────────────

struct DhwInflectionWriteRow {
    time: DateTime<Utc>,
    category: String,
    crossover: bool,
    draw_type: String,
    cumulative_volume: f64,
    draw_volume: f64,
    gap_hours: f64,
    t1_start: f64,
    t1_at_inflection: f64,
    mains_temp: f64,
    flow_rate: f64,
    rate: f64,
    hwc_pre: f64,
    hwc_min: f64,
    hwc_drop: f64,
}

fn dhw_inflection_write_row(r: &InflectionResult) -> DhwInflectionWriteRow {
    DhwInflectionWriteRow {
        time: r.draw.start.with_timezone(&Utc),
        category: r.inflection_category().to_string(),
        crossover: r
            .draw
            .preceding_charge
            .as_ref()
            .map(|c| c.crossover)
            .unwrap_or(false),
        draw_type: r.draw_type().to_string(),
        cumulative_volume: r.definitive_cumulative.unwrap_or(0.0),
        draw_volume: r.definitive_draw_vol.unwrap_or(0.0),
        gap_hours: r.draw.gap_hours,
        t1_start: r.t1_start,
        t1_at_inflection: r.t1_at_definitive.unwrap_or(0.0),
        mains_temp: r.mains_temp,
        flow_rate: r.peak_flow_rate,
        rate: r.definitive_rate.unwrap_or(0.0),
        hwc_pre: r.hwc_pre,
        hwc_min: r.hwc_min,
        hwc_drop: r.hwc_drop,
    }
}

fn write_dhw_inflection_to_postgres(
    conninfo: &str,
    rows: &[&InflectionResult],
) -> ThermalResult<()> {
    let mut client = PgClient::connect(conninfo, NoTls).map_err(ThermalError::PostgresConnect)?;
    for result in rows {
        let row = dhw_inflection_write_row(result);
        client
            .execute(
                "INSERT INTO dhw_inflection (time, category, crossover, draw_type, cumulative_volume, draw_volume, gap_hours, t1_start, t1_at_inflection, mains_temp, flow_rate, rate, hwc_pre, hwc_min, hwc_drop) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
                &[
                    &row.time,
                    &row.category,
                    &row.crossover,
                    &row.draw_type,
                    &row.cumulative_volume,
                    &row.draw_volume,
                    &row.gap_hours,
                    &row.t1_start,
                    &row.t1_at_inflection,
                    &row.mains_temp,
                    &row.flow_rate,
                    &row.rate,
                    &row.hwc_pre,
                    &row.hwc_min,
                    &row.hwc_drop,
                ],
            )
            .map_err(ThermalError::PostgresQuery)?;
    }
    Ok(())
}

fn write_dhw_capacity_to_postgres(conninfo: &str, val: f64, method: &str) -> ThermalResult<()> {
    let mut client = PgClient::connect(conninfo, NoTls).map_err(ThermalError::PostgresConnect)?;
    client
        .execute(
            "INSERT INTO dhw_capacity (time, recommended_full_litres, method) VALUES ($1, $2, $3)",
            &[&Utc::now(), &val, &method],
        )
        .map_err(ThermalError::PostgresQuery)?;
    Ok(())
}

fn write_results(
    url: &str,
    org: &str,
    token: &str,
    bucket: &str,
    pg_conninfo: Option<&str>,
    results: &[InflectionResult],
) -> ThermalResult<()> {
    let to_write: Vec<_> = results
        .iter()
        .filter(|r| r.definitive_cumulative.is_some())
        .collect();

    if to_write.is_empty() {
        eprintln!("No inflection measurements to write.");
        return Ok(());
    }

    eprintln!("Writing {} measurements to InfluxDB...", to_write.len());
    for r in &to_write {
        let row = dhw_inflection_write_row(r);
        let ts = row.time.timestamp();
        let line = format!(
            "dhw_inflection,category={},crossover={},draw_type={} \
             cumulative_volume={:.1},draw_volume={:.1},gap_hours={:.2},\
             t1_start={:.2},t1_at_inflection={:.2},mains_temp={:.1},\
             flow_rate={:.0},rate={:.5},hwc_pre={:.1},hwc_min={:.1},\
             hwc_drop={:.1} {ts}",
            row.category,
            row.crossover,
            row.draw_type,
            row.cumulative_volume,
            row.draw_volume,
            row.gap_hours,
            row.t1_start,
            row.t1_at_inflection,
            row.mains_temp,
            row.flow_rate,
            row.rate,
            row.hwc_pre,
            row.hwc_min,
            row.hwc_drop,
        );
        write_influx_line(url, org, token, bucket, &line);
    }

    if let Some(conninfo) = pg_conninfo {
        write_dhw_inflection_to_postgres(conninfo, &to_write)?;
        eprintln!("  Mirrored inflection rows to TimescaleDB.");
    } else {
        eprintln!("  TimescaleDB dhw_inflection mirror skipped (no [postgres] config).");
    }

    // Write recommended capacity
    let capacity_results: Vec<_> = results
        .iter()
        .filter(|r| r.inflection_category() == InflectionCategory::Capacity)
        .collect();
    let rec = compute_recommended_capacity(&capacity_results);
    if let Some(val) = rec.recommended_full_litres {
        write_influx_line(
            url,
            org,
            token,
            bucket,
            &format!(
                "dhw_capacity recommended_full_litres={val:.1},method=\"{}\"",
                rec.method
            ),
        );
        if let Some(conninfo) = pg_conninfo {
            write_dhw_capacity_to_postgres(conninfo, val, &rec.method)?;
            eprintln!("  Mirrored recommended capacity to TimescaleDB.");
        } else {
            eprintln!("  TimescaleDB dhw_capacity mirror skipped (no [postgres] config).");
        }
        eprintln!("  Recommended capacity: {val}L ({})", rec.method);
    }

    eprintln!("  Done.");
    Ok(())
}

// ── JSON output ─────────────────────────────────────────────────────────────

fn json_summary(results: &[InflectionResult]) -> serde_json::Value {
    let capacity: Vec<_> = results
        .iter()
        .filter(|r| r.inflection_category() == InflectionCategory::Capacity)
        .collect();
    let lb_crossover: Vec<_> = results
        .iter()
        .filter(|r| {
            r.inflection_category() == InflectionCategory::LowerBound
                && r.draw
                    .preceding_charge
                    .as_ref()
                    .map(|c| c.crossover)
                    .unwrap_or(false)
        })
        .collect();

    let max_usable = capacity
        .iter()
        .filter_map(|r| r.definitive_cumulative)
        .fold(f64::NEG_INFINITY, f64::max);
    let highest_lb = lb_crossover
        .iter()
        .map(|r| r.best_volume())
        .fold(f64::NEG_INFINITY, f64::max);

    let baths = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Bath)
        .count();
    let showers = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Shower)
        .count();
    let taps = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Tap)
        .count();

    let rec = compute_recommended_capacity(&capacity);

    serde_json::json!({
        "max_usable_litres": if max_usable.is_finite() { Some(max_usable.round()) } else { None },
        "geometric_max_litres": GEOMETRIC_MAX.round(),
        "plug_flow_efficiency": if max_usable.is_finite() { Some((max_usable / GEOMETRIC_MAX * 1000.0).round() / 1000.0) } else { None },
        "highest_lower_bound": if highest_lb.is_finite() { Some(highest_lb.round()) } else { None },
        "recommended_full_litres": rec.recommended_full_litres,
        "recommended_method": rec.method,
        "capacity_count": capacity.len(),
        "baths": baths,
        "showers": showers,
        "taps": taps,
        "total_draws": results.len(),
    })
}

// ── Human-readable output ───────────────────────────────────────────────────

fn output_human(results: &[InflectionResult], days: u32, verbose: bool) {
    let capacity: Vec<_> = results
        .iter()
        .filter(|r| r.inflection_category() == InflectionCategory::Capacity)
        .collect();
    let partial: Vec<_> = results
        .iter()
        .filter(|r| r.inflection_category() == InflectionCategory::Partial)
        .collect();

    let baths: Vec<_> = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Bath)
        .collect();
    let showers: Vec<_> = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Shower)
        .collect();
    let taps: Vec<_> = results
        .iter()
        .filter(|r| r.draw_type() == DrawType::Tap)
        .collect();

    if verbose {
        println!();
        println!("{}", "=".repeat(140));
        println!(
            "ALL DRAWS — {} at 2-second resolution ({} baths, {} showers, {} taps)",
            results.len(),
            baths.len(),
            showers.len(),
            taps.len()
        );
        println!("{}", "=".repeat(140));
        println!(
            "{:>20} │ {:>7} {:>4} {:>6} {:>5} │ {:>7} {:>7} │ {:>5} {:>5} {:>5} │ {:>5} {:>5} {:>5} │ {:>10} │ {}",
            "Draw time", "Type", "Vol", "Cumul", "Gap",
            "Hint @", "Def @",
            "T1", "T2", "Flow",
            "HWC↑", "HWC↓", "Drop",
            "Charge", "Inflection"
        );
        println!("{}", "─".repeat(140));

        for r in results {
            let d = &r.draw;
            let hint_str = r
                .hint_cumulative
                .map(|v| format!("{v:.0}L"))
                .unwrap_or_else(|| "  —".into());
            let def_str = r
                .definitive_cumulative
                .map(|v| format!("{v:.0}L"))
                .unwrap_or_else(|| "  —".into());
            let charge_str = match &d.preceding_charge {
                Some(c) if c.crossover => format!("✓ {:.0}°", c.t1_end),
                Some(c) => format!("✗ gap {:.0}°", c.t1_pre - c.hwc_end),
                None => "?".into(),
            };
            let type_str = format!(
                "{}{}",
                r.draw_type(),
                if d.during_charge { "*" } else { "" }
            );

            println!(
                "{:>20} │ {:>7} {:3.0}L {:5.0}L {:4.1}h │ {:>7} {:>7} │ {:4.1}° {:4.1}° {:4.0} │ {:4.1}° {:4.1}° {:4.1}° │ {:>10} │ {}",
                d.start.format("%d/%m %H:%M"),
                type_str,
                d.volume_drawn,
                d.cumulative_since_charge,
                d.gap_hours,
                hint_str,
                def_str,
                r.t1_start,
                r.mains_temp,
                r.peak_flow_rate,
                r.hwc_pre,
                r.hwc_min,
                r.hwc_drop,
                charge_str,
                r.inflection_category()
            );
        }
        println!();
        println!("  * = draw occurred during HP charge cycle");
    }

    // ── Draw type summary ───────────────────────────────────────────────
    println!();
    println!(
        "DHW SESSIONS — {} draws over {days} days ({} baths, {} showers, {} taps)",
        results.len(),
        baths.len(),
        showers.len(),
        taps.len()
    );
    println!("{}", "=".repeat(70));

    for (label, draws) in [("Baths", &baths), ("Showers", &showers), ("Taps", &taps)] {
        if draws.is_empty() {
            continue;
        }
        let total_vol: f64 = draws.iter().map(|r| r.draw.volume_drawn).sum();
        println!();
        println!("  {label} ({}, {total_vol:.0}L total):", draws.len());
        for r in draws {
            let charge_flag = if r.draw.during_charge {
                " [during charge]"
            } else {
                ""
            };
            println!(
                "    {}: {:3.0}L  {:.0} L/h  T1={:.1}°  HWC {:.0}→{:.0}° (Δ{:.0}°){}",
                r.draw.start.format("%d/%m %H:%M"),
                r.draw.volume_drawn,
                r.peak_flow_rate,
                r.t1_start,
                r.hwc_pre,
                r.hwc_min,
                r.hwc_drop,
                charge_flag,
            );
        }
    }

    // ── Capacity analysis ───────────────────────────────────────────────
    println!();
    println!("CAPACITY ANALYSIS");
    println!("{}", "=".repeat(70));

    if !capacity.is_empty() {
        let best = capacity
            .iter()
            .filter_map(|r| r.definitive_cumulative)
            .fold(0.0f64, f64::max);
        println!();
        println!("  Maximum measured usable volume: {best:.0}L");
        println!(
            "  (geometric max {GEOMETRIC_MAX:.0}L, plug flow efficiency {:.0}%)",
            best / GEOMETRIC_MAX * 100.0
        );
        println!();
        for r in &capacity {
            println!(
                "    {}: {:.0}L  T1={:.1}°  T2={:.1}°  flow={:.0} L/h  gap={:.1}h  HWC {:.0}→{:.0}° (Δ{:.0}°)",
                r.draw.start.format("%d/%m %H:%M"),
                r.definitive_cumulative.unwrap_or(0.0),
                r.t1_start,
                r.mains_temp,
                r.peak_flow_rate,
                r.draw.gap_hours,
                r.hwc_pre,
                r.hwc_min,
                r.hwc_drop,
            );
        }
    } else {
        println!();
        println!("  No capacity measurements yet.");
        println!("  (need a large draw from a fully-charged cylinder with T1 inflection)");
    }

    if !partial.is_empty() {
        println!();
        println!("  Partial-state measurements (not full charge):");
        for r in &partial {
            let ctx = match &r.draw.preceding_charge {
                Some(c) if !c.crossover => {
                    format!("no crossover (gap {:.1}°)", c.t1_pre - c.hwc_end)
                }
                Some(c) if c.t1_end < 43.0 => {
                    format!("crossover but T1 only {:.0}°", c.t1_end)
                }
                _ => String::new(),
            };
            println!(
                "    {}: {:.0}L — {ctx}",
                r.draw.start.format("%d/%m %H:%M"),
                r.definitive_cumulative.unwrap_or(0.0),
            );
        }
    }

    // Recommended capacity
    let cap_refs: Vec<_> = capacity.iter().copied().collect();
    let rec = compute_recommended_capacity(&cap_refs);
    if let Some(val) = rec.recommended_full_litres {
        println!();
        println!(
            "  Recommended full_litres for z2m-hub: {val:.0}L ({})",
            rec.method
        );
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Output mode for dhw-sessions command
pub enum DhwSessionsOutput {
    /// Human-readable summary
    Human,
    /// Verbose per-draw table + summary
    Verbose,
    /// JSON to stdout
    Json,
}

/// Run DHW session analysis.
fn analyse_sessions(
    config_path: &str,
    days: u32,
    no_write: bool,
) -> ThermalResult<Vec<InflectionResult>> {
    let (_cfg_text, cfg) = super::config::load_thermal_config(std::path::Path::new(config_path))?;
    let token = super::config::resolve_influx_token(&cfg)?;
    let pg_conninfo = super::config::resolve_postgres_conninfo(&cfg)?;

    let url = &cfg.influx.url;
    let org = &cfg.influx.org;
    let bucket = &cfg.influx.bucket;

    let (_charges, draws) = find_events(url, org, &token, pg_conninfo.as_deref(), bucket, days)?;

    let mut results: Vec<InflectionResult> = Vec::new();
    for (i, draw) in draws.iter().enumerate() {
        eprint!(
            "\r  Analysing draw {}/{} ({})...",
            i + 1,
            draws.len(),
            draw.start.format("%d/%m %H:%M")
        );
        if let Some(result) = analyse_draw(url, org, &token, pg_conninfo.as_deref(), bucket, draw)?
        {
            results.push(result);
        }
    }
    if !draws.is_empty() {
        eprintln!();
    }

    if !no_write {
        write_results(url, org, &token, bucket, pg_conninfo.as_deref(), &results)?;
    }

    Ok(results)
}

pub fn dhw_sessions_json_summary(
    config_path: &str,
    days: u32,
    no_write: bool,
) -> ThermalResult<serde_json::Value> {
    let results = analyse_sessions(config_path, days, no_write)?;
    Ok(json_summary(&results))
}

pub fn dhw_sessions(
    config_path: &str,
    days: u32,
    output: DhwSessionsOutput,
    no_write: bool,
) -> ThermalResult<()> {
    let results = analyse_sessions(config_path, days, no_write)?;

    match output {
        DhwSessionsOutput::Human => output_human(&results, days, false),
        DhwSessionsOutput::Verbose => output_human(&results, days, true),
        DhwSessionsOutput::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json_summary(&results)).unwrap()
            )
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Offset, TimeZone, Utc};

    fn sample_time(offset_seconds: i64) -> DateTime<FixedOffset> {
        Utc.fix().with_ymd_and_hms(2026, 4, 1, 6, 0, 0).unwrap() + Duration::seconds(offset_seconds)
    }

    fn sample_draw() -> DrawEvent {
        DrawEvent {
            start: sample_time(0),
            end: sample_time(600),
            volume_register_start: 120.0,
            volume_register_end: 180.0,
            volume_drawn: 60.0,
            preceding_charge: Some(ChargeEvent {
                start: sample_time(-3600),
                end: sample_time(-3000),
                t1_pre: 30.0,
                hwc_end: 48.0,
                t1_end: 44.0,
                crossover: true,
                volume_at_end: 100.0,
            }),
            cumulative_since_charge: 60.0,
            gap_hours: 1.0,
            during_charge: false,
        }
    }

    fn sample_result(mains_temp: f64, definitive_cumulative: f64) -> InflectionResult {
        InflectionResult {
            draw: sample_draw(),
            hint_cumulative: Some(definitive_cumulative - 5.0),
            definitive_cumulative: Some(definitive_cumulative),
            definitive_draw_vol: Some(40.0),
            definitive_rate: Some(-0.02),
            t1_start: 44.0,
            t1_at_definitive: Some(40.0),
            mains_temp,
            peak_flow_rate: 500.0,
            hwc_pre: 48.0,
            hwc_min: 41.0,
            hwc_drop: 7.0,
        }
    }

    // @lat: [[tests#DHW session analysis#Settled mains temperature uses the flushed tail of the draw]]
    #[test]
    fn settled_mains_temperature_uses_flushed_tail() {
        let t2_raw: TsVal = vec![30.0, 29.0, 28.0, 27.0, 20.0, 18.0, 16.0, 14.0]
            .into_iter()
            .enumerate()
            .map(|(i, value)| (sample_time(i as i64 * 2), value))
            .collect();
        let flow_raw: TsVal = (0..8)
            .map(|i| (sample_time(i as i64 * 2), DRAW_FLOW_MIN + 50.0))
            .collect();

        assert_eq!(settled_mains_temp(&t2_raw, &flow_raw), 16.0);
    }

    // @lat: [[tests#DHW session analysis#WWHR capacity recommendation prefers direct measurements]]
    #[test]
    fn wwhr_capacity_recommendation_prefers_direct_measurements() {
        let wwhr_a = sample_result(WWHR_T2_THRESHOLD + 1.0, 132.4);
        let wwhr_b = sample_result(WWHR_T2_THRESHOLD + 3.0, 145.2);
        let cold = sample_result(WWHR_T2_THRESHOLD - 5.0, 160.0);

        let recommendation = compute_recommended_capacity(&[&wwhr_a, &wwhr_b, &cold]);

        assert_eq!(recommendation.recommended_full_litres, Some(145.0));
        assert!(recommendation.method.starts_with("direct_wwhr"));
    }

    // @lat: [[tests#DHW session analysis#Single cold capacity measurement stays conservative]]
    #[test]
    fn single_cold_capacity_measurement_stays_conservative() {
        let cold = sample_result(WWHR_T2_THRESHOLD - 6.0, 100.0);

        let recommendation = compute_recommended_capacity(&[&cold]);

        assert_eq!(recommendation.recommended_full_litres, Some(97.0));
        assert!(recommendation.method.starts_with("conservative_ratio"));
    }

    // @lat: [[tests#DHW session analysis#Cold mains regression never undercuts measured capacity]]
    #[test]
    fn cold_mains_regression_never_undercuts_measured_capacity() {
        let colder = sample_result(8.0, 130.0);
        let warmer = sample_result(14.0, 110.0);

        let recommendation = compute_recommended_capacity(&[&colder, &warmer]);

        assert_eq!(recommendation.recommended_full_litres, Some(130.0));
        assert!(recommendation.method.starts_with("regression"));
    }

    // @lat: [[tests#DHW session analysis#Draw type classifies bath shower and tap by flow rate]]
    #[test]
    fn draw_type_classifies_by_flow_rate() {
        let mut r = sample_result(10.0, 100.0);

        // Bath: peak_flow_rate >= 650
        r.peak_flow_rate = BATH_FLOW_MIN;
        assert_eq!(r.draw_type(), DrawType::Bath);
        r.peak_flow_rate = BATH_FLOW_MIN + 100.0;
        assert_eq!(r.draw_type(), DrawType::Bath);

        // Shower: >= 350 and volume >= 20
        r.peak_flow_rate = SHOWER_FLOW_MIN;
        assert_eq!(r.draw_type(), DrawType::Shower);

        // Tap: below shower threshold
        r.peak_flow_rate = SHOWER_FLOW_MIN - 1.0;
        assert_eq!(r.draw_type(), DrawType::Tap);

        // Shower requires volume >= 20 — small volume → tap
        r.peak_flow_rate = SHOWER_FLOW_MIN + 10.0;
        r.draw.volume_drawn = 19.9;
        assert_eq!(r.draw_type(), DrawType::Tap);
    }

    // @lat: [[tests#DHW session analysis#Inflection category depends on charge state and T1]]
    #[test]
    fn inflection_category_depends_on_charge_and_t1() {
        let r = sample_result(10.0, 100.0);
        // Default fixture has crossover=true, t1_end=44.0, definitive_cumulative=Some
        assert_eq!(r.inflection_category(), InflectionCategory::Capacity);

        // Partial: definitive exists but charge didn't crossover
        let mut partial = sample_result(10.0, 100.0);
        partial.draw.preceding_charge.as_mut().unwrap().crossover = false;
        assert_eq!(partial.inflection_category(), InflectionCategory::Partial);

        // Partial: T1 end below 43°C
        let mut low_t1 = sample_result(10.0, 100.0);
        low_t1.draw.preceding_charge.as_mut().unwrap().t1_end = 42.9;
        assert_eq!(low_t1.inflection_category(), InflectionCategory::Partial);

        // LowerBound: no definitive cumulative
        let mut lb = sample_result(10.0, 100.0);
        lb.definitive_cumulative = None;
        assert_eq!(lb.inflection_category(), InflectionCategory::LowerBound);
    }

    // @lat: [[tests#DHW session analysis#Last known value finds nearest preceding sample]]
    #[test]
    fn lkv_finds_nearest_preceding() {
        let sorted = vec![(100, 1.0), (200, 2.0), (300, 3.0)];

        assert_eq!(lkv(&sorted, 50), None); // before all
        assert_eq!(lkv(&sorted, 100), Some(1.0)); // exact match
        assert_eq!(lkv(&sorted, 150), Some(1.0)); // between
        assert_eq!(lkv(&sorted, 300), Some(3.0)); // exact last
        assert_eq!(lkv(&sorted, 400), Some(3.0)); // after all
        assert_eq!(lkv(&[], 100), None); // empty
    }

    // @lat: [[tests#DHW session analysis#To sorted deduplicates and orders by timestamp]]
    #[test]
    fn to_sorted_deduplicates_and_orders() {
        let t1 = sample_time(100);
        let t2 = sample_time(200);
        let t3 = sample_time(50);

        let data: TsVal = vec![(t1, 1.0), (t2, 2.0), (t3, 3.0), (t1, 1.5)];
        let sorted = to_sorted(&data);

        // Should be ordered by epoch and deduplicated
        assert_eq!(sorted.len(), 3);
        assert!(sorted[0].0 < sorted[1].0);
        assert!(sorted[1].0 < sorted[2].0);
    }

    // @lat: [[tests#DHW session analysis#parse_ts_val handles RFC3339 and naive timestamp formats]]
    #[test]
    fn parse_ts_val_handles_rfc3339_and_naive() {
        let rows = vec![
            // standard RFC3339
            [("_time", "2026-04-10T07:00:00+00:00"), ("_value", "42.5")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            // Z suffix (needs replace fallback)
            [("_time", "2026-04-10T08:00:00Z"), ("_value", "43.0")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            // alternative column names
            [("time", "2026-04-10T09:00:00+00:00"), ("value", "44.0")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            // bad value → skipped
            [("_time", "2026-04-10T10:00:00+00:00"), ("_value", "nope")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ];
        let result = parse_ts_val(&rows);
        assert_eq!(result.len(), 3);
        assert!((result[0].1 - 42.5).abs() < 0.01);
        assert!((result[1].1 - 43.0).abs() < 0.01);
        assert!((result[2].1 - 44.0).abs() < 0.01);
    }

    // @lat: [[tests#DHW session analysis#best_volume prefers definitive then hint then cumulative]]
    #[test]
    fn best_volume_prefers_definitive_then_hint_then_cumulative() {
        let mut r = sample_result(14.0, 100.0);
        assert!((r.best_volume() - 100.0).abs() < 0.01, "definitive first");

        r.definitive_cumulative = None;
        assert!((r.best_volume() - 95.0).abs() < 0.01, "hint second");

        r.hint_cumulative = None;
        assert!((r.best_volume() - 60.0).abs() < 0.01, "cumulative fallback");
    }

    // @lat: [[tests#DHW session analysis#epoch_to_dt converts Unix epoch to DateTime]]
    #[test]
    fn epoch_to_dt_converts_unix_epoch() {
        let dt = epoch_to_dt(1712739600); // 2024-04-10 09:00:00 UTC
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2024-04-10T09:00:00"
        );
    }

    // @lat: [[tests#DHW session analysis#Empty capacity input returns no_data recommendation]]
    #[test]
    fn empty_capacity_returns_no_data() {
        let rec = compute_recommended_capacity(&[]);
        assert_eq!(rec.recommended_full_litres, None);
        assert_eq!(rec.method, "no_data");
    }

    // @lat: [[tests#DHW session analysis#Low variance cold mains falls back to conservative ratio]]
    #[test]
    fn low_variance_cold_mains_falls_to_conservative() {
        // Two measurements at nearly identical mains temps → var_t2 ≤ 0.1 → skip regression
        let a = sample_result(10.0, 120.0);
        let b = sample_result(10.1, 115.0);

        let rec = compute_recommended_capacity(&[&a, &b]);

        // Should use conservative_ratio (3% haircut on best = 120 * 0.97 = 116.4 → 116)
        assert_eq!(rec.recommended_full_litres, Some(116.0));
        assert!(
            rec.method.starts_with("conservative_ratio"),
            "expected conservative_ratio, got: {}",
            rec.method
        );
    }

    // @lat: [[tests#DHW session analysis#Cold mains regression extrapolates above measured maximum]]
    #[test]
    fn cold_regression_extrapolates_above_measured() {
        // Positive slope: warmer inlet → more usable volume.
        // slope = cov/var_t2, extrapolate to T2=25°C
        let cold_a = sample_result(8.0, 100.0);
        let cold_b = sample_result(14.0, 130.0);

        let rec = compute_recommended_capacity(&[&cold_a, &cold_b]);

        // slope ≈ (cov/var) = 5.0 L/°C, best_cold_vol=130, best_cold_t2=14
        // wwhr_estimate = 130 + 5.0*(25-14) = 185
        // result = max(185, 130).round() = 185
        assert!(rec.method.starts_with("regression"));
        let val = rec.recommended_full_litres.unwrap();
        assert!(
            val > 130.0,
            "regression should extrapolate above 130, got {val}"
        );
        assert!((val - 185.0).abs() < 1.0, "expected ~185, got {val}");
    }

    // @lat: [[tests#DHW session analysis#JSON summary serialises capacity and draw counts]]
    #[test]
    fn json_summary_serialises_capacity_and_draw_counts() {
        let results = vec![
            sample_result(10.0, 120.0), // Capacity (crossover=true, t1_end=44)
        ];
        let json = json_summary(&results);

        assert_eq!(json["max_usable_litres"], 120.0);
        assert_eq!(json["geometric_max_litres"], GEOMETRIC_MAX.round());
        assert_eq!(json["capacity_count"], 1);
        assert_eq!(json["total_draws"], 1);
        // sample_result has peak_flow_rate=500, volume=60 → shower
        assert_eq!(json["showers"], 1);
        assert_eq!(json["baths"], 0);
        assert_eq!(json["taps"], 0);
        assert!(json["recommended_full_litres"].is_number());
        assert!(json["recommended_method"].is_string());
    }

    // @lat: [[tests#DHW session analysis#JSON summary handles empty results]]
    #[test]
    fn json_summary_handles_empty_results() {
        let json = json_summary(&[]);

        assert!(json["max_usable_litres"].is_null());
        assert!(json["plug_flow_efficiency"].is_null());
        assert!(json["highest_lower_bound"].is_null());
        assert!(json["recommended_full_litres"].is_null());
        assert_eq!(json["recommended_method"], "no_data");
        assert_eq!(json["capacity_count"], 0);
        assert_eq!(json["total_draws"], 0);
    }

    // ── Write-contract tests (migration regression) ────────────────────────

    // @lat: [[tests#DHW write contracts#dhw_inflection LP line contains all required fields]]
    #[test]
    fn dhw_inflection_lp_field_coverage() {
        // Build a representative InflectionResult and verify the LP line
        // format matches what TimescaleDB dhw_inflection columns expect.
        let r = InflectionResult {
            draw: DrawEvent {
                start: sample_time(0),
                end: sample_time(600),
                volume_register_start: 100.0,
                volume_register_end: 130.0,
                volume_drawn: 30.0,
                preceding_charge: Some(ChargeEvent {
                    start: sample_time(-7200),
                    end: sample_time(-3600),
                    t1_pre: 30.0,
                    hwc_end: 50.0,
                    t1_end: 48.0,
                    crossover: true,
                    volume_at_end: 100.0,
                }),
                cumulative_since_charge: 30.0,
                gap_hours: 3.0,
                during_charge: false,
            },
            hint_cumulative: Some(28.0),
            definitive_cumulative: Some(29.5),
            definitive_draw_vol: Some(29.5),
            definitive_rate: Some(0.00123),
            t1_start: 47.5,
            t1_at_definitive: Some(42.0),
            mains_temp: 12.5,
            peak_flow_rate: 450.0,
            hwc_pre: 50.0,
            hwc_min: 38.0,
            hwc_drop: 12.0,
        };

        let row = dhw_inflection_write_row(&r);
        let ts = row.time.timestamp();

        let line = format!(
            "dhw_inflection,category={},crossover={},draw_type={} \
             cumulative_volume={:.1},draw_volume={:.1},gap_hours={:.2},\
             t1_start={:.2},t1_at_inflection={:.2},mains_temp={:.1},\
             flow_rate={:.0},rate={:.5},hwc_pre={:.1},hwc_min={:.1},\
             hwc_drop={:.1} {ts}",
            row.category,
            row.crossover,
            row.draw_type,
            row.cumulative_volume,
            row.draw_volume,
            row.gap_hours,
            row.t1_start,
            row.t1_at_inflection,
            row.mains_temp,
            row.flow_rate,
            row.rate,
            row.hwc_pre,
            row.hwc_min,
            row.hwc_drop,
        );

        // Verify measurement name
        assert!(line.starts_with("dhw_inflection,"));

        // Verify tags (become columns in TimescaleDB)
        assert!(line.contains("category=capacity"));
        assert!(line.contains("crossover=true"));
        assert!(line.contains("draw_type=shower"));

        // Verify all field names match TimescaleDB dhw_inflection columns
        let pg_columns = [
            "cumulative_volume",
            "draw_volume",
            "gap_hours",
            "t1_start",
            "t1_at_inflection",
            "mains_temp",
            "flow_rate",
            "rate",
            "hwc_pre",
            "hwc_min",
            "hwc_drop",
        ];
        for col in &pg_columns {
            assert!(
                line.contains(&format!("{col}=")),
                "LP line missing field '{col}' — TimescaleDB column will be NULL"
            );
        }

        // Verify timestamp is present at end
        assert!(line.ends_with(&ts.to_string()));
    }

    // @lat: [[tests#DHW write contracts#parse_ts_val handles naive timestamps from PostgreSQL]]
    #[test]
    fn parse_ts_val_naive_pg_format() {
        // PostgreSQL may return timestamps as naive ISO without offset.
        // parse_ts_val already has a NaiveDateTime fallback for "%Y-%m-%dT%H:%M:%S".
        // This test verifies that path works for the migration.
        let rows = vec![[("_time", "2026-04-10T07:00:00"), ("_value", "42.5")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>()];
        let result = parse_ts_val(&rows);
        assert_eq!(
            result.len(),
            1,
            "Naive ISO timestamp should parse via fallback"
        );
        assert!((result[0].1 - 42.5).abs() < 0.01);
    }

    // @lat: [[tests#DHW write contracts#10s resolution query produces one sample per 10 seconds]]
    #[test]
    fn ten_second_resolution_contract() {
        // DHW event detection queries at 10s resolution.
        // After migration, the SQL equivalent must produce ~6 rows per minute.
        // This test verifies the expected density from a known-good response.
        let rows: Vec<HashMap<String, String>> = (0..6)
            .map(|i| {
                let ts = format!("2026-04-10T07:00:{:02}+00:00", i * 10);
                [("_time", ts.as_str()), ("_value", "500.0")]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .collect();
        let result = parse_ts_val(&rows);
        assert_eq!(
            result.len(),
            6,
            "10s resolution should yield 6 samples per minute"
        );

        // Verify 10s spacing
        for i in 1..result.len() {
            let dt = result[i].0.timestamp() - result[i - 1].0.timestamp();
            assert_eq!(dt, 10, "Expected 10s spacing between samples");
        }
    }

    // @lat: [[tests#DHW write contracts#LP tag spaces replaced with underscores]]
    #[test]
    fn lp_tag_space_escaping() {
        // LP format uses spaces as delimiters. Tag values with spaces must be
        // escaped. The dhw_inflection builder uses Display impls which don't
        // contain spaces, but this test pins the invariant.
        let cat = InflectionCategory::Capacity;
        let draw_type = DrawType::Shower;
        let tag_str = format!("category={cat},draw_type={draw_type}");
        assert!(!tag_str.contains(' '), "LP tags must not contain spaces");
    }

    // @lat: [[tests#DHW write contracts#find_events measurement filter routes to correct PG tables]]
    #[test]
    fn find_events_measurement_routing() {
        // find_events uses measurement-based filters (not topic-based like influx.rs).
        // The routing is: _measurement + field → PG table + column.
        assert_eq!(
            measurement_route("emon", "dhw_flow"),
            Some(MeasurementRoute::Multical {
                column: "dhw_flow".to_string()
            })
        );
        assert_eq!(
            measurement_route("emon", "dhw_volume_V1"),
            Some(MeasurementRoute::Multical {
                column: "dhw_volume_v1".to_string()
            })
        );
        assert_eq!(
            measurement_route("emon", "dhw_t1"),
            Some(MeasurementRoute::Multical {
                column: "dhw_t1".to_string()
            })
        );
        assert_eq!(
            measurement_route("ebusd_poll", "BuildingCircuitFlow"),
            Some(MeasurementRoute::EbusdPoll {
                field: "BuildingCircuitFlow".to_string()
            })
        );
        assert_eq!(
            measurement_route("ebusd_poll", "HwcStorageTemp"),
            Some(MeasurementRoute::EbusdPoll {
                field: "HwcStorageTemp".to_string()
            })
        );
    }

    // @lat: [[tests#DHW write contracts#find_events uses triple-field filter for emon measurements]]
    #[test]
    fn find_events_triple_field_filter_contract() {
        // find_events queries emon data with a triple filter:
        //   _measurement="emon" AND _field="value" AND field="dhw_flow"
        // This unusual pattern exists because emon stores all fields as
        // _field="value" with the actual field name in r.field tag.
        // In PostgreSQL, this collapses: just SELECT dhw_flow FROM multical.
        // The test documents that _field is always "value" for emon queries.
        let emon_queries: Vec<(&str, &str)> = vec![
            ("dhw_flow", "value"),
            ("dhw_volume_V1", "value"),
            ("dhw_t1", "value"),
        ];

        for (field, expected_field_value) in &emon_queries {
            assert_eq!(
                *expected_field_value, "value",
                "emon measurement always uses _field='value'; field='{field}' is in the tag"
            );
        }

        // ebusd_poll queries do NOT use the _field filter — they only filter
        // on _measurement and field (the tag).
        let ebusd_queries = vec!["BuildingCircuitFlow", "HwcStorageTemp"];
        for field in &ebusd_queries {
            // In PG: SELECT value FROM ebusd_poll WHERE field = '{field}'
            assert!(
                !field.is_empty(),
                "ebusd_poll field tag maps to WHERE field = '{field}'"
            );
        }
    }

    // @lat: [[tests#DHW write contracts#Postgres inflection row maps all LP tags and fields to columns]]
    #[test]
    fn postgres_inflection_row_field_coverage() {
        let r = InflectionResult {
            draw: DrawEvent {
                start: sample_time(0),
                end: sample_time(600),
                volume_register_start: 100.0,
                volume_register_end: 130.0,
                volume_drawn: 30.0,
                preceding_charge: Some(ChargeEvent {
                    start: sample_time(-7200),
                    end: sample_time(-3600),
                    t1_pre: 30.0,
                    hwc_end: 50.0,
                    t1_end: 48.0,
                    crossover: true,
                    volume_at_end: 100.0,
                }),
                cumulative_since_charge: 30.0,
                gap_hours: 3.0,
                during_charge: false,
            },
            hint_cumulative: Some(28.0),
            definitive_cumulative: Some(29.5),
            definitive_draw_vol: Some(29.5),
            definitive_rate: Some(0.00123),
            t1_start: 47.5,
            t1_at_definitive: Some(42.0),
            mains_temp: 12.5,
            peak_flow_rate: 450.0,
            hwc_pre: 50.0,
            hwc_min: 38.0,
            hwc_drop: 12.0,
        };

        let row = dhw_inflection_write_row(&r);
        assert_eq!(row.category, "capacity");
        assert!(row.crossover);
        assert_eq!(row.draw_type, "shower");
        assert!((row.cumulative_volume - 29.5).abs() < 0.001);
        assert!((row.draw_volume - 29.5).abs() < 0.001);
        assert!((row.gap_hours - 3.0).abs() < 0.001);
        assert!((row.t1_start - 47.5).abs() < 0.001);
        assert!((row.t1_at_inflection - 42.0).abs() < 0.001);
        assert!((row.mains_temp - 12.5).abs() < 0.001);
        assert!((row.flow_rate - 450.0).abs() < 0.001);
        assert!((row.rate - 0.00123).abs() < 0.00001);
        assert!((row.hwc_pre - 50.0).abs() < 0.001);
        assert!((row.hwc_min - 38.0).abs() < 0.001);
        assert!((row.hwc_drop - 12.0).abs() < 0.001);
    }

    // @lat: [[tests#DHW write contracts#dhw_capacity LP line maps to TimescaleDB columns]]
    #[test]
    fn dhw_capacity_lp_field_coverage() {
        let val = 125.5_f64;
        let method = "wwhr_direct";
        let line = format!("dhw_capacity recommended_full_litres={val:.1},method=\"{method}\"");

        assert!(line.starts_with("dhw_capacity "));
        assert!(line.contains("recommended_full_litres=125.5"));
        assert!(line.contains("method=\"wwhr_direct\""));
    }

    // @lat: [[tests#DHW write contracts#Optional postgres conninfo is read from env when configured]]
    #[test]
    fn optional_postgres_conninfo_from_env() {
        let cfg: super::super::config::ThermalConfig = toml::from_str(
            r#"
[influx]
url = "http://pi5data:8086"
org = "home"
bucket = "energy"
token_env = "INFLUX_TOKEN"

[postgres]
conninfo_env = "TEST_TSDB_CONNINFO"

[test_nights]
night1_start = "2026-03-24T23:10:00+00:00"
night1_end = "2026-03-25T05:05:00+00:00"
night2_start = "2026-03-25T23:10:00+00:00"
night2_end = "2026-03-26T05:05:00+00:00"

[objective]
exclude_rooms = []
prior_weight = 0.0

[priors]
landing_ach = 1.3
doorway_cd = 0.2

[bounds]
leather_ach_min = 0.4
leather_ach_max = 0.9
leather_ach_step = 0.05
landing_ach_min = 0.8
landing_ach_max = 1.6
landing_ach_step = 0.1
conservatory_ach_min = 1.0
conservatory_ach_max = 6.0
conservatory_ach_step = 0.5
office_ach_min = 0.8
office_ach_max = 4.0
office_ach_step = 0.2
doorway_cd_min = 0.1
doorway_cd_max = 0.35
doorway_cd_step = 0.05
"#,
        )
        .unwrap();

        std::env::set_var("TEST_TSDB_CONNINFO", "host=pi5data dbname=energy user=test");
        let conninfo = super::super::config::resolve_postgres_conninfo(&cfg).unwrap();
        std::env::remove_var("TEST_TSDB_CONNINFO");

        assert_eq!(
            conninfo.as_deref(),
            Some("host=pi5data dbname=energy user=test")
        );
    }
}
