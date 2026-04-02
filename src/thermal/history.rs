use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveTime, Timelike};
use serde::Serialize;

use super::config::{load_thermal_config, resolve_influx_token};
use super::error::{ThermalError, ThermalResult};
use super::influx::{parse_dt, query_flux_csv_pub};

const DHW_FLOW_THRESHOLD_LH: f64 = 900.0;
const DHW_MIN_DURATION_SECONDS: i64 = 300;
const WAKING_START_HOUR: u32 = 7;
const WAKING_END_HOUR: u32 = 23;
const COMFORT_MIN_C: f64 = 20.0;

#[derive(Debug)]
struct HistoryCtx {
    url: String,
    org: String,
    bucket: String,
    token: String,
}

#[derive(Debug, Clone, Serialize)]
struct NumericPoint {
    ts: String,
    value: f64,
}

#[derive(Debug, Clone, Serialize)]
struct NumericSummary {
    samples: usize,
    start: Option<NumericPoint>,
    end: Option<NumericPoint>,
    min: Option<NumericPoint>,
    max: Option<NumericPoint>,
    latest: Option<NumericPoint>,
}

#[derive(Debug, Clone, Serialize)]
struct StringPoint {
    ts: String,
    value: String,
}

#[derive(Debug, Clone, Serialize)]
struct Period {
    start: String,
    end: String,
    duration_minutes: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ControllerEvent {
    ts: String,
    mode: String,
    action: String,
    tariff: String,
    target_flow_c: Option<f64>,
    curve_after: Option<f64>,
    flow_desired_c: Option<f64>,
}

#[derive(Debug, Clone)]
struct ControllerRow {
    ts: DateTime<FixedOffset>,
    mode: String,
    action: String,
    tariff: String,
    target_flow_c: Option<f64>,
    curve_after: Option<f64>,
    flow_desired_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ModeChange {
    ts: String,
    from: Option<String>,
    to: String,
}

#[derive(Debug, Clone, Serialize)]
struct HeatingEvents {
    comfort_miss_periods: Vec<Period>,
    likely_preheat_start: Option<ControllerEvent>,
    dhw_overlap_periods: Vec<Period>,
    likely_sawtooth: bool,
    sawtooth_alternations: usize,
}

#[derive(Debug, Clone, Serialize)]
struct HeatingHistorySummary {
    window: WindowSummary,
    leather_c: Option<NumericSummary>,
    aldora_c: Option<NumericSummary>,
    outside_c: Option<NumericSummary>,
    heat_curve: Option<NumericSummary>,
    target_flow_c: Option<NumericSummary>,
    actual_flow_desired_c: Option<NumericSummary>,
    actual_flow_c: Option<NumericSummary>,
    return_c: Option<NumericSummary>,
    controller_mode_changes: Vec<ModeChange>,
    controller_events: Vec<ControllerEvent>,
    events: HeatingEvents,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DhwChargeSummary {
    start: String,
    end: String,
    duration_minutes: f64,
    t1_start_c: Option<f64>,
    t1_peak_c: Option<f64>,
    t1_end_c: Option<f64>,
    hwc_start_c: Option<f64>,
    hwc_peak_c: Option<f64>,
    hwc_end_c: Option<f64>,
    remaining_litres_start: Option<f64>,
    remaining_litres_end: Option<f64>,
    sfmode_start: Option<String>,
    sfmode_end: Option<String>,
    crossover: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct DhwEvents {
    no_crossover: bool,
    low_t1: bool,
    hwc_sfmode_load_stuck: bool,
    large_t1_hwc_divergence: bool,
    max_t1_hwc_divergence_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct DhwHistorySummary {
    window: WindowSummary,
    charges_detected: Vec<DhwChargeSummary>,
    t1_c: Option<NumericSummary>,
    hwc_storage_c: Option<NumericSummary>,
    remaining_litres: Option<NumericSummary>,
    sfmode: Vec<StringPoint>,
    charging: bool,
    events: DhwEvents,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WindowSummary {
    since: String,
    until: String,
}

pub fn heating_history(
    config_path: &Path,
    since: &str,
    until: &str,
    human: bool,
) -> ThermalResult<()> {
    let (ctx, since_dt, until_dt) = load_ctx_and_window(config_path, since, until)?;

    let leather = query_topic_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "emon/emonth2_23/temperature",
        "value",
        "5m",
        "mean",
    )?;
    let aldora = query_topic_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "zigbee2mqtt/aldora_temp_humid",
        "temperature",
        "5m",
        "mean",
    )?;
    let outside = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "OutsideTemp",
        "5m",
        "mean",
    )?;
    let heat_curve = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "Hc1HeatCurve",
        "1m",
        "last",
    )?;
    let actual_flow_desired = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "Hc1FlowTempDesired",
        "1m",
        "last",
    )?;
    let actual_flow = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "FlowTemp",
        "1m",
        "mean",
    )?;
    let return_c = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "ReturnTemp",
        "1m",
        "mean",
    )?;
    let building_circuit_flow = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "BuildingCircuitFlow",
        "1m",
        "last",
    )?;
    let controller_rows = query_controller_rows(&ctx, &since_dt, &until_dt)?;

    let comfort_miss_periods = periods_from_numeric_predicate(&leather, |v| v < COMFORT_MIN_C)
        .into_iter()
        .filter(|p| period_intersects_waking_hours(p, &since_dt, &until_dt))
        .collect::<Vec<_>>();
    let dhw_overlap_periods =
        periods_from_numeric_predicate(&building_circuit_flow, |v| v >= DHW_FLOW_THRESHOLD_LH)
            .into_iter()
            .filter(|p| period_duration_seconds(p) >= DHW_MIN_DURATION_SECONDS)
            .collect::<Vec<_>>();

    let likely_preheat_start = controller_rows
        .iter()
        .find(|row| {
            matches!(
                row.action.as_str(),
                "overnight_preheat" | "preheat_model" | "overnight_maintain"
            )
        })
        .map(controller_event_from_row);

    let (likely_sawtooth, sawtooth_alternations) = detect_sawtooth(&controller_rows);

    let mut warnings = Vec::new();
    add_missing_numeric_warning(&mut warnings, "Leather room temperature", &leather);
    add_missing_numeric_warning(&mut warnings, "Aldora room temperature", &aldora);
    add_missing_numeric_warning(&mut warnings, "Outside temperature", &outside);
    add_missing_numeric_warning(&mut warnings, "heat curve", &heat_curve);
    add_missing_numeric_warning(
        &mut warnings,
        "target flow",
        &controller_rows_target_series(&controller_rows),
    );
    add_missing_numeric_warning(&mut warnings, "actual desired flow", &actual_flow_desired);
    add_missing_numeric_warning(&mut warnings, "actual flow", &actual_flow);
    add_missing_numeric_warning(&mut warnings, "return flow", &return_c);
    if controller_rows.is_empty() {
        warnings.push("adaptive_heating_mvp controller rows unavailable in InfluxDB".to_string());
    }
    if dhw_overlap_periods.is_empty() {
        warnings.push("no DHW overlap periods detected in this window".to_string());
    }

    let summary = HeatingHistorySummary {
        window: WindowSummary {
            since: since_dt.to_rfc3339(),
            until: until_dt.to_rfc3339(),
        },
        leather_c: summarize_numeric(&leather),
        aldora_c: summarize_numeric(&aldora),
        outside_c: summarize_numeric(&outside),
        heat_curve: summarize_numeric(&heat_curve),
        target_flow_c: summarize_numeric(&controller_rows_target_series(&controller_rows)),
        actual_flow_desired_c: summarize_numeric(&actual_flow_desired),
        actual_flow_c: summarize_numeric(&actual_flow),
        return_c: summarize_numeric(&return_c),
        controller_mode_changes: controller_mode_changes(&controller_rows),
        controller_events: controller_rows
            .iter()
            .map(controller_event_from_row)
            .collect(),
        events: HeatingEvents {
            comfort_miss_periods,
            likely_preheat_start,
            dhw_overlap_periods,
            likely_sawtooth,
            sawtooth_alternations,
        },
        warnings,
    };

    if human {
        print_heating_history_human(&summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary)
                .map_err(|e| ThermalError::ArtifactSerialize(e))?
        );
    }

    Ok(())
}

pub fn dhw_history(config_path: &Path, since: &str, until: &str, human: bool) -> ThermalResult<()> {
    let (ctx, since_dt, until_dt) = load_ctx_and_window(config_path, since, until)?;

    let t1 = query_measurement_numeric_series(
        &ctx, &since_dt, &until_dt, "emon", "dhw_t1", "30s", "last",
    )?;
    let hwc = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "HwcStorageTemp",
        "30s",
        "last",
    )?;
    let remaining = query_plain_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "dhw",
        "remaining_litres",
        "1m",
        "last",
    )?;
    let sfmode = query_measurement_string_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "HwcSFMode",
        "1m",
    )?;
    let building_circuit_flow = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "BuildingCircuitFlow",
        "1m",
        "last",
    )?;

    let charge_periods =
        periods_from_numeric_predicate(&building_circuit_flow, |v| v >= DHW_FLOW_THRESHOLD_LH)
            .into_iter()
            .filter(|p| period_duration_seconds(p) >= DHW_MIN_DURATION_SECONDS)
            .collect::<Vec<_>>();

    let charges_detected = charge_periods
        .iter()
        .map(|period| summarize_charge(period, &t1, &hwc, &remaining, &sfmode))
        .collect::<Vec<_>>();

    let max_divergence = max_series_divergence(&t1, &hwc);
    let hwc_sfmode_load_stuck = sfmode.last().map(|(_, v)| v == "load").unwrap_or(false)
        && !charge_periods
            .iter()
            .any(|p| period_contains_recent_end(p, &until_dt, 600));

    let mut warnings = Vec::new();
    add_missing_numeric_warning(&mut warnings, "DHW T1", &t1);
    add_missing_numeric_warning(&mut warnings, "HwcStorageTemp", &hwc);
    add_missing_numeric_warning(&mut warnings, "remaining litres", &remaining);
    if sfmode.is_empty() {
        warnings.push("HwcSFMode unavailable in this window".to_string());
    }
    if charges_detected.is_empty() {
        warnings.push("no DHW charge periods detected in this window".to_string());
    }
    if hwc_sfmode_load_stuck {
        warnings.push("HwcSFMode appears stuck on load".to_string());
    }
    if max_divergence.unwrap_or(0.0) >= 8.0 {
        warnings.push(format!(
            "large T1/HwcStorageTemp divergence detected ({:.1}°C)",
            max_divergence.unwrap_or(0.0)
        ));
    }

    let summary = DhwHistorySummary {
        window: WindowSummary {
            since: since_dt.to_rfc3339(),
            until: until_dt.to_rfc3339(),
        },
        charges_detected: charges_detected.clone(),
        t1_c: summarize_numeric(&t1),
        hwc_storage_c: summarize_numeric(&hwc),
        remaining_litres: summarize_numeric(&remaining),
        sfmode: sfmode
            .iter()
            .map(|(ts, value)| StringPoint {
                ts: ts.to_rfc3339(),
                value: value.clone(),
            })
            .collect(),
        charging: !charge_periods.is_empty(),
        events: DhwEvents {
            no_crossover: !charges_detected.is_empty()
                && charges_detected.iter().all(|c| c.crossover != Some(true)),
            low_t1: t1.iter().any(|(_, v)| *v < 42.0),
            hwc_sfmode_load_stuck,
            large_t1_hwc_divergence: max_divergence.unwrap_or(0.0) >= 8.0,
            max_t1_hwc_divergence_c: max_divergence,
        },
        warnings,
    };

    if human {
        print_dhw_history_human(&summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary)
                .map_err(|e| ThermalError::ArtifactSerialize(e))?
        );
    }

    Ok(())
}

fn load_ctx_and_window(
    config_path: &Path,
    since: &str,
    until: &str,
) -> ThermalResult<(HistoryCtx, DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    let (_, cfg) = load_thermal_config(config_path)?;
    let token = resolve_influx_token(&cfg)?;
    let since_dt = parse_dt(since)?;
    let until_dt = parse_dt(until)?;
    Ok((
        HistoryCtx {
            url: cfg.influx.url,
            org: cfg.influx.org,
            bucket: cfg.influx.bucket,
            token,
        },
        since_dt,
        until_dt,
    ))
}

fn query_numeric_series(
    ctx: &HistoryCtx,
    flux: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, flux)?;
    let mut out = Vec::new();
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        let Some(value_str) = row.get("_value") else {
            continue;
        };
        let Ok(value) = value_str.parse::<f64>() else {
            continue;
        };
        out.push((parse_dt(ts_str)?, value));
    }
    out.sort_by_key(|(ts, _)| *ts);
    Ok(out)
}

fn query_string_series(
    ctx: &HistoryCtx,
    flux: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String)>> {
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, flux)?;
    let mut out = Vec::new();
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        let Some(value_str) = row.get("_value") else {
            continue;
        };
        out.push((parse_dt(ts_str)?, value_str.clone()));
    }
    out.sort_by_key(|(ts, _)| *ts);
    out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    Ok(out)
}

fn query_topic_numeric_series(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    topic: &str,
    field: &str,
    every: &str,
    agg: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\")\n  |> aggregateWindow(every: {}, fn: {}, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        topic,
        field,
        every,
        agg,
    );
    query_numeric_series(ctx, &flux)
}

fn query_measurement_numeric_series(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
    every: &str,
    agg: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\")\n  |> aggregateWindow(every: {}, fn: {}, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
        every,
        agg,
    );
    query_numeric_series(ctx, &flux)
}

fn query_plain_measurement_numeric_series(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
    every: &str,
    agg: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, f64)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"{}\" and r._field == \"{}\")\n  |> aggregateWindow(every: {}, fn: {}, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
        every,
        agg,
    );
    query_numeric_series(ctx, &flux)
}

fn query_measurement_string_series(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
    every: &str,
) -> ThermalResult<Vec<(DateTime<FixedOffset>, String)>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\")\n  |> aggregateWindow(every: {}, fn: last, createEmpty: false)\n  |> keep(columns: [\"_time\", \"_value\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
        every,
    );
    query_string_series(ctx, &flux)
}

fn query_controller_rows(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<ControllerRow>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"adaptive_heating_mvp\")\n  |> filter(fn: (r) => r._field == \"target_flow_c\" or r._field == \"curve_after\" or r._field == \"flow_desired_c\")\n  |> pivot(rowKey: [\"_time\", \"mode\", \"action\", \"tariff\"], columnKey: [\"_field\"], valueColumn: \"_value\")\n  |> keep(columns: [\"_time\", \"mode\", \"action\", \"tariff\", \"target_flow_c\", \"curve_after\", \"flow_desired_c\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
    );
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        let ts = parse_dt(ts_str)?;
        out.push(ControllerRow {
            ts,
            mode: row
                .get("mode")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            action: row
                .get("action")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            tariff: row
                .get("tariff")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            target_flow_c: row.get("target_flow_c").and_then(|v| v.parse().ok()),
            curve_after: row.get("curve_after").and_then(|v| v.parse().ok()),
            flow_desired_c: row.get("flow_desired_c").and_then(|v| v.parse().ok()),
        });
    }
    out.sort_by_key(|row| row.ts);
    Ok(out)
}

fn summarize_numeric(series: &[(DateTime<FixedOffset>, f64)]) -> Option<NumericSummary> {
    if series.is_empty() {
        return None;
    }
    let start = series.first().map(point_from_pair);
    let end = series.last().map(point_from_pair);
    let latest = end.clone();
    let min = series
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(point_from_pair);
    let max = series
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(point_from_pair);
    Some(NumericSummary {
        samples: series.len(),
        start,
        end,
        min,
        max,
        latest,
    })
}

fn point_from_pair(pair: &(DateTime<FixedOffset>, f64)) -> NumericPoint {
    NumericPoint {
        ts: pair.0.to_rfc3339(),
        value: pair.1,
    }
}

fn periods_from_numeric_predicate<F>(
    series: &[(DateTime<FixedOffset>, f64)],
    predicate: F,
) -> Vec<Period>
where
    F: Fn(f64) -> bool,
{
    if series.is_empty() {
        return Vec::new();
    }

    let step_seconds = typical_step_seconds(series);
    let mut out = Vec::new();
    let mut current_start: Option<DateTime<FixedOffset>> = None;
    let mut current_end: Option<DateTime<FixedOffset>> = None;

    for (ts, value) in series {
        if predicate(*value) {
            current_start.get_or_insert(*ts);
            current_end = Some(*ts + chrono::TimeDelta::seconds(step_seconds));
        } else if let (Some(start), Some(end)) = (current_start.take(), current_end.take()) {
            out.push(period_from_times(start, end));
        }
    }

    if let (Some(start), Some(end)) = (current_start, current_end) {
        out.push(period_from_times(start, end));
    }

    out
}

fn typical_step_seconds(series: &[(DateTime<FixedOffset>, f64)]) -> i64 {
    let mut steps = series
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).num_seconds())
        .filter(|v| *v > 0)
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return 60;
    }
    steps.sort_unstable();
    steps[steps.len() / 2]
}

fn period_from_times(start: DateTime<FixedOffset>, end: DateTime<FixedOffset>) -> Period {
    Period {
        start: start.to_rfc3339(),
        end: end.to_rfc3339(),
        duration_minutes: (end - start).num_seconds() as f64 / 60.0,
    }
}

fn period_duration_seconds(period: &Period) -> i64 {
    let Ok(start) = parse_dt(&period.start) else {
        return 0;
    };
    let Ok(end) = parse_dt(&period.end) else {
        return 0;
    };
    (end - start).num_seconds()
}

fn period_intersects_waking_hours(
    period: &Period,
    _since: &DateTime<FixedOffset>,
    _until: &DateTime<FixedOffset>,
) -> bool {
    let Ok(start) = parse_dt(&period.start) else {
        return false;
    };
    let Ok(end) = parse_dt(&period.end) else {
        return false;
    };
    let mut current = start;
    while current < end {
        if is_waking_time(current.time()) {
            return true;
        }
        current += chrono::TimeDelta::minutes(5);
    }
    false
}

fn is_waking_time(time: NaiveTime) -> bool {
    let minutes = time.hour() * 60 + time.minute();
    let start = WAKING_START_HOUR * 60;
    let end = WAKING_END_HOUR * 60;
    minutes >= start && minutes < end
}

fn controller_event_from_row(row: &ControllerRow) -> ControllerEvent {
    ControllerEvent {
        ts: row.ts.to_rfc3339(),
        mode: row.mode.clone(),
        action: row.action.clone(),
        tariff: row.tariff.clone(),
        target_flow_c: row.target_flow_c,
        curve_after: row.curve_after,
        flow_desired_c: row.flow_desired_c,
    }
}

fn controller_mode_changes(rows: &[ControllerRow]) -> Vec<ModeChange> {
    let mut out = Vec::new();
    let mut last_mode: Option<String> = None;
    for row in rows {
        if last_mode.as_deref() != Some(row.mode.as_str()) {
            out.push(ModeChange {
                ts: row.ts.to_rfc3339(),
                from: last_mode.clone(),
                to: row.mode.clone(),
            });
            last_mode = Some(row.mode.clone());
        }
    }
    out
}

fn controller_rows_target_series(rows: &[ControllerRow]) -> Vec<(DateTime<FixedOffset>, f64)> {
    rows.iter()
        .filter_map(|row| row.target_flow_c.map(|v| (row.ts, v)))
        .collect()
}

fn detect_sawtooth(rows: &[ControllerRow]) -> (bool, usize) {
    let significant = rows
        .iter()
        .filter_map(|row| row.curve_after.map(|v| (row.ts, v)))
        .collect::<Vec<_>>();
    if significant.len() < 4 {
        return (false, 0);
    }

    let mut deltas = Vec::new();
    for window in significant.windows(2) {
        let delta = window[1].1 - window[0].1;
        if delta.abs() >= 0.05 {
            deltas.push(delta.signum() as i32);
        }
    }
    if deltas.len() < 3 {
        return (false, 0);
    }

    let mut alternations = 0;
    for window in deltas.windows(2) {
        if window[0] != window[1] {
            alternations += 1;
        }
    }
    (alternations >= 3, alternations)
}

fn add_missing_numeric_warning(
    warnings: &mut Vec<String>,
    label: &str,
    series: &[(DateTime<FixedOffset>, f64)],
) {
    if series.is_empty() {
        warnings.push(format!("{label} unavailable in this window"));
    }
}

fn value_at_or_before(
    series: &[(DateTime<FixedOffset>, f64)],
    ts: DateTime<FixedOffset>,
) -> Option<f64> {
    series
        .iter()
        .take_while(|(t, _)| *t <= ts)
        .last()
        .map(|(_, v)| *v)
}

fn string_at_or_before(
    series: &[(DateTime<FixedOffset>, String)],
    ts: DateTime<FixedOffset>,
) -> Option<String> {
    series
        .iter()
        .take_while(|(t, _)| *t <= ts)
        .last()
        .map(|(_, v)| v.clone())
}

fn max_in_period(
    series: &[(DateTime<FixedOffset>, f64)],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Option<f64> {
    series
        .iter()
        .filter(|(ts, _)| *ts >= start && *ts <= end)
        .map(|(_, v)| *v)
        .max_by(|a, b| a.total_cmp(b))
}

fn summarize_charge(
    period: &Period,
    t1: &[(DateTime<FixedOffset>, f64)],
    hwc: &[(DateTime<FixedOffset>, f64)],
    remaining: &[(DateTime<FixedOffset>, f64)],
    sfmode: &[(DateTime<FixedOffset>, String)],
) -> DhwChargeSummary {
    let start = parse_dt(&period.start).expect("valid charge period start");
    let end = parse_dt(&period.end).expect("valid charge period end");
    let t1_start = value_at_or_before(t1, start);
    let t1_end = value_at_or_before(t1, end);
    let hwc_start = value_at_or_before(hwc, start);
    let hwc_end = value_at_or_before(hwc, end);
    DhwChargeSummary {
        start: period.start.clone(),
        end: period.end.clone(),
        duration_minutes: period.duration_minutes,
        t1_start_c: t1_start,
        t1_peak_c: max_in_period(t1, start, end),
        t1_end_c: t1_end,
        hwc_start_c: hwc_start,
        hwc_peak_c: max_in_period(hwc, start, end),
        hwc_end_c: hwc_end,
        remaining_litres_start: value_at_or_before(remaining, start),
        remaining_litres_end: value_at_or_before(remaining, end),
        sfmode_start: string_at_or_before(sfmode, start),
        sfmode_end: string_at_or_before(sfmode, end),
        crossover: match (t1_start, hwc_end) {
            (Some(t1_pre), Some(hwc_final)) => Some(hwc_final >= t1_pre),
            _ => None,
        },
    }
}

fn max_series_divergence(
    a: &[(DateTime<FixedOffset>, f64)],
    b: &[(DateTime<FixedOffset>, f64)],
) -> Option<f64> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let mut times = BTreeSet::new();
    for (ts, _) in a {
        times.insert(*ts);
    }
    for (ts, _) in b {
        times.insert(*ts);
    }
    let mut max_divergence: Option<f64> = None;
    for ts in times {
        let Some(av) = value_at_or_before(a, ts) else {
            continue;
        };
        let Some(bv) = value_at_or_before(b, ts) else {
            continue;
        };
        let diff = (av - bv).abs();
        max_divergence = Some(max_divergence.map(|m| m.max(diff)).unwrap_or(diff));
    }
    max_divergence
}

fn period_contains_recent_end(
    period: &Period,
    until: &DateTime<FixedOffset>,
    seconds: i64,
) -> bool {
    let Ok(end) = parse_dt(&period.end) else {
        return false;
    };
    end <= *until && end >= *until - chrono::TimeDelta::seconds(seconds)
}

fn print_heating_history_human(summary: &HeatingHistorySummary) {
    println!("Heating history");
    println!("---------------");
    println!(
        "window: {} → {}",
        summary.window.since, summary.window.until
    );
    print_numeric_summary_line("Leather", &summary.leather_c);
    print_numeric_summary_line("Aldora", &summary.aldora_c);
    print_numeric_summary_line("Outside", &summary.outside_c);
    print_numeric_summary_line("Heat curve", &summary.heat_curve);
    print_numeric_summary_line("Target flow", &summary.target_flow_c);
    print_numeric_summary_line("Actual desired flow", &summary.actual_flow_desired_c);
    print_numeric_summary_line("Actual flow", &summary.actual_flow_c);
    print_numeric_summary_line("Return", &summary.return_c);
    println!(
        "controller_mode_changes: {}",
        summary.controller_mode_changes.len()
    );
    println!(
        "dhw_overlap_periods: {}",
        summary.events.dhw_overlap_periods.len()
    );
    println!(
        "comfort_miss_periods: {}",
        summary.events.comfort_miss_periods.len()
    );
    if let Some(preheat) = &summary.events.likely_preheat_start {
        println!(
            "likely_preheat_start: {} {} target_flow={}",
            preheat.ts,
            preheat.action,
            preheat
                .target_flow_c
                .map(|v| format!("{:.1}°C", v))
                .unwrap_or_else(|| "n/a".to_string())
        );
    } else {
        println!("likely_preheat_start: none");
    }
    println!(
        "likely_sawtooth: {} (alternations={})",
        summary.events.likely_sawtooth, summary.events.sawtooth_alternations
    );
    if summary.warnings.is_empty() {
        println!("warnings: none");
    } else {
        println!("warnings:");
        for warning in &summary.warnings {
            println!("- {warning}");
        }
    }
}

fn print_dhw_history_human(summary: &DhwHistorySummary) {
    println!("DHW history");
    println!("-----------");
    println!(
        "window: {} → {}",
        summary.window.since, summary.window.until
    );
    print_numeric_summary_line("T1", &summary.t1_c);
    print_numeric_summary_line("HwcStorageTemp", &summary.hwc_storage_c);
    print_numeric_summary_line("Remaining litres", &summary.remaining_litres);
    println!("charges_detected: {}", summary.charges_detected.len());
    for (idx, charge) in summary.charges_detected.iter().enumerate() {
        println!(
            "charge[{idx}]: {} → {} ({:.1} min) crossover={}",
            charge.start,
            charge.end,
            charge.duration_minutes,
            charge
                .crossover
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
    }
    println!("charging: {}", summary.charging);
    println!(
        "events: no_crossover={} low_t1={} sfmode_load_stuck={} large_divergence={} max_divergence={}",
        summary.events.no_crossover,
        summary.events.low_t1,
        summary.events.hwc_sfmode_load_stuck,
        summary.events.large_t1_hwc_divergence,
        summary
            .events
            .max_t1_hwc_divergence_c
            .map(|v| format!("{:.1}°C", v))
            .unwrap_or_else(|| "n/a".to_string())
    );
    if summary.warnings.is_empty() {
        println!("warnings: none");
    } else {
        println!("warnings:");
        for warning in &summary.warnings {
            println!("- {warning}");
        }
    }
}

fn print_numeric_summary_line(label: &str, summary: &Option<NumericSummary>) {
    match summary {
        Some(s) => {
            let min = s.min.as_ref().map(|p| p.value).unwrap_or(f64::NAN);
            let max = s.max.as_ref().map(|p| p.value).unwrap_or(f64::NAN);
            let latest = s.latest.as_ref().map(|p| p.value).unwrap_or(f64::NAN);
            println!(
                "{label}: min={min:.1} max={max:.1} latest={latest:.1} samples={}",
                s.samples
            );
        }
        None => println!("{label}: unavailable"),
    }
}
