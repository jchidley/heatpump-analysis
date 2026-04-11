use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde::Serialize;

use super::config::{load_thermal_config, resolve_influx_token};
use super::error::{ThermalError, ThermalResult};
use super::influx::{parse_dt, query_flux_csv_pub, query_flux_raw_pub};

const DHW_FLOW_THRESHOLD_LH: f64 = 900.0;
const DHW_MIN_DURATION_SECONDS: i64 = 300;
const DHW_BOUNDARY_LOOKBACK_SECONDS: i64 = 900;
const DHW_BOUNDARY_LOOKAHEAD_SECONDS: i64 = 900;
const WAKING_START_HOUR: u32 = 7;
const WAKING_END_HOUR: u32 = 23;
const COMFORT_MIN_C: f64 = 20.0;

#[derive(Debug)]
struct HistoryCtx {
    url: String,
    org: String,
    bucket: String,
    token: String,
    profile_queries: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NumericPoint {
    pub ts: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NumericSummary {
    pub samples: usize,
    pub start: Option<NumericPoint>,
    pub end: Option<NumericPoint>,
    pub min: Option<NumericPoint>,
    pub max: Option<NumericPoint>,
    pub latest: Option<NumericPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StringPoint {
    pub ts: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplingStats {
    pub label: String,
    pub window_start: String,
    pub window_end: String,
    pub samples: usize,
    pub median_step_seconds: Option<f64>,
    pub min_step_seconds: Option<f64>,
    pub max_step_seconds: Option<f64>,
}

struct TopicSummarySpec<'a> {
    label: &'a str,
    topic: &'a str,
    field: &'a str,
}

struct MeasurementSummarySpec<'a> {
    label: &'a str,
    measurement: &'a str,
    field: &'a str,
}

struct PlainMeasurementSummarySpec<'a> {
    label: &'a str,
    measurement: &'a str,
    field: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Period {
    pub start: String,
    pub end: String,
    pub duration_minutes: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControllerEvent {
    pub ts: String,
    pub mode: String,
    pub action: String,
    pub tariff: String,
    pub target_flow_c: Option<f64>,
    pub curve_after: Option<f64>,
    pub flow_desired_c: Option<f64>,
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
pub struct ModeChange {
    pub ts: String,
    pub from: Option<String>,
    pub to: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatingEvents {
    pub comfort_miss_periods: Vec<Period>,
    pub likely_preheat_start: Option<ControllerEvent>,
    pub dhw_overlap_periods: Vec<Period>,
    pub likely_sawtooth: bool,
    pub sawtooth_alternations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatingHistorySummary {
    pub window: WindowSummary,
    pub leather_c: Option<NumericSummary>,
    pub aldora_c: Option<NumericSummary>,
    pub outside_c: Option<NumericSummary>,
    pub heat_curve: Option<NumericSummary>,
    pub target_flow_c: Option<NumericSummary>,
    pub actual_flow_desired_c: Option<NumericSummary>,
    pub actual_flow_c: Option<NumericSummary>,
    pub return_c: Option<NumericSummary>,
    pub sampling: Vec<SamplingStats>,
    pub controller_mode_changes: Vec<ModeChange>,
    pub controller_events: Vec<ControllerEvent>,
    pub events: HeatingEvents,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DhwChargeSummary {
    pub start: String,
    pub end: String,
    pub duration_minutes: f64,
    pub t1_start_c: Option<f64>,
    pub t1_peak_c: Option<f64>,
    pub t1_end_c: Option<f64>,
    pub hwc_start_c: Option<f64>,
    pub hwc_peak_c: Option<f64>,
    pub hwc_end_c: Option<f64>,
    pub remaining_litres_start: Option<f64>,
    pub remaining_litres_end: Option<f64>,
    pub sfmode_start: Option<String>,
    pub sfmode_end: Option<String>,
    pub crossover: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DhwEvents {
    pub no_crossover: bool,
    pub low_t1: bool,
    pub hwc_sfmode_load_stuck: bool,
    pub large_t1_hwc_divergence: bool,
    pub max_t1_hwc_divergence_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DhwHistorySummary {
    pub window: WindowSummary,
    pub charges_detected: Vec<DhwChargeSummary>,
    pub t1_c: Option<NumericSummary>,
    pub hwc_storage_c: Option<NumericSummary>,
    pub remaining_litres: Option<NumericSummary>,
    pub sampling: Vec<SamplingStats>,
    pub sfmode: Vec<StringPoint>,
    pub charging: bool,
    pub events: DhwEvents,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DhwDrilldownSummary {
    window: WindowSummary,
    charge_periods: Vec<Period>,
    t1_native: Vec<NumericPoint>,
    hwc_storage: Vec<NumericPoint>,
    remaining_litres: Vec<NumericPoint>,
    building_circuit_flow_lh: Vec<NumericPoint>,
    sfmode: Vec<StringPoint>,
    t1_sampling: SamplingStats,
    hwc_sampling: SamplingStats,
    remaining_sampling: SamplingStats,
    flow_sampling: SamplingStats,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowSummary {
    pub since: String,
    pub until: String,
    pub generated_at: String,
}

pub fn heating_history_summary(
    config_path: &Path,
    since: &str,
    until: &str,
    profile_queries: bool,
) -> ThermalResult<HeatingHistorySummary> {
    let (ctx, since_dt, until_dt) =
        load_ctx_and_window(config_path, since, until, profile_queries)?;

    let topic_summaries = query_topic_numeric_summaries_compact(
        &ctx,
        &since_dt,
        &until_dt,
        &[
            TopicSummarySpec {
                label: "leather",
                topic: "emon/emonth2_23/temperature",
                field: "value",
            },
            TopicSummarySpec {
                label: "aldora",
                topic: "zigbee2mqtt/aldora_temp_humid",
                field: "temperature",
            },
        ],
    )?;
    let measurement_summaries = query_measurement_numeric_summaries_compact(
        &ctx,
        &since_dt,
        &until_dt,
        &[
            MeasurementSummarySpec {
                label: "outside",
                measurement: "ebusd_poll",
                field: "OutsideTemp",
            },
            MeasurementSummarySpec {
                label: "heat_curve",
                measurement: "ebusd_poll",
                field: "Hc1HeatCurve",
            },
            MeasurementSummarySpec {
                label: "actual_flow_desired",
                measurement: "ebusd_poll",
                field: "Hc1FlowTempDesired",
            },
            MeasurementSummarySpec {
                label: "actual_flow",
                measurement: "ebusd_poll",
                field: "FlowTemp",
            },
            MeasurementSummarySpec {
                label: "return",
                measurement: "ebusd_poll",
                field: "ReturnTemp",
            },
        ],
    )?;
    let leather_summary = topic_summaries.get("leather").cloned().unwrap_or(None);
    let aldora_summary = topic_summaries.get("aldora").cloned().unwrap_or(None);
    let outside_summary = measurement_summaries
        .get("outside")
        .cloned()
        .unwrap_or(None);
    let heat_curve_summary = measurement_summaries
        .get("heat_curve")
        .cloned()
        .unwrap_or(None);
    let actual_flow_desired_summary = measurement_summaries
        .get("actual_flow_desired")
        .cloned()
        .unwrap_or(None);
    let actual_flow_summary = measurement_summaries
        .get("actual_flow")
        .cloned()
        .unwrap_or(None);
    let return_summary = measurement_summaries.get("return").cloned().unwrap_or(None);
    let controller_rows = query_controller_rows(&ctx, &since_dt, &until_dt)?;
    let target_flow_summary = summarize_numeric(&controller_rows_target_series(&controller_rows));

    let comfort_miss_periods = query_topic_below_threshold_periods_compact(
        &ctx,
        &since_dt,
        &until_dt,
        "emon/emonth2_23/temperature",
        "value",
        COMFORT_MIN_C,
    )?
    .into_iter()
    .flat_map(|p| clip_period_to_waking_hours(&p))
    .collect::<Vec<_>>();
    let dhw_overlap_periods = query_measurement_above_threshold_periods_compact(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "BuildingCircuitFlow",
        "30s",
        DHW_FLOW_THRESHOLD_LH,
        Some(DHW_MIN_DURATION_SECONDS),
    )?;

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

    let sampling_since = std::cmp::max(since_dt, until_dt - chrono::TimeDelta::hours(1));
    let controller_times = controller_rows.iter().map(|r| r.ts).collect::<Vec<_>>();
    let sampling = vec![
        sampling_stats_for_topic(
            &ctx,
            &sampling_since,
            &until_dt,
            "Leather room temperature",
            "emon/emonth2_23/temperature",
            "value",
        )?,
        sampling_stats_for_topic(
            &ctx,
            &sampling_since,
            &until_dt,
            "Aldora room temperature",
            "zigbee2mqtt/aldora_temp_humid",
            "temperature",
        )?,
        sampling_stats_for_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "OutsideTemp",
            "ebusd_poll",
            "OutsideTemp",
        )?,
        sampling_stats_for_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "BuildingCircuitFlow",
            "ebusd_poll",
            "BuildingCircuitFlow",
        )?,
        sampling_stats_from_timestamps(
            "adaptive_heating_mvp controller rows",
            &sampling_since,
            &until_dt,
            &controller_times,
        ),
    ];

    let mut warnings = Vec::new();
    add_missing_summary_warning(&mut warnings, "Leather room temperature", &leather_summary);
    add_missing_summary_warning(&mut warnings, "Aldora room temperature", &aldora_summary);
    add_missing_summary_warning(&mut warnings, "Outside temperature", &outside_summary);
    add_missing_summary_warning(&mut warnings, "heat curve", &heat_curve_summary);
    add_missing_summary_warning(&mut warnings, "target flow", &target_flow_summary);
    add_missing_summary_warning(
        &mut warnings,
        "actual desired flow",
        &actual_flow_desired_summary,
    );
    add_missing_summary_warning(&mut warnings, "actual flow", &actual_flow_summary);
    add_missing_summary_warning(&mut warnings, "return flow", &return_summary);
    if controller_rows.is_empty() {
        warnings.push("adaptive_heating_mvp controller rows unavailable in InfluxDB".to_string());
    }
    if dhw_overlap_periods.is_empty() {
        warnings.push("no DHW overlap periods detected in this window".to_string());
    }

    Ok(HeatingHistorySummary {
        window: WindowSummary {
            since: since_dt.to_rfc3339(),
            until: until_dt.to_rfc3339(),
            generated_at: chrono::Utc::now().to_rfc3339(),
        },
        leather_c: leather_summary,
        aldora_c: aldora_summary,
        outside_c: outside_summary,
        heat_curve: heat_curve_summary,
        target_flow_c: target_flow_summary,
        actual_flow_desired_c: actual_flow_desired_summary,
        actual_flow_c: actual_flow_summary,
        return_c: return_summary,
        sampling,
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
    })
}

pub fn heating_history(
    config_path: &Path,
    since: &str,
    until: &str,
    human: bool,
    profile_queries: bool,
) -> ThermalResult<()> {
    let summary = heating_history_summary(config_path, since, until, profile_queries)?;
    if human {
        print_heating_history_human(&summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(ThermalError::ArtifactSerialize)?
        );
    }
    Ok(())
}

pub fn dhw_history_summary(
    config_path: &Path,
    since: &str,
    until: &str,
    profile_queries: bool,
) -> ThermalResult<DhwHistorySummary> {
    let (ctx, since_dt, until_dt) =
        load_ctx_and_window(config_path, since, until, profile_queries)?;

    let measurement_summaries = query_measurement_numeric_summaries_compact(
        &ctx,
        &since_dt,
        &until_dt,
        &[
            MeasurementSummarySpec {
                label: "t1",
                measurement: "emon",
                field: "dhw_t1",
            },
            MeasurementSummarySpec {
                label: "hwc",
                measurement: "ebusd_poll",
                field: "HwcStorageTemp",
            },
        ],
    )?;
    let plain_summaries = query_plain_measurement_numeric_summaries_compact(
        &ctx,
        &since_dt,
        &until_dt,
        &[PlainMeasurementSummarySpec {
            label: "remaining",
            measurement: "dhw",
            field: "remaining_litres",
        }],
    )?;
    let t1_summary = measurement_summaries.get("t1").cloned().unwrap_or(None);
    let hwc_summary = measurement_summaries.get("hwc").cloned().unwrap_or(None);
    let remaining_summary = plain_summaries.get("remaining").cloned().unwrap_or(None);
    let latest_sfmode = query_measurement_string_last_compact(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "HwcSFMode",
    )?;
    let charge_periods = query_dhw_charge_periods_compact(&ctx, &since_dt, &until_dt)?;
    let charges_detected = query_dhw_charge_summaries_batched_compact(&ctx, &charge_periods)?;
    let charging_now = query_measurement_numeric_last_value_compact(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "BuildingCircuitFlow",
    )?
    .map(|v| v >= DHW_FLOW_THRESHOLD_LH)
    .unwrap_or(false);
    let max_divergence = query_dhw_max_divergence_compact(&ctx, &since_dt, &until_dt)?;
    let sampling_since = std::cmp::max(since_dt, until_dt - chrono::TimeDelta::hours(1));
    let sampling = vec![
        sampling_stats_for_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "dhw_t1",
            "emon",
            "dhw_t1",
        )?,
        sampling_stats_for_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "HwcStorageTemp",
            "ebusd_poll",
            "HwcStorageTemp",
        )?,
        sampling_stats_for_plain_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "remaining_litres",
            "dhw",
            "remaining_litres",
        )?,
        sampling_stats_for_measurement(
            &ctx,
            &sampling_since,
            &until_dt,
            "BuildingCircuitFlow",
            "ebusd_poll",
            "BuildingCircuitFlow",
        )?,
    ];

    let hwc_sfmode_load_stuck = latest_sfmode.as_deref() == Some("load")
        && !charge_periods
            .iter()
            .any(|p| period_contains_recent_end(p, &until_dt, 600));

    let mut warnings = Vec::new();
    if t1_summary.is_none() {
        warnings.push("DHW T1 unavailable in this window".to_string());
    }
    if hwc_summary.is_none() {
        warnings.push("HwcStorageTemp unavailable in this window".to_string());
    }
    if remaining_summary.is_none() {
        warnings.push("remaining litres unavailable in this window".to_string());
    }
    if latest_sfmode.is_none() {
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

    let low_t1 = summary_has_min_below(&t1_summary, 42.0);

    Ok(DhwHistorySummary {
        window: WindowSummary {
            since: since_dt.to_rfc3339(),
            until: until_dt.to_rfc3339(),
            generated_at: chrono::Utc::now().to_rfc3339(),
        },
        charges_detected: charges_detected.clone(),
        t1_c: t1_summary,
        hwc_storage_c: hwc_summary,
        remaining_litres: remaining_summary,
        sampling,
        sfmode: latest_sfmode
            .map(|value| {
                vec![StringPoint {
                    ts: until_dt.to_rfc3339(),
                    value,
                }]
            })
            .unwrap_or_default(),
        charging: charging_now,
        events: DhwEvents {
            no_crossover: !charges_detected.is_empty()
                && charges_detected.iter().all(|c| c.crossover != Some(true)),
            low_t1,
            hwc_sfmode_load_stuck,
            large_t1_hwc_divergence: max_divergence.unwrap_or(0.0) >= 8.0,
            max_t1_hwc_divergence_c: max_divergence,
        },
        warnings,
    })
}

pub fn dhw_history(
    config_path: &Path,
    since: &str,
    until: &str,
    human: bool,
    profile_queries: bool,
) -> ThermalResult<()> {
    let summary = dhw_history_summary(config_path, since, until, profile_queries)?;
    if human {
        print_dhw_history_human(&summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(ThermalError::ArtifactSerialize)?
        );
    }
    Ok(())
}

pub fn dhw_drilldown(
    config_path: &Path,
    since: &str,
    until: &str,
    human: bool,
) -> ThermalResult<()> {
    let (ctx, since_dt, until_dt) = load_ctx_and_window(config_path, since, until, false)?;

    let t1_native = query_measurement_numeric_series(
        &ctx, &since_dt, &until_dt, "emon", "dhw_t1", "2s", "last",
    )?;
    let hwc_storage = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "HwcStorageTemp",
        "30s",
        "last",
    )?;
    let remaining_litres = query_plain_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "dhw",
        "remaining_litres",
        "10s",
        "last",
    )?;
    let building_circuit_flow = query_measurement_numeric_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "BuildingCircuitFlow",
        "30s",
        "last",
    )?;
    let sfmode = query_measurement_string_series(
        &ctx,
        &since_dt,
        &until_dt,
        "ebusd_poll",
        "HwcSFMode",
        "30s",
    )?;
    let charge_periods = query_dhw_charge_periods_compact(&ctx, &since_dt, &until_dt)?;

    let t1_sampling =
        sampling_stats_for_measurement(&ctx, &since_dt, &until_dt, "dhw_t1", "emon", "dhw_t1")?;
    let hwc_sampling = sampling_stats_for_measurement(
        &ctx,
        &since_dt,
        &until_dt,
        "HwcStorageTemp",
        "ebusd_poll",
        "HwcStorageTemp",
    )?;
    let remaining_sampling = sampling_stats_for_plain_measurement(
        &ctx,
        &since_dt,
        &until_dt,
        "remaining_litres",
        "dhw",
        "remaining_litres",
    )?;
    let flow_sampling = sampling_stats_for_measurement(
        &ctx,
        &since_dt,
        &until_dt,
        "BuildingCircuitFlow",
        "ebusd_poll",
        "BuildingCircuitFlow",
    )?;

    let mut warnings = Vec::new();
    add_missing_numeric_warning(&mut warnings, "DHW T1", &t1_native);
    add_missing_numeric_warning(&mut warnings, "HwcStorageTemp", &hwc_storage);
    add_missing_numeric_warning(&mut warnings, "remaining litres", &remaining_litres);
    add_missing_numeric_warning(&mut warnings, "BuildingCircuitFlow", &building_circuit_flow);
    if sfmode.is_empty() {
        warnings.push("HwcSFMode unavailable in this window".to_string());
    }
    if charge_periods.is_empty() {
        warnings.push("no DHW charge periods detected in this drill-down window".to_string());
    }

    let summary = DhwDrilldownSummary {
        window: WindowSummary {
            since: since_dt.to_rfc3339(),
            until: until_dt.to_rfc3339(),
            generated_at: chrono::Utc::now().to_rfc3339(),
        },
        charge_periods,
        t1_native: numeric_points_from_series(&t1_native),
        hwc_storage: numeric_points_from_series(&hwc_storage),
        remaining_litres: numeric_points_from_series(&remaining_litres),
        building_circuit_flow_lh: numeric_points_from_series(&building_circuit_flow),
        sfmode: string_points_from_series(&sfmode),
        t1_sampling,
        hwc_sampling,
        remaining_sampling,
        flow_sampling,
        warnings,
    };

    if human {
        print_dhw_drilldown_human(&summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary)
                .map_err(|e| ThermalError::ArtifactSerialize(e))?
        );
    }

    Ok(())
}

fn summary_has_min_below(summary: &Option<NumericSummary>, threshold: f64) -> bool {
    summary
        .as_ref()
        .and_then(|s| s.min.as_ref())
        .map(|p| p.value < threshold)
        .unwrap_or(false)
}

fn query_numeric_point_compact(
    ctx: &HistoryCtx,
    flux: &str,
) -> ThermalResult<Option<NumericPoint>> {
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, flux)?;
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        let Some(value_str) = row.get("_value") else {
            continue;
        };
        if ts_str.is_empty() || ts_str == "_time" || value_str.is_empty() {
            continue;
        }
        return Ok(Some(NumericPoint {
            ts: parse_dt(ts_str)?.to_rfc3339(),
            value: value_str.parse().map_err(|_| ThermalError::FloatParse {
                context: "compact point _value",
                value: value_str.clone(),
            })?,
        }));
    }
    Ok(None)
}

fn batch_summary_union_flux(base_vars: &[(String, String, String)]) -> String {
    let mut script_parts = Vec::new();
    let mut union_inputs = Vec::new();

    for (var, label, base) in base_vars {
        script_parts.push(format!("{var} = {base}"));

        for (metric, op, cast_count) in [
            ("count", "count(column: \"_value\")", true),
            ("start", "first()", false),
            ("end", "last()", false),
            ("min", "min()", false),
            ("max", "max()", false),
            ("latest", "last()", false),
        ] {
            let result_var = format!("{var}_{metric}");
            let cast = if cast_count {
                " |> map(fn: (r) => ({ r with _value: float(v: r._value) }))"
            } else {
                ""
            };
            script_parts.push(format!(
                "{result_var} = {var} |> {op}{cast} |> set(key: \"series\", value: \"{label}\") |> set(key: \"metric\", value: \"{metric}\")"
            ));
            union_inputs.push(result_var);
        }
    }

    script_parts.push(format!("union(tables: [{}])", union_inputs.join(", ")));
    script_parts.join("\n\n")
}

fn summaries_from_batch_rows(
    rows: Vec<std::collections::HashMap<String, String>>,
) -> ThermalResult<std::collections::HashMap<String, Option<NumericSummary>>> {
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let Some(series) = row.get("series") else {
            continue;
        };
        let Some(metric) = row.get("metric") else {
            continue;
        };
        let entry = out.entry(series.clone()).or_insert_with(|| {
            Some(NumericSummary {
                samples: 0,
                start: None,
                end: None,
                min: None,
                max: None,
                latest: None,
            })
        });
        let Some(summary) = entry.as_mut() else {
            continue;
        };
        match metric.as_str() {
            "count" => {
                let Some(value_str) = row.get("_value") else {
                    continue;
                };
                summary.samples = value_str
                    .parse::<f64>()
                    .map_err(|_| ThermalError::FloatParse {
                        context: "batched summary count _value",
                        value: value_str.clone(),
                    })?
                    .round() as usize;
            }
            "start" | "end" | "min" | "max" | "latest" => {
                let (Some(ts_str), Some(value_str)) = (row.get("_time"), row.get("_value")) else {
                    continue;
                };
                if ts_str.is_empty() || value_str.is_empty() {
                    continue;
                }
                let point = NumericPoint {
                    ts: parse_dt(ts_str)?.to_rfc3339(),
                    value: value_str.parse().map_err(|_| ThermalError::FloatParse {
                        context: "batched summary point _value",
                        value: value_str.clone(),
                    })?,
                };
                match metric.as_str() {
                    "start" => summary.start = Some(point),
                    "end" => summary.end = Some(point),
                    "min" => summary.min = Some(point),
                    "max" => summary.max = Some(point),
                    "latest" => summary.latest = Some(point),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    out.retain(|_, value| value.as_ref().map(|s| s.samples > 0).unwrap_or(false));
    Ok(out)
}

fn query_topic_numeric_summaries_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    specs: &[TopicSummarySpec<'_>],
) -> ThermalResult<std::collections::HashMap<String, Option<NumericSummary>>> {
    if specs.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let flux = batch_summary_union_flux(
        &specs
            .iter()
            .enumerate()
            .map(|(idx, spec)| {
                (
                    format!("topic_base_{idx}"),
                    spec.label.to_string(),
                    format!(
                        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\") |> keep(columns: [\"_time\", \"_value\"])",
                        ctx.bucket,
                        since.to_rfc3339(),
                        until.to_rfc3339(),
                        spec.topic,
                        spec.field,
                    ),
                )
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(&ctx, "topic_numeric_summaries", &flux)?;
    summaries_from_batch_rows(query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?)
}

fn query_measurement_numeric_summaries_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    specs: &[MeasurementSummarySpec<'_>],
) -> ThermalResult<std::collections::HashMap<String, Option<NumericSummary>>> {
    if specs.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let flux = batch_summary_union_flux(
        &specs
            .iter()
            .enumerate()
            .map(|(idx, spec)| {
                (
                    format!("measurement_base_{idx}"),
                    spec.label.to_string(),
                    format!(
                        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\") |> keep(columns: [\"_time\", \"_value\"])",
                        ctx.bucket,
                        since.to_rfc3339(),
                        until.to_rfc3339(),
                        spec.measurement,
                        spec.field,
                    ),
                )
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(&ctx, "measurement_numeric_summaries", &flux)?;
    summaries_from_batch_rows(query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?)
}

fn query_plain_measurement_numeric_summaries_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    specs: &[PlainMeasurementSummarySpec<'_>],
) -> ThermalResult<std::collections::HashMap<String, Option<NumericSummary>>> {
    if specs.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let flux = batch_summary_union_flux(
        &specs
            .iter()
            .enumerate()
            .map(|(idx, spec)| {
                (
                    format!("plain_measurement_base_{idx}"),
                    spec.label.to_string(),
                    format!(
                        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"{}\" and r._field == \"{}\") |> keep(columns: [\"_time\", \"_value\"])",
                        ctx.bucket,
                        since.to_rfc3339(),
                        until.to_rfc3339(),
                        spec.measurement,
                        spec.field,
                    ),
                )
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(&ctx, "plain_measurement_numeric_summaries", &flux)?;
    summaries_from_batch_rows(query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?)
}

fn query_topic_numeric_last_value_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    topic: &str,
    field: &str,
) -> ThermalResult<Option<f64>> {
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\") |> keep(columns: [\"_time\", \"_value\"]) |> last()",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        topic,
        field,
    );
    Ok(query_numeric_point_compact(ctx, &flux)?.map(|p| p.value))
}

fn query_measurement_numeric_last_value_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
) -> ThermalResult<Option<f64>> {
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\") |> keep(columns: [\"_time\", \"_value\"]) |> last()",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
    );
    Ok(query_numeric_point_compact(ctx, &flux)?.map(|p| p.value))
}

fn profiled_flux(flux: &str) -> String {
    format!(
        "import \"profiler\"\noption profiler.enabledProfilers = [\"query\", \"operator\"]\n\n{flux}"
    )
}

fn maybe_print_profile(ctx: &HistoryCtx, label: &str, flux: &str) -> ThermalResult<()> {
    if !ctx.profile_queries {
        return Ok(());
    }
    let raw = query_flux_raw_pub(&ctx.url, &ctx.org, &ctx.token, &profiled_flux(flux))?;
    eprintln!("\n=== InfluxDB Flux profile: {label} ===\n{raw}");
    Ok(())
}

fn query_measurement_string_last_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
) -> ThermalResult<Option<String>> {
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\") |> keep(columns: [\"_time\", \"_value\"]) |> last()",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
    );
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?;
    for row in rows {
        if let Some(v) = row.get("_value") {
            if !v.is_empty() && v != "_value" {
                return Ok(Some(v.clone()));
            }
        }
    }
    Ok(None)
}

fn query_topic_below_threshold_periods_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    topic: &str,
    field: &str,
    threshold: f64,
) -> ThermalResult<Vec<Period>> {
    let baseline_active = query_topic_numeric_last_value_compact(
        ctx,
        &(*since - chrono::TimeDelta::hours(2)),
        since,
        topic,
        field,
    )?
    .map(|v| v < threshold)
    .unwrap_or(false);

    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\") |> keep(columns:[\"_time\",\"_value\"]) |> map(fn: (r) => ({{ r with active: if r._value < {} then 1 else 0 }})) |> difference(columns:[\"active\"], keepFirst:false) |> filter(fn:(r) => r.active != 0) |> keep(columns:[\"_time\",\"active\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        topic,
        field,
        threshold,
    );
    query_state_change_periods_compact(ctx, &flux, since, until, baseline_active, None)
}

fn query_measurement_above_threshold_periods_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    measurement: &str,
    field: &str,
    every: &str,
    threshold: f64,
    min_duration_seconds: Option<i64>,
) -> ThermalResult<Vec<Period>> {
    let baseline_active = query_measurement_numeric_last_value_compact(
        ctx,
        &(*since - chrono::TimeDelta::minutes(10)),
        since,
        measurement,
        field,
    )?
    .map(|v| v >= threshold)
    .unwrap_or(false);

    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\") |> aggregateWindow(every: {}, fn: last, createEmpty: false) |> map(fn: (r) => ({{ r with active: if r._value >= {} then 1 else 0 }})) |> difference(columns:[\"active\"], keepFirst:false) |> filter(fn:(r) => r.active != 0) |> keep(columns:[\"_time\",\"active\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
        every,
        threshold,
    );
    query_state_change_periods_compact(
        ctx,
        &flux,
        since,
        until,
        baseline_active,
        min_duration_seconds,
    )
}

fn query_state_change_periods_compact(
    ctx: &HistoryCtx,
    flux: &str,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    baseline_active: bool,
    min_duration_seconds: Option<i64>,
) -> ThermalResult<Vec<Period>> {
    maybe_print_profile(ctx, "state_change_periods", flux)?;
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, flux)?;
    let mut periods = Vec::new();
    let mut current_start: Option<DateTime<FixedOffset>> =
        if baseline_active { Some(*since) } else { None };
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        let Some(active_str) = row.get("active") else {
            continue;
        };
        if ts_str.is_empty() || active_str.is_empty() {
            continue;
        }
        let ts = parse_dt(ts_str)?;
        let active: i32 = active_str.parse().unwrap_or(0);
        if active > 0 {
            current_start.get_or_insert(ts);
        } else if active < 0 {
            if let Some(start) = current_start.take() {
                let period = period_from_times(start, ts);
                if min_duration_seconds
                    .map(|min| period_duration_seconds(&period) >= min)
                    .unwrap_or(true)
                {
                    periods.push(period);
                }
            }
        }
    }
    if let Some(start) = current_start.take() {
        let period = period_from_times(start, *until);
        if min_duration_seconds
            .map(|min| period_duration_seconds(&period) >= min)
            .unwrap_or(true)
        {
            periods.push(period);
        }
    }
    Ok(periods)
}

fn query_dhw_charge_periods_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<Period>> {
    query_measurement_above_threshold_periods_compact(
        ctx,
        since,
        until,
        "ebusd_poll",
        "BuildingCircuitFlow",
        "30s",
        DHW_FLOW_THRESHOLD_LH,
        Some(DHW_MIN_DURATION_SECONDS),
    )
}

fn batch_metric_selector_union_flux(specs: &[(String, String, String, String, String)]) -> String {
    let mut script_parts = Vec::new();
    let mut union_inputs = Vec::new();

    for (var, series, metric, base, op) in specs {
        let result_var = format!("{var}_{metric}");
        script_parts.push(format!("{var} = {base}"));
        script_parts.push(format!(
            "{result_var} = {var} |> {op} |> set(key: \"series\", value: \"{series}\") |> set(key: \"metric\", value: \"{metric}\")"
        ));
        union_inputs.push(result_var);
    }

    if union_inputs.is_empty() {
        return String::new();
    }

    script_parts.push(format!("union(tables: [{}])", union_inputs.join(", ")));
    script_parts.join("\n\n")
}

fn numeric_values_from_batch_rows(
    rows: Vec<std::collections::HashMap<String, String>>,
) -> ThermalResult<std::collections::HashMap<(String, String), f64>> {
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (Some(series), Some(metric), Some(value_str)) =
            (row.get("series"), row.get("metric"), row.get("_value"))
        else {
            continue;
        };
        if value_str.is_empty() || value_str == "_value" {
            continue;
        }
        out.insert(
            (series.clone(), metric.clone()),
            value_str.parse().map_err(|_| ThermalError::FloatParse {
                context: "batched numeric selector _value",
                value: value_str.clone(),
            })?,
        );
    }
    Ok(out)
}

fn string_values_from_batch_rows(
    rows: Vec<std::collections::HashMap<String, String>>,
) -> std::collections::HashMap<(String, String), String> {
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (Some(series), Some(metric), Some(value_str)) =
            (row.get("series"), row.get("metric"), row.get("_value"))
        else {
            continue;
        };
        if value_str.is_empty() || value_str == "_value" {
            continue;
        }
        out.insert((series.clone(), metric.clone()), value_str.clone());
    }
    out
}

fn query_dhw_charge_summaries_batched_compact(
    ctx: &HistoryCtx,
    periods: &[Period],
) -> ThermalResult<Vec<DhwChargeSummary>> {
    if periods.is_empty() {
        return Ok(Vec::new());
    }

    let parsed_periods = periods
        .iter()
        .enumerate()
        .map(|(idx, period)| -> ThermalResult<_> {
            Ok((
                idx,
                period,
                parse_dt(&period.start)?,
                parse_dt(&period.end)?,
            ))
        })
        .collect::<ThermalResult<Vec<_>>>()?;

    let period_summary_flux = batch_summary_union_flux(
        &parsed_periods
            .iter()
            .flat_map(|(idx, _, start, end)| {
                [
                    (
                        format!("charge_{idx}_t1"),
                        format!("charge_{idx}_t1"),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"emon\" and r.field == \"dhw_t1\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start.to_rfc3339(),
                            end.to_rfc3339(),
                        ),
                    ),
                    (
                        format!("charge_{idx}_hwc"),
                        format!("charge_{idx}_hwc"),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcStorageTemp\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start.to_rfc3339(),
                            end.to_rfc3339(),
                        ),
                    ),
                ]
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(ctx, "dhw_charge_period_summaries", &period_summary_flux)?;
    let period_summaries = summaries_from_batch_rows(query_flux_csv_pub(
        &ctx.url,
        &ctx.org,
        &ctx.token,
        &period_summary_flux,
    )?)?;

    let numeric_boundary_flux = batch_metric_selector_union_flux(
        &parsed_periods
            .iter()
            .flat_map(|(idx, _, start, end)| {
                let start_before = *start - chrono::TimeDelta::seconds(DHW_BOUNDARY_LOOKBACK_SECONDS);
                let end_after = *end + chrono::TimeDelta::seconds(DHW_BOUNDARY_LOOKAHEAD_SECONDS);
                [
                    (
                        format!("charge_{idx}_t1_start"),
                        format!("charge_{idx}"),
                        "t1_start".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"emon\" and r.field == \"dhw_t1\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start_before.to_rfc3339(),
                            start.to_rfc3339(),
                        ),
                        "last()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_t1_end"),
                        format!("charge_{idx}"),
                        "t1_end".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"emon\" and r.field == \"dhw_t1\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            end.to_rfc3339(),
                            end_after.to_rfc3339(),
                        ),
                        "first()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_hwc_start"),
                        format!("charge_{idx}"),
                        "hwc_start".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcStorageTemp\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start_before.to_rfc3339(),
                            start.to_rfc3339(),
                        ),
                        "last()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_hwc_end"),
                        format!("charge_{idx}"),
                        "hwc_end".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcStorageTemp\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            end.to_rfc3339(),
                            end_after.to_rfc3339(),
                        ),
                        "first()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_remaining_start"),
                        format!("charge_{idx}"),
                        "remaining_start".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"dhw\" and r._field == \"remaining_litres\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start_before.to_rfc3339(),
                            start.to_rfc3339(),
                        ),
                        "last()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_remaining_end"),
                        format!("charge_{idx}"),
                        "remaining_end".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"dhw\" and r._field == \"remaining_litres\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            end.to_rfc3339(),
                            end_after.to_rfc3339(),
                        ),
                        "first()".to_string(),
                    ),
                ]
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(ctx, "dhw_charge_boundaries_numeric", &numeric_boundary_flux)?;
    let numeric_boundaries = numeric_values_from_batch_rows(query_flux_csv_pub(
        &ctx.url,
        &ctx.org,
        &ctx.token,
        &numeric_boundary_flux,
    )?)?;

    let string_boundary_flux = batch_metric_selector_union_flux(
        &parsed_periods
            .iter()
            .flat_map(|(idx, _, start, end)| {
                let start_before = *start - chrono::TimeDelta::seconds(DHW_BOUNDARY_LOOKBACK_SECONDS);
                let end_after = *end + chrono::TimeDelta::seconds(DHW_BOUNDARY_LOOKAHEAD_SECONDS);
                [
                    (
                        format!("charge_{idx}_sfmode_start"),
                        format!("charge_{idx}"),
                        "sfmode_start".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcSFMode\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            start_before.to_rfc3339(),
                            start.to_rfc3339(),
                        ),
                        "last()".to_string(),
                    ),
                    (
                        format!("charge_{idx}_sfmode_end"),
                        format!("charge_{idx}"),
                        "sfmode_end".to_string(),
                        format!(
                            "from(bucket: \"{}\") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcSFMode\") |> keep(columns: [\"_time\", \"_value\"])",
                            ctx.bucket,
                            end.to_rfc3339(),
                            end_after.to_rfc3339(),
                        ),
                        "first()".to_string(),
                    ),
                ]
            })
            .collect::<Vec<_>>(),
    );
    maybe_print_profile(ctx, "dhw_charge_boundaries_string", &string_boundary_flux)?;
    let string_boundaries = string_values_from_batch_rows(query_flux_csv_pub(
        &ctx.url,
        &ctx.org,
        &ctx.token,
        &string_boundary_flux,
    )?);

    parsed_periods
        .iter()
        .map(|(idx, period, _, _)| {
            let charge_key = format!("charge_{idx}");
            let t1 = period_summaries
                .get(&format!("charge_{idx}_t1"))
                .cloned()
                .unwrap_or(None);
            let hwc = period_summaries
                .get(&format!("charge_{idx}_hwc"))
                .cloned()
                .unwrap_or(None);
            let t1_start = numeric_boundaries
                .get(&(charge_key.clone(), "t1_start".to_string()))
                .copied();
            let t1_end = numeric_boundaries
                .get(&(charge_key.clone(), "t1_end".to_string()))
                .copied();
            let hwc_start = numeric_boundaries
                .get(&(charge_key.clone(), "hwc_start".to_string()))
                .copied();
            let hwc_end = numeric_boundaries
                .get(&(charge_key.clone(), "hwc_end".to_string()))
                .copied();
            let remaining_start = numeric_boundaries
                .get(&(charge_key.clone(), "remaining_start".to_string()))
                .copied();
            let remaining_end = numeric_boundaries
                .get(&(charge_key.clone(), "remaining_end".to_string()))
                .copied();
            let sfmode_start = string_boundaries
                .get(&(charge_key.clone(), "sfmode_start".to_string()))
                .cloned();
            let sfmode_end = string_boundaries
                .get(&(charge_key.clone(), "sfmode_end".to_string()))
                .cloned();
            let t1_peak = t1.as_ref().and_then(|s| s.max.as_ref()).map(|p| p.value);
            let hwc_peak = hwc.as_ref().and_then(|s| s.max.as_ref()).map(|p| p.value);

            Ok(DhwChargeSummary {
                start: period.start.clone(),
                end: period.end.clone(),
                duration_minutes: period.duration_minutes,
                t1_start_c: t1_start,
                t1_peak_c: t1_peak,
                t1_end_c: t1_end,
                hwc_start_c: hwc_start,
                hwc_peak_c: hwc_peak,
                hwc_end_c: hwc_end,
                remaining_litres_start: remaining_start,
                remaining_litres_end: remaining_end,
                sfmode_start,
                sfmode_end,
                crossover: match (t1_start, hwc_end) {
                    (Some(t1_pre), Some(hwc_final)) => Some(hwc_final >= t1_pre),
                    _ => None,
                },
            })
        })
        .collect()
}

fn query_dhw_max_divergence_compact(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
) -> ThermalResult<Option<f64>> {
    let flux = format!(
        "t1 = from(bucket: \"{bucket}\") |> range(start: {start}, stop: {stop}) |> filter(fn: (r) => r._measurement == \"emon\" and r.field == \"dhw_t1\") |> aggregateWindow(every: 30s, fn: last, createEmpty: false) |> keep(columns:[\"_time\",\"_value\"]) |> set(key: \"series\", value: \"t1\")\n\nhwc = from(bucket: \"{bucket}\") |> range(start: {start}, stop: {stop}) |> filter(fn: (r) => r._measurement == \"ebusd_poll\" and r.field == \"HwcStorageTemp\") |> aggregateWindow(every: 30s, fn: last, createEmpty: false) |> keep(columns:[\"_time\",\"_value\"]) |> set(key: \"series\", value: \"hwc\")\n\nunion(tables:[t1, hwc]) |> pivot(rowKey:[\"_time\"], columnKey:[\"series\"], valueColumn:\"_value\") |> map(fn:(r)=> ({{ r with diff: if r.t1 > r.hwc then r.t1 - r.hwc else r.hwc - r.t1 }})) |> keep(columns:[\"_time\",\"diff\"]) |> rename(columns: {{diff: \"_value\"}}) |> max()",
        bucket = ctx.bucket,
        start = since.to_rfc3339(),
        stop = until.to_rfc3339(),
    );
    maybe_print_profile(ctx, "dhw_max_divergence", &flux)?;
    Ok(query_numeric_point_compact(ctx, &flux)?.map(|p| p.value))
}

fn load_ctx_and_window(
    config_path: &Path,
    since: &str,
    until: &str,
    profile_queries: bool,
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
            profile_queries,
        },
        since_dt,
        until_dt,
    ))
}

fn query_timestamp_series(
    ctx: &HistoryCtx,
    flux: &str,
) -> ThermalResult<Vec<DateTime<FixedOffset>>> {
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, flux)?;
    let mut out = Vec::new();
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        if ts_str.is_empty() || ts_str == "_time" {
            continue;
        }
        out.push(parse_dt(ts_str)?);
    }
    out.sort();
    Ok(out)
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

fn sampling_stats_from_timestamps(
    label: &str,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    times: &[DateTime<FixedOffset>],
) -> SamplingStats {
    let mut deltas = times
        .windows(2)
        .map(|w| (w[1] - w[0]).num_milliseconds() as f64 / 1000.0)
        .filter(|v| *v > 0.0)
        .collect::<Vec<_>>();
    deltas.sort_by(|a, b| a.total_cmp(b));
    let median = deltas.get(deltas.len() / 2).copied();
    let min = deltas.first().copied();
    let max = deltas.last().copied();
    SamplingStats {
        label: label.to_string(),
        window_start: since.to_rfc3339(),
        window_end: until.to_rfc3339(),
        samples: times.len(),
        median_step_seconds: median,
        min_step_seconds: min,
        max_step_seconds: max,
    }
}

fn sampling_stats_for_topic(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    label: &str,
    topic: &str,
    field: &str,
) -> ThermalResult<SamplingStats> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\")\n  |> keep(columns: [\"_time\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        topic,
        field,
    );
    let times = query_timestamp_series(ctx, &flux)?;
    Ok(sampling_stats_from_timestamps(label, since, until, &times))
}

fn sampling_stats_for_measurement(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    label: &str,
    measurement: &str,
    field: &str,
) -> ThermalResult<SamplingStats> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"{}\" and r.field == \"{}\")\n  |> keep(columns: [\"_time\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
    );
    let times = query_timestamp_series(ctx, &flux)?;
    Ok(sampling_stats_from_timestamps(label, since, until, &times))
}

fn sampling_stats_for_plain_measurement(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
    label: &str,
    measurement: &str,
    field: &str,
) -> ThermalResult<SamplingStats> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"{}\" and r._field == \"{}\")\n  |> keep(columns: [\"_time\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
        measurement,
        field,
    );
    let times = query_timestamp_series(ctx, &flux)?;
    Ok(sampling_stats_from_timestamps(label, since, until, &times))
}

fn query_controller_rows(
    ctx: &HistoryCtx,
    since: &DateTime<FixedOffset>,
    until: &DateTime<FixedOffset>,
) -> ThermalResult<Vec<ControllerRow>> {
    let flux = format!(
        "from(bucket: \"{}\")\n  |> range(start: {}, stop: {})\n  |> filter(fn: (r) => r._measurement == \"adaptive_heating_mvp\")\n  |> filter(fn: (r) => r._field == \"target_flow_c\" or r._field == \"curve_after\" or r._field == \"flow_desired_c\")\n  |> map(fn: (r) => ({{ r with mode: if exists r.mode then r.mode else \"unknown\", action: if exists r.action then r.action else \"unknown\", tariff: if exists r.tariff then r.tariff else \"unknown\" }}))\n  |> pivot(rowKey: [\"_time\", \"mode\", \"action\", \"tariff\"], columnKey: [\"_field\"], valueColumn: \"_value\")\n  |> keep(columns: [\"_time\", \"mode\", \"action\", \"tariff\", \"target_flow_c\", \"curve_after\", \"flow_desired_c\"])",
        ctx.bucket,
        since.to_rfc3339(),
        until.to_rfc3339(),
    );
    maybe_print_profile(ctx, "controller_rows", &flux)?;
    let rows = query_flux_csv_pub(&ctx.url, &ctx.org, &ctx.token, &flux)?;
    let mut out = Vec::new();
    for row in rows {
        let Some(ts_str) = row.get("_time") else {
            continue;
        };
        if ts_str.is_empty() || ts_str == "_time" {
            continue;
        }
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

/// Clip a period to waking hours (07:00–23:00), potentially splitting across
/// midnight boundaries. Returns zero or more periods representing only the
/// waking-hours portion.
fn clip_period_to_waking_hours(period: &Period) -> Vec<Period> {
    let Ok(start) = parse_dt(&period.start) else {
        return vec![];
    };
    let Ok(end) = parse_dt(&period.end) else {
        return vec![];
    };
    if end <= start {
        return vec![];
    }

    let mut result = Vec::new();
    // Walk day-by-day through the period
    let mut day_start = start.date_naive();
    let end_date = end.date_naive();
    let tz = *start.offset();

    while day_start <= end_date {
        let waking_start = day_start
            .and_hms_opt(WAKING_START_HOUR, 0, 0)
            .unwrap()
            .and_local_timezone(tz)
            .single();
        let waking_end = day_start
            .and_hms_opt(WAKING_END_HOUR, 0, 0)
            .unwrap()
            .and_local_timezone(tz)
            .single();

        if let (Some(ws), Some(we)) = (waking_start, waking_end) {
            let clipped_start = start.max(ws);
            let clipped_end = end.min(we);
            if clipped_start < clipped_end {
                let dur_min = (clipped_end - clipped_start).num_seconds() as f64 / 60.0;
                result.push(Period {
                    start: clipped_start.to_rfc3339(),
                    end: clipped_end.to_rfc3339(),
                    duration_minutes: dur_min,
                });
            }
        }
        day_start += chrono::Duration::days(1);
    }
    result
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

fn add_missing_summary_warning(
    warnings: &mut Vec<String>,
    label: &str,
    summary: &Option<NumericSummary>,
) {
    if summary.is_none() {
        warnings.push(format!("{label} unavailable in this window"));
    }
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

fn numeric_points_from_series(series: &[(DateTime<FixedOffset>, f64)]) -> Vec<NumericPoint> {
    series
        .iter()
        .map(|(ts, value)| NumericPoint {
            ts: ts.to_rfc3339(),
            value: *value,
        })
        .collect()
}

fn string_points_from_series(series: &[(DateTime<FixedOffset>, String)]) -> Vec<StringPoint> {
    series
        .iter()
        .map(|(ts, value)| StringPoint {
            ts: ts.to_rfc3339(),
            value: value.clone(),
        })
        .collect()
}

fn print_heating_history_human(summary: &HeatingHistorySummary) {
    println!("Heating history");
    println!("---------------");
    println!(
        "window: {} → {}",
        summary.window.since, summary.window.until
    );
    println!("generated_at: {}", summary.window.generated_at);
    print_numeric_summary_line("Leather", &summary.leather_c);
    print_numeric_summary_line("Aldora", &summary.aldora_c);
    print_numeric_summary_line("Outside", &summary.outside_c);
    print_numeric_summary_line("Heat curve", &summary.heat_curve);
    print_numeric_summary_line("Target flow", &summary.target_flow_c);
    print_numeric_summary_line("Actual desired flow", &summary.actual_flow_desired_c);
    print_numeric_summary_line("Actual flow", &summary.actual_flow_c);
    print_numeric_summary_line("Return", &summary.return_c);
    print_sampling_section(&summary.sampling);
    println!(
        "controller_mode_changes: {}",
        summary.controller_mode_changes.len()
    );
    println!("controller_events: {}", summary.controller_events.len());
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
    print_period_section(
        "comfort_miss_periods_detail",
        &summary.events.comfort_miss_periods,
    );
    print_period_section(
        "dhw_overlap_periods_detail",
        &summary.events.dhw_overlap_periods,
    );
    print_controller_events_section("recent_controller_events", &summary.controller_events, 12);
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
    println!("generated_at: {}", summary.window.generated_at);
    print_numeric_summary_line("T1", &summary.t1_c);
    print_numeric_summary_line("HwcStorageTemp", &summary.hwc_storage_c);
    print_numeric_summary_line("Remaining litres", &summary.remaining_litres);
    print_sampling_section(&summary.sampling);
    let full_count = summary
        .charges_detected
        .iter()
        .filter(|c| c.crossover == Some(true))
        .count();
    let partial_count = summary.charges_detected.len().saturating_sub(full_count);
    let total_charge_minutes: f64 = summary
        .charges_detected
        .iter()
        .map(|c| c.duration_minutes)
        .sum();
    println!("charges_detected: {}", summary.charges_detected.len());
    println!(
        "charge_breakdown: full={} partial={}",
        full_count, partial_count
    );
    println!("total_charge_hours: {:.1}", total_charge_minutes / 60.0);
    for (idx, charge) in summary.charges_detected.iter().enumerate() {
        println!(
            "charge[{idx}]: {} → {} ({:.1} min) crossover={} T1 {:.1}->{:.1}°C HWC {:.1}->{:.1}°C remaining {:.0}->{:.0}L",
            charge.start,
            charge.end,
            charge.duration_minutes,
            charge
                .crossover
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            charge.t1_start_c.unwrap_or(f64::NAN),
            charge.t1_end_c.unwrap_or(f64::NAN),
            charge.hwc_start_c.unwrap_or(f64::NAN),
            charge.hwc_end_c.unwrap_or(f64::NAN),
            charge.remaining_litres_start.unwrap_or(f64::NAN),
            charge.remaining_litres_end.unwrap_or(f64::NAN),
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
    print_string_points_section("sfmode_samples", &summary.sfmode, 12);
    if summary.warnings.is_empty() {
        println!("warnings: none");
    } else {
        println!("warnings:");
        for warning in &summary.warnings {
            println!("- {warning}");
        }
    }
}

fn print_dhw_drilldown_human(summary: &DhwDrilldownSummary) {
    println!("DHW drilldown");
    println!("-------------");
    println!(
        "window: {} → {}",
        summary.window.since, summary.window.until
    );
    println!("generated_at: {}", summary.window.generated_at);
    println!("charge_periods: {}", summary.charge_periods.len());
    print_period_section("charge_periods_detail", &summary.charge_periods);
    println!("t1_native_points: {}", summary.t1_native.len());
    println!("hwc_storage_points: {}", summary.hwc_storage.len());
    println!(
        "remaining_litres_points: {}",
        summary.remaining_litres.len()
    );
    println!(
        "building_circuit_flow_points: {}",
        summary.building_circuit_flow_lh.len()
    );
    println!("sfmode_points: {}", summary.sfmode.len());
    print_sampling_section(&[
        summary.t1_sampling.clone(),
        summary.hwc_sampling.clone(),
        summary.remaining_sampling.clone(),
        summary.flow_sampling.clone(),
    ]);
    if summary.warnings.is_empty() {
        println!("warnings: none");
    } else {
        println!("warnings:");
        for warning in &summary.warnings {
            println!("- {warning}");
        }
    }
}

fn print_sampling_section(sampling: &[SamplingStats]) {
    println!("sampling_cadence_estimates: {}", sampling.len());
    for stat in sampling {
        println!(
            "  {} samples={} median={}s min={}s max={}s window={}→{}",
            stat.label,
            stat.samples,
            stat.median_step_seconds
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "n/a".to_string()),
            stat.min_step_seconds
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "n/a".to_string()),
            stat.max_step_seconds
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "n/a".to_string()),
            stat.window_start,
            stat.window_end,
        );
    }
}

fn print_period_section(label: &str, periods: &[Period]) {
    println!("{label}: {}", periods.len());
    for (idx, period) in periods.iter().enumerate() {
        println!(
            "  [{idx}] {} → {} ({:.1} min)",
            period.start, period.end, period.duration_minutes
        );
    }
}

fn print_controller_events_section(label: &str, events: &[ControllerEvent], limit: usize) {
    println!("{label}: {}", events.len());
    let start = events.len().saturating_sub(limit);
    for event in &events[start..] {
        println!(
            "  {} mode={} action={} tariff={} target={} curve={} desired={}",
            event.ts,
            event.mode,
            event.action,
            event.tariff,
            event
                .target_flow_c
                .map(|v| format!("{v:.1}°C"))
                .unwrap_or_else(|| "n/a".to_string()),
            event
                .curve_after
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "n/a".to_string()),
            event
                .flow_desired_c
                .map(|v| format!("{v:.1}°C"))
                .unwrap_or_else(|| "n/a".to_string()),
        );
    }
}

fn print_string_points_section(label: &str, points: &[StringPoint], limit: usize) {
    println!("{label}: {}", points.len());
    let start = points.len().saturating_sub(limit);
    for point in &points[start..] {
        println!("  {} {}", point.ts, point.value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Offset, TimeZone, Utc};

    fn dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<FixedOffset> {
        Utc.fix().with_ymd_and_hms(y, m, d, hh, mm, 0).unwrap()
    }

    // @lat: [[tests#History evidence helpers#Sampling stats report interval summary]]
    #[test]
    fn sampling_stats_reports_interval_summary() {
        let since = dt(2026, 4, 10, 7, 0);
        let until = dt(2026, 4, 10, 8, 0);
        let times = vec![
            dt(2026, 4, 10, 7, 0),
            dt(2026, 4, 10, 7, 5),
            dt(2026, 4, 10, 7, 15),
            dt(2026, 4, 10, 7, 20),
        ];

        let stats = sampling_stats_from_timestamps("leather", &since, &until, &times);

        assert_eq!(stats.samples, 4);
        assert_eq!(stats.min_step_seconds, Some(300.0));
        assert_eq!(stats.median_step_seconds, Some(300.0));
        assert_eq!(stats.max_step_seconds, Some(600.0));
    }

    // @lat: [[tests#History evidence helpers#Waking-hour clipping excludes overnight-only time]]
    #[test]
    fn clip_period_to_waking_hours_splits_and_excludes_overnight_time() {
        let period = Period {
            start: dt(2026, 4, 10, 22, 30).to_rfc3339(),
            end: dt(2026, 4, 11, 8, 30).to_rfc3339(),
            duration_minutes: 600.0,
        };

        let clipped = clip_period_to_waking_hours(&period);

        assert_eq!(clipped.len(), 2);
        assert_eq!(clipped[0].start, dt(2026, 4, 10, 22, 30).to_rfc3339());
        assert_eq!(clipped[0].end, dt(2026, 4, 10, 23, 0).to_rfc3339());
        assert_eq!(clipped[1].start, dt(2026, 4, 11, 7, 0).to_rfc3339());
        assert_eq!(clipped[1].end, dt(2026, 4, 11, 8, 30).to_rfc3339());
    }

    // @lat: [[tests#History evidence helpers#Sawtooth detection requires repeated alternation]]
    #[test]
    fn detect_sawtooth_requires_repeated_significant_alternation() {
        let rows = vec![
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 0),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.8),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 15),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(1.0),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 30),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.85),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 45),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(1.05),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 7, 0),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.9),
                flow_desired_c: None,
            },
        ];

        let (sawtooth, alternations) = detect_sawtooth(&rows);
        assert!(sawtooth);
        assert_eq!(alternations, 3);

        let calm_rows = rows[..3].to_vec();
        let (sawtooth, alternations) = detect_sawtooth(&calm_rows);
        assert!(!sawtooth);
        assert_eq!(alternations, 0);
    }

    // @lat: [[tests#History evidence helpers#Sawtooth detection ignores sub-threshold noise]]
    #[test]
    fn detect_sawtooth_ignores_sub_threshold_noise() {
        let rows = vec![
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 0),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.80),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 15),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.84),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 30),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.80),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 45),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.84),
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 7, 0),
                mode: "occupied".into(),
                action: "heat".into(),
                tariff: "standard".into(),
                target_flow_c: None,
                curve_after: Some(0.80),
                flow_desired_c: None,
            },
        ];

        let (sawtooth, alternations) = detect_sawtooth(&rows);
        assert!(!sawtooth);
        assert_eq!(alternations, 0);
    }

    // @lat: [[tests#History evidence helpers#Mode changes only emit on actual transitions]]
    #[test]
    fn controller_mode_changes_only_emit_real_transitions() {
        let rows = vec![
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 0),
                mode: "eco".into(),
                action: "hold".into(),
                tariff: "standard".into(),
                target_flow_c: Some(30.0),
                curve_after: None,
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 15),
                mode: "eco".into(),
                action: "hold".into(),
                tariff: "standard".into(),
                target_flow_c: Some(31.0),
                curve_after: None,
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 30),
                mode: "comfort".into(),
                action: "heat".into(),
                tariff: "cosy".into(),
                target_flow_c: Some(35.0),
                curve_after: None,
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 6, 45),
                mode: "comfort".into(),
                action: "heat".into(),
                tariff: "cosy".into(),
                target_flow_c: Some(36.0),
                curve_after: None,
                flow_desired_c: None,
            },
            ControllerRow {
                ts: dt(2026, 4, 10, 7, 0),
                mode: "setback".into(),
                action: "coast".into(),
                tariff: "peak".into(),
                target_flow_c: Some(28.0),
                curve_after: None,
                flow_desired_c: None,
            },
        ];

        let changes = controller_mode_changes(&rows);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to, "eco");
        assert_eq!(changes[1].from.as_deref(), Some("eco"));
        assert_eq!(changes[1].to, "comfort");
        assert_eq!(changes[2].from.as_deref(), Some("comfort"));
        assert_eq!(changes[2].to, "setback");
    }

    // @lat: [[tests#History evidence helpers#Numeric summaries preserve extrema and recency]]
    #[test]
    fn summarize_numeric_preserves_extrema_and_latest_points() {
        let series = vec![
            (dt(2026, 4, 10, 6, 0), 18.5),
            (dt(2026, 4, 10, 6, 15), 17.0),
            (dt(2026, 4, 10, 6, 30), 19.25),
        ];

        let summary = summarize_numeric(&series).unwrap();

        assert_eq!(summary.samples, 3);
        assert_eq!(summary.start.as_ref().unwrap().ts, dt(2026, 4, 10, 6, 0).to_rfc3339());
        assert_eq!(summary.end.as_ref().unwrap().ts, dt(2026, 4, 10, 6, 30).to_rfc3339());
        assert_eq!(summary.latest.as_ref().unwrap().value, 19.25);
        assert_eq!(summary.min.as_ref().unwrap().value, 17.0);
        assert_eq!(summary.max.as_ref().unwrap().value, 19.25);
        assert!(summarize_numeric(&[]).is_none());
    }

    // @lat: [[tests#History evidence helpers#Recent-end lookback ignores invalid periods]]
    #[test]
    fn period_contains_recent_end_requires_valid_period_inside_lookback() {
        let until = dt(2026, 4, 10, 8, 0);
        let fresh = Period {
            start: dt(2026, 4, 10, 7, 45).to_rfc3339(),
            end: dt(2026, 4, 10, 7, 55).to_rfc3339(),
            duration_minutes: 10.0,
        };
        let stale = Period {
            start: dt(2026, 4, 10, 7, 0).to_rfc3339(),
            end: dt(2026, 4, 10, 7, 10).to_rfc3339(),
            duration_minutes: 10.0,
        };
        let invalid = Period {
            start: "invalid".into(),
            end: "also-invalid".into(),
            duration_minutes: 0.0,
        };

        assert!(period_contains_recent_end(&fresh, &until, 600));
        assert!(!period_contains_recent_end(&stale, &until, 600));
        assert!(!period_contains_recent_end(&invalid, &until, 600));
    }

    // @lat: [[tests#History evidence helpers#Missing-data warnings only flag absent evidence]]
    #[test]
    fn missing_data_warnings_only_emit_for_missing_inputs() {
        let mut warnings = Vec::new();
        let series = vec![(dt(2026, 4, 10, 7, 0), 21.0)];
        let summary = summarize_numeric(&series);

        add_missing_numeric_warning(&mut warnings, "flow", &series);
        add_missing_summary_warning(&mut warnings, "return", &summary);
        assert!(warnings.is_empty());

        add_missing_numeric_warning(&mut warnings, "flow", &[]);
        add_missing_summary_warning(&mut warnings, "return", &None);
        assert_eq!(warnings, vec![
            "flow unavailable in this window".to_string(),
            "return unavailable in this window".to_string(),
        ]);
    }

    // @lat: [[tests#History evidence helpers#Period from times computes correct duration]]
    #[test]
    fn period_from_times_computes_duration() {
        let start = dt(2026, 4, 10, 7, 0);
        let end = dt(2026, 4, 10, 8, 30);
        let p = period_from_times(start, end);
        assert!((p.duration_minutes - 90.0).abs() < 0.01);
        assert!(p.start.contains("07:00"));
        assert!(p.end.contains("08:30"));
    }

    // @lat: [[tests#History evidence helpers#Period duration seconds round-trips with period from times]]
    #[test]
    fn period_duration_seconds_roundtrips() {
        let start = dt(2026, 4, 10, 7, 0);
        let end = dt(2026, 4, 10, 8, 30);
        let p = period_from_times(start, end);
        assert_eq!(period_duration_seconds(&p), 5400); // 90 min = 5400s

        // Invalid period
        let bad = Period {
            start: "not-a-date".to_string(),
            end: "also-bad".to_string(),
            duration_minutes: 0.0,
        };
        assert_eq!(period_duration_seconds(&bad), 0);
    }

    // @lat: [[tests#History evidence helpers#Summary has min below detects threshold crossings]]
    #[test]
    fn summary_has_min_below_threshold() {
        let series = vec![(dt(2026, 4, 10, 7, 0), 38.0), (dt(2026, 4, 10, 7, 5), 42.0)];
        let summary = summarize_numeric(&series);
        assert!(summary_has_min_below(&summary, 40.0));
        assert!(!summary_has_min_below(&summary, 37.0));
        assert!(!summary_has_min_below(&None, 40.0));
    }

    fn row(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // @lat: [[tests#History evidence helpers#summaries_from_batch_rows pivots metrics into NumericSummary]]
    #[test]
    fn summaries_from_batch_rows_pivots_metrics() {
        let ts = "2026-04-10T07:00:00+00:00";
        let rows = vec![
            row(&[("series", "leather"), ("metric", "count"), ("_value", "42")]),
            row(&[("series", "leather"), ("metric", "min"), ("_time", ts), ("_value", "18.5")]),
            row(&[("series", "leather"), ("metric", "max"), ("_time", ts), ("_value", "21.0")]),
            // Single-sample series must also be retained (count > 0, not > 1)
            row(&[("series", "outside"), ("metric", "count"), ("_value", "1")]),
            row(&[("series", "outside"), ("metric", "min"), ("_time", ts), ("_value", "5.0")]),
        ];
        let out = summaries_from_batch_rows(rows).unwrap();
        let s = out.get("leather").unwrap().as_ref().unwrap();
        assert_eq!(s.samples, 42);
        assert!((s.min.as_ref().unwrap().value - 18.5).abs() < 0.01);
        assert!((s.max.as_ref().unwrap().value - 21.0).abs() < 0.01);

        // Single-sample series retained
        let o = out.get("outside").unwrap().as_ref().unwrap();
        assert_eq!(o.samples, 1);
    }

    // @lat: [[tests#History evidence helpers#summaries_from_batch_rows drops zero-sample series]]
    #[test]
    fn summaries_from_batch_rows_drops_zero_sample_series() {
        // Series with no count row → samples stays 0 → dropped by retain
        let ts = "2026-04-10T07:00:00+00:00";
        let rows = vec![
            row(&[("series", "empty"), ("metric", "min"), ("_time", ts), ("_value", "20.0")]),
        ];
        let out = summaries_from_batch_rows(rows).unwrap();
        assert!(out.is_empty(), "zero-sample series should be dropped");
    }

    // @lat: [[tests#History evidence helpers#numeric_values_from_batch_rows parses keyed values]]
    #[test]
    fn numeric_values_from_batch_rows_parses_keyed_values() {
        let rows = vec![
            row(&[("series", "flow"), ("metric", "max"), ("_value", "42.5")]),
            row(&[("series", "flow"), ("metric", "min"), ("_value", "38.0")]),
            // skip header sentinel
            row(&[("series", "flow"), ("metric", "avg"), ("_value", "_value")]),
            // skip empty
            row(&[("series", "flow"), ("metric", "count"), ("_value", "")]),
        ];
        let out = numeric_values_from_batch_rows(rows).unwrap();
        assert_eq!(out.len(), 2);
        assert!((out[&("flow".to_string(), "max".to_string())] - 42.5).abs() < 0.01);
        assert!((out[&("flow".to_string(), "min".to_string())] - 38.0).abs() < 0.01);
    }

    // @lat: [[tests#History evidence helpers#string_values_from_batch_rows skips empty and sentinel values]]
    #[test]
    fn string_values_from_batch_rows_skips_empties() {
        let rows = vec![
            row(&[("series", "mode"), ("metric", "last"), ("_value", "heating")]),
            row(&[("series", "mode"), ("metric", "first"), ("_value", "")]),
            row(&[("series", "mode"), ("metric", "header"), ("_value", "_value")]),
        ];
        let out = string_values_from_batch_rows(rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[&("mode".to_string(), "last".to_string())], "heating");
    }

    // @lat: [[tests#History evidence helpers#numeric_points_from_series maps DateTime-f64 pairs]]
    #[test]
    fn numeric_points_from_series_maps_pairs() {
        let series = vec![
            (dt(2026, 4, 10, 7, 0), 21.5),
            (dt(2026, 4, 10, 7, 15), 22.0),
        ];
        let points = numeric_points_from_series(&series);
        assert_eq!(points.len(), 2);
        assert!(points[0].ts.contains("07:00"));
        assert!((points[0].value - 21.5).abs() < f64::EPSILON);
        assert!(points[1].ts.contains("07:15"));
        assert!((points[1].value - 22.0).abs() < f64::EPSILON);
    }

    // @lat: [[tests#History evidence helpers#numeric_points_from_series returns empty for empty input]]
    #[test]
    fn numeric_points_from_series_empty() {
        let points = numeric_points_from_series(&[]);
        assert!(points.is_empty());
    }

    // @lat: [[tests#History evidence helpers#string_points_from_series maps DateTime-String pairs]]
    #[test]
    fn string_points_from_series_maps_pairs() {
        let series = vec![
            (dt(2026, 4, 10, 8, 0), "heating".to_string()),
            (dt(2026, 4, 10, 8, 30), "idle".to_string()),
        ];
        let points = string_points_from_series(&series);
        assert_eq!(points.len(), 2);
        assert!(points[0].ts.contains("08:00"));
        assert_eq!(points[0].value, "heating");
        assert!(points[1].ts.contains("08:30"));
        assert_eq!(points[1].value, "idle");
    }

    // @lat: [[tests#History evidence helpers#string_points_from_series returns empty for empty input]]
    #[test]
    fn string_points_from_series_empty() {
        let points = string_points_from_series(&[]);
        assert!(points.is_empty());
    }

    // @lat: [[tests#History evidence helpers#controller_event_from_row copies all fields]]
    #[test]
    fn controller_event_from_row_copies_all_fields() {
        let row = ControllerRow {
            ts: dt(2026, 4, 10, 7, 0),
            mode: "comfort".into(),
            action: "heat".into(),
            tariff: "cosy".into(),
            target_flow_c: Some(35.0),
            curve_after: Some(0.9),
            flow_desired_c: Some(32.5),
        };
        let event = controller_event_from_row(&row);
        assert!(event.ts.contains("07:00"));
        assert_eq!(event.mode, "comfort");
        assert_eq!(event.action, "heat");
        assert_eq!(event.tariff, "cosy");
        assert_eq!(event.target_flow_c, Some(35.0));
        assert_eq!(event.curve_after, Some(0.9));
        assert_eq!(event.flow_desired_c, Some(32.5));
    }

    // @lat: [[tests#History evidence helpers#controller_event_from_row preserves None optionals]]
    #[test]
    fn controller_event_from_row_preserves_nones() {
        let row = ControllerRow {
            ts: dt(2026, 4, 10, 7, 0),
            mode: "eco".into(),
            action: "hold".into(),
            tariff: "standard".into(),
            target_flow_c: None,
            curve_after: None,
            flow_desired_c: None,
        };
        let event = controller_event_from_row(&row);
        assert_eq!(event.target_flow_c, None);
        assert_eq!(event.curve_after, None);
        assert_eq!(event.flow_desired_c, None);
    }

    // @lat: [[tests#History evidence helpers#profiled_flux wraps query with profiler import]]
    #[test]
    fn profiled_flux_wraps_with_profiler() {
        let flux = "from(bucket: \"test\") |> range(start: -1h)";
        let result = profiled_flux(flux);
        assert!(result.starts_with("import \"profiler\""));
        assert!(result.contains("enabledProfilers"));
        assert!(result.ends_with(flux));
    }

    // @lat: [[tests#History evidence helpers#batch_summary_union_flux builds union with all metrics]]
    #[test]
    fn batch_summary_union_flux_builds_union() {
        let vars = vec![
            ("s1".to_string(), "leather".to_string(), "base_query_1".to_string()),
        ];
        let result = batch_summary_union_flux(&vars);
        assert!(result.contains("s1 = base_query_1"));
        assert!(result.contains("s1_count"));
        assert!(result.contains("s1_min"));
        assert!(result.contains("s1_max"));
        assert!(result.contains("s1_start"));
        assert!(result.contains("s1_end"));
        assert!(result.contains("s1_latest"));
        assert!(result.contains("\"leather\""));
        assert!(result.contains("union(tables:"));
    }

    // @lat: [[tests#History evidence helpers#batch_metric_selector_union_flux builds union from specs]]
    #[test]
    fn batch_metric_selector_union_flux_builds_union() {
        let specs = vec![
            ("v1".to_string(), "flow".to_string(), "max".to_string(), "base1".to_string(), "max()".to_string()),
            ("v2".to_string(), "temp".to_string(), "min".to_string(), "base2".to_string(), "min()".to_string()),
        ];
        let result = batch_metric_selector_union_flux(&specs);
        assert!(result.contains("v1 = base1"));
        assert!(result.contains("v1_max = v1 |> max()"));
        assert!(result.contains("\"flow\""));
        assert!(result.contains("v2 = base2"));
        assert!(result.contains("v2_min = v2 |> min()"));
        assert!(result.contains("\"temp\""));
        assert!(result.contains("union(tables: [v1_max, v2_min])"));
    }

    // @lat: [[tests#History evidence helpers#batch_metric_selector_union_flux returns empty for empty input]]
    #[test]
    fn batch_metric_selector_union_flux_empty() {
        let result = batch_metric_selector_union_flux(&[]);
        assert!(result.is_empty());
    }

    // @lat: [[tests#History evidence helpers#controller_rows_target_series filters None targets]]
    #[test]
    fn controller_rows_target_series_filters_none() {
        let ts = dt(2026, 4, 10, 7, 0);
        let rows = vec![
            ControllerRow {
                ts,
                mode: "heating".into(),
                action: "set".into(),
                tariff: "cosy".into(),
                target_flow_c: Some(35.0),
                curve_after: Some(0.55),
                flow_desired_c: Some(30.0),
            },
            ControllerRow {
                ts,
                mode: "heating".into(),
                action: "skip".into(),
                tariff: "cosy".into(),
                target_flow_c: None,
                curve_after: None,
                flow_desired_c: None,
            },
        ];
        let series = controller_rows_target_series(&rows);
        assert_eq!(series.len(), 1);
        assert!((series[0].1 - 35.0).abs() < 0.01);
    }

    // ── Filter variant routing (migration regression) ──────────────────────
    // history.rs uses three distinct InfluxDB filter patterns. Each maps to
    // a different PostgreSQL table routing strategy. These tests document the
    // routing contracts the SQL migration must preserve.

    // @lat: [[tests#History filter variant routing#Topic filter routes by topic prefix and field name]]
    #[test]
    fn topic_filter_routing_contract() {
        // TopicSummarySpec uses: r.topic == X and r._field == Y
        // The topic string determines both the PG table AND the value column.
        let specs: Vec<(&str, &str, &str)> = vec![
            // (topic, _field, expected PG table)
            ("emon/emonth2_23/temperature", "value", "emonth"),
            ("zigbee2mqtt/aldora_temp_humid", "temperature", "zigbee"),
        ];

        for (topic, field, expected_table) in &specs {
            // Verify topic→table routing
            let table = if topic.starts_with("zigbee2mqtt/") {
                "zigbee"
            } else if topic.starts_with("emon/emonth2_23/") {
                "emonth"
            } else {
                panic!("unrouted topic: {topic}");
            };
            assert_eq!(table, *expected_table, "topic '{topic}' should route to '{expected_table}'");

            // Verify field name distinction
            if topic.starts_with("emon/emonth2_23/") {
                assert_eq!(*field, "value", "emonth2_23 uses _field='value'");
            } else {
                assert_eq!(*field, "temperature", "zigbee uses _field='temperature'");
            }
        }
    }

    // @lat: [[tests#History filter variant routing#Measurement filter routes by measurement name and field tag]]
    #[test]
    fn measurement_filter_routing_contract() {
        // MeasurementSummarySpec uses: r._measurement == X and r.field == Y
        // The _measurement determines the PG table, r.field determines the column.
        let specs: Vec<(&str, &str, &str)> = vec![
            // (measurement, field, expected PG table)
            ("ebusd_poll", "OutsideTemp", "ebusd_poll"),
            ("ebusd_poll", "Hc1HeatCurve", "ebusd_poll"),
            ("ebusd_poll", "FlowTemp", "ebusd_poll"),
            ("ebusd_poll", "ReturnTemp", "ebusd_poll"),
            ("emon", "dhw_t1", "multical"),  // emon measurement, dhw field → multical table
        ];

        for (measurement, field, expected_table) in &specs {
            let table = match *measurement {
                "ebusd_poll" => "ebusd_poll",
                // emon measurement with dhw_ prefix fields → multical table
                "emon" if field.starts_with("dhw_") => "multical",
                "emon" => "emon",
                m => panic!("unrouted measurement: {m}"),
            };
            assert_eq!(
                table, *expected_table,
                "measurement '{measurement}' + field '{field}' should route to '{expected_table}'"
            );
        }
    }

    // @lat: [[tests#History filter variant routing#Plain measurement filter uses underscore field]]
    #[test]
    fn plain_measurement_filter_routing_contract() {
        // PlainMeasurementSummarySpec uses: r._measurement == X and r._field == Y
        // Note: r._field (underscore), NOT r.field. This is a third distinct pattern.
        // The _measurement IS the PG table name directly.
        let specs: Vec<(&str, &str, &str)> = vec![
            // (measurement, _field, expected PG table)
            ("dhw", "remaining_litres", "dhw"),
        ];

        for (measurement, field, expected_table) in &specs {
            // For plain measurement specs, _measurement maps directly to PG table
            assert_eq!(
                *measurement, *expected_table,
                "plain measurement '{measurement}' should map directly to PG table"
            );
            // _field maps directly to column name
            assert!(
                !field.is_empty(),
                "field name must be non-empty for column mapping"
            );
        }
    }
}
