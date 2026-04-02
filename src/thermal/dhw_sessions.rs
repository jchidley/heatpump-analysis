//! DHW session analysis — historical draw/charge detection with inflection analysis.
//!
//! Queries InfluxDB at 10s resolution for event detection, then 2s for per-draw
//! inflection analysis. Classifies draws by type (bath/shower/tap), tracks
//! HwcStorageTemp during draws, detects draws during HP charging.
//! Writes `dhw_inflection` + `dhw_capacity` to InfluxDB (z2m-hub autoloads on startup).

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, FixedOffset, Offset, TimeDelta, TimeZone, Utc};
use reqwest::blocking::Client;

use super::error::ThermalResult;
use super::influx::query_flux_csv_pub;

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
    bucket: &str,
    days: u32,
) -> ThermalResult<(Vec<ChargeEvent>, Vec<DrawEvent>)> {
    eprintln!("Finding events in last {days} days...");

    // Query at 10s resolution (raw for eBUS ~30s, 10s aggregate for 2s Multical)
    let flow_data = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_flow")
  |> aggregateWindow(every: 10s, fn: max, createEmpty: false)"#
        ),
    )?;

    let vol_data = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_volume_V1")
  |> aggregateWindow(every: 10s, fn: last, createEmpty: false)"#
        ),
    )?;

    let bc_data = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "BuildingCircuitFlow")
  |> aggregateWindow(every: 10s, fn: last, createEmpty: false)"#
        ),
    )?;

    let t1_data = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_t1")
  |> aggregateWindow(every: 10s, fn: last, createEmpty: false)"#
        ),
    )?;

    let hwc_data = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "HwcStorageTemp")
  |> aggregateWindow(every: 10s, fn: last, createEmpty: false)"#
        ),
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
    bucket: &str,
    draw: &DrawEvent,
) -> ThermalResult<Option<InflectionResult>> {
    let start_iso = (draw.start - TimeDelta::minutes(2))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let end_iso = (draw.end + TimeDelta::minutes(2))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let t1_raw = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start_iso}, stop: {end_iso})
  |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_t1")"#
        ),
    )?;

    let flow_raw = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start_iso}, stop: {end_iso})
  |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_flow")"#
        ),
    )?;

    let t2_raw = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start_iso}, stop: {end_iso})
  |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_t2")"#
        ),
    )?;

    // Also fetch HwcStorageTemp — extend window to +5 min to catch post-draw settling
    let hwc_end_iso = (draw.end + TimeDelta::minutes(5))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let hwc_raw = query_ts(
        url,
        org,
        token,
        &format!(
            r#"from(bucket: "{bucket}")
  |> range(start: {start_iso}, stop: {hwc_end_iso})
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "HwcStorageTemp")"#
        ),
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
            let min_vol = vols.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            return CapacityRecommendation {
                recommended_full_litres: Some(wwhr_estimate.max(min_vol).round()),
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

// ── InfluxDB writes ─────────────────────────────────────────────────────────

fn write_results_to_influx(
    url: &str,
    org: &str,
    token: &str,
    bucket: &str,
    results: &[InflectionResult],
) {
    let to_write: Vec<_> = results
        .iter()
        .filter(|r| r.definitive_cumulative.is_some())
        .collect();

    if to_write.is_empty() {
        eprintln!("No inflection measurements to write.");
        return;
    }

    eprintln!("Writing {} measurements to InfluxDB...", to_write.len());
    for r in &to_write {
        let ts = r.draw.start.timestamp();
        let cat = r.inflection_category();
        let draw_type = r.draw_type();
        let crossover = r
            .draw
            .preceding_charge
            .as_ref()
            .map(|c| c.crossover)
            .unwrap_or(false);
        let line = format!(
            "dhw_inflection,category={cat},crossover={crossover},draw_type={draw_type} \
             cumulative_volume={:.1},draw_volume={:.1},gap_hours={:.2},\
             t1_start={:.2},t1_at_inflection={:.2},mains_temp={:.1},\
             flow_rate={:.0},rate={:.5},hwc_pre={:.1},hwc_min={:.1},\
             hwc_drop={:.1} {ts}",
            r.definitive_cumulative.unwrap_or(0.0),
            r.definitive_draw_vol.unwrap_or(0.0),
            r.draw.gap_hours,
            r.t1_start,
            r.t1_at_definitive.unwrap_or(0.0),
            r.mains_temp,
            r.peak_flow_rate,
            r.definitive_rate.unwrap_or(0.0),
            r.hwc_pre,
            r.hwc_min,
            r.hwc_drop,
        );
        write_influx_line(url, org, token, bucket, &line);
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
        eprintln!("  Recommended capacity: {val}L ({})", rec.method);
    }

    eprintln!("  Done.");
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

    let url = &cfg.influx.url;
    let org = &cfg.influx.org;
    let bucket = &cfg.influx.bucket;

    let (_charges, draws) = find_events(url, org, &token, bucket, days)?;

    let mut results: Vec<InflectionResult> = Vec::new();
    for (i, draw) in draws.iter().enumerate() {
        eprint!(
            "\r  Analysing draw {}/{} ({})...",
            i + 1,
            draws.len(),
            draw.start.format("%d/%m %H:%M")
        );
        if let Some(result) = analyse_draw(url, org, &token, bucket, draw)? {
            results.push(result);
        }
    }
    if !draws.is_empty() {
        eprintln!();
    }

    if !no_write {
        write_results_to_influx(url, org, &token, bucket, &results);
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
            println!("{}", serde_json::to_string_pretty(&json_summary(&results)).unwrap())
        }
    }

    Ok(())
}
