use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use super::artifact::{build_artifact, write_artifact};
use super::calibration::{
    calibrate_model, measured_rates, parse_validation_windows, predict_rates,
};
use super::config::{resolve_influx_token, resolve_postgres_conninfo};
use super::error::{ThermalError, ThermalResult};
use super::geometry::build_doorways;
use super::influx;
use super::physics::{compute_thermal_masses, doors_all_closed_except_chimney};
use super::report;
use super::wind::{fetch_open_meteo_wind, wind_multiplier_for_window};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct RoomResidual {
    pub room: String,
    pub measured: f64,
    pub predicted: f64,
    pub residual: f64,
    pub abs_residual: f64,
    pub thermal_mass_kj_per_k: f64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct RoomHeatError {
    pub room: String,
    pub measured_w: f64,
    pub predicted_w: f64,
    pub error_w: f64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct WholeHouseMetrics {
    pub measured_w: f64,
    pub predicted_w: f64,
    pub error_w: f64,
    pub pred_over_meas: f64,
    pub top_contributors: Vec<RoomHeatError>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Metrics {
    pub rooms_count: usize,
    pub rmse: f64,
    pub mae: f64,
    pub bias: f64,
    pub max_abs_error: f64,
    pub within_0_5c: f64,
    pub within_1_0c: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct WindowValidation {
    pub name: String,
    pub start: String,
    pub end: String,
    pub door_state: String,
    pub outside_avg_c: f64,
    pub wind_avg_ms: f64,
    pub wind_multiplier: f64,
    pub metrics: Metrics,
    pub whole_house: WholeHouseMetrics,
    pub pass: bool,
    pub residuals: Vec<RoomResidual>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ThresholdResult {
    pub rmse_max: f64,
    pub bias_abs_max: f64,
    pub within_1c_min: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidationSummary {
    pub thresholds: ThresholdResult,
    pub aggregate_metrics: Metrics,
    pub aggregate_whole_house: WholeHouseMetrics,
    pub aggregate_pass: bool,
    pub windows: Vec<WindowValidation>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn validate(config_path: &Path) -> ThermalResult<()> {
    let m = calibrate_model(config_path)?;
    let (cfg_txt, cfg, setup, result, rooms) = (&m.cfg_txt, &m.cfg, &m.setup, &m.result, &m.rooms);

    if cfg.validation.windows.is_empty() {
        return Err(ThermalError::NoValidationWindows);
    }

    let parsed_windows = parse_validation_windows(&cfg.validation.windows)?;
    let earliest_val = parsed_windows.iter().map(|w| w.start).min().unwrap();
    let latest_val = parsed_windows.iter().map(|w| w.end).max().unwrap();

    let wind_points = if cfg.wind.enabled {
        fetch_open_meteo_wind(
            cfg.wind.latitude,
            cfg.wind.longitude,
            earliest_val,
            latest_val,
        )
    } else {
        Vec::new()
    };

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = resolve_influx_token(cfg)?;
    let pg_conninfo = resolve_postgres_conninfo(cfg)?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &sensor_topics,
        &earliest_val,
        &latest_val,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &earliest_val,
        &latest_val,
    )?;

    let room_series = super::calibration::build_room_series(&room_rows, rooms)?;
    let doors_normal = build_doorways()?;
    let doors_closed = doors_all_closed_except_chimney(&doors_normal);

    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();
    let mut window_results = Vec::new();

    let thermal_masses = compute_thermal_masses(rooms, &setup.connections);

    println!("Config: {}", config_path.display());
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );

    for window in parsed_windows {
        let (wind_mult, wind_avg) =
            wind_multiplier_for_window(&cfg.wind, &wind_points, window.start, window.end);
        let (measured, avg_temps, outside_avg) =
            measured_rates(&room_series, &outside_rows, window.start, window.end)?;
        let doorways = match window.door_state.as_str() {
            "closed_except_chimney" | "all_closed_except_chimney" | "closed" => &doors_closed,
            _ => &doors_normal,
        };
        let predicted = predict_rates(
            rooms,
            &setup.connections,
            doorways,
            &avg_temps,
            outside_avg,
            result.doorway_cd,
            wind_mult,
        );

        report::print_table(
            &format!("Validation {}", window.name),
            &measured,
            &predicted,
        );
        let residuals =
            residuals_for_rooms(&measured, &predicted, Some(&exclude_rooms), &thermal_masses);
        let metrics = compute_metrics(&residuals);
        let wh = whole_house_metrics(&residuals, 5);
        let pass = metrics.rmse <= cfg.validation.thresholds.rmse_max
            && metrics.bias.abs() <= cfg.validation.thresholds.bias_abs_max
            && metrics.within_1_0c >= cfg.validation.thresholds.within_1c_min;

        println!(
            "  {}: rmse={:.3}, mae={:.3}, bias={:+.3}, within_1C={:.1}% => {}",
            window.name,
            metrics.rmse,
            metrics.mae,
            metrics.bias,
            metrics.within_1_0c * 100.0,
            if pass { "PASS" } else { "FAIL" }
        );
        println!(
            "  whole-house: meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
            wh.measured_w, wh.predicted_w, wh.error_w, wh.pred_over_meas
        );
        println!("  top error contributors:");
        for c in &wh.top_contributors {
            println!(
                "    {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
                c.room, c.measured_w, c.predicted_w, c.error_w
            );
        }

        window_results.push(WindowValidation {
            name: window.name,
            start: window.start.to_rfc3339(),
            end: window.end.to_rfc3339(),
            door_state: window.door_state,
            outside_avg_c: outside_avg,
            wind_avg_ms: wind_avg,
            wind_multiplier: wind_mult,
            metrics,
            whole_house: wh,
            pass,
            residuals,
        });
    }

    let aggregate_metrics = aggregate_metrics_from_windows(&window_results);
    let aggregate_whole_house = aggregate_whole_house_from_windows(&window_results, 5);

    let aggregate_pass = aggregate_metrics.rmse <= cfg.validation.thresholds.rmse_max
        && aggregate_metrics.bias.abs() <= cfg.validation.thresholds.bias_abs_max
        && aggregate_metrics.within_1_0c >= cfg.validation.thresholds.within_1c_min;

    println!("\nValidation aggregate:");
    println!(
        "  rmse={:.3}, mae={:.3}, bias={:+.3}, within_1C={:.1}% => {}",
        aggregate_metrics.rmse,
        aggregate_metrics.mae,
        aggregate_metrics.bias,
        aggregate_metrics.within_1_0c * 100.0,
        if aggregate_pass { "PASS" } else { "FAIL" }
    );
    println!(
        "  whole-house: meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
        aggregate_whole_house.measured_w,
        aggregate_whole_house.predicted_w,
        aggregate_whole_house.error_w,
        aggregate_whole_house.pred_over_meas
    );
    println!("  top aggregate error contributors:");
    for c in &aggregate_whole_house.top_contributors {
        println!(
            "    {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
            c.room, c.measured_w, c.predicted_w, c.error_w
        );
    }

    let validation = ValidationSummary {
        thresholds: ThresholdResult {
            rmse_max: cfg.validation.thresholds.rmse_max,
            bias_abs_max: cfg.validation.thresholds.bias_abs_max,
            within_1c_min: cfg.validation.thresholds.within_1c_min,
        },
        aggregate_metrics,
        aggregate_whole_house,
        aggregate_pass,
        windows: window_results,
    };

    let artifact = build_artifact(
        "thermal-validate",
        config_path,
        cfg_txt,
        cfg,
        setup,
        result,
        Some(validation),
    )?;
    let artifact_path = write_artifact("thermal-validate", &artifact)?;
    println!("\nWrote validation artifact: {}", artifact_path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Metric computation
// ---------------------------------------------------------------------------

pub(crate) fn residuals_for_rooms(
    measured: &HashMap<String, f64>,
    predicted: &HashMap<String, f64>,
    exclude: Option<&HashSet<String>>,
    thermal_masses: &HashMap<String, f64>,
) -> Vec<RoomResidual> {
    let mut out = Vec::new();
    let mut keys: Vec<_> = measured.keys().cloned().collect();
    keys.sort();

    for room in keys {
        if exclude.is_some_and(|x| x.contains(&room)) {
            continue;
        }
        let measured_v = measured.get(&room).copied().unwrap_or(0.0);
        let predicted_v = predicted.get(&room).copied().unwrap_or(f64::NAN);
        if predicted_v.is_nan() {
            continue;
        }
        let residual = predicted_v - measured_v;
        let c_kj = thermal_masses.get(&room).copied().unwrap_or(0.0);
        out.push(RoomResidual {
            room,
            measured: measured_v,
            predicted: predicted_v,
            residual,
            abs_residual: residual.abs(),
            thermal_mass_kj_per_k: c_kj,
        });
    }

    out
}

pub(crate) fn whole_house_metrics(residuals: &[RoomResidual], top_n: usize) -> WholeHouseMetrics {
    let mut entries: Vec<RoomHeatError> = residuals
        .iter()
        .map(|r| {
            let meas_w = r.measured * r.thermal_mass_kj_per_k / 3.6;
            let pred_w = r.predicted * r.thermal_mass_kj_per_k / 3.6;
            RoomHeatError {
                room: r.room.clone(),
                measured_w: meas_w,
                predicted_w: pred_w,
                error_w: pred_w - meas_w,
            }
        })
        .collect();

    let measured_w: f64 = entries.iter().map(|e| e.measured_w).sum();
    let predicted_w: f64 = entries.iter().map(|e| e.predicted_w).sum();
    let error_w = predicted_w - measured_w;
    let pred_over_meas = if measured_w.abs() > 1e-9 {
        predicted_w / measured_w
    } else {
        f64::NAN
    };

    entries.sort_by(|a, b| b.error_w.abs().total_cmp(&a.error_w.abs()));
    let top_contributors: Vec<RoomHeatError> = entries.into_iter().take(top_n).collect();

    WholeHouseMetrics {
        measured_w,
        predicted_w,
        error_w,
        pred_over_meas,
        top_contributors,
    }
}

pub(crate) fn compute_metrics(residuals: &[RoomResidual]) -> Metrics {
    let values: Vec<f64> = residuals.iter().map(|r| r.residual).collect();
    compute_metrics_from_values(&values)
}

pub(crate) fn aggregate_metrics_from_windows(windows: &[WindowValidation]) -> Metrics {
    let mut values = Vec::new();
    for window in windows {
        values.extend(window.residuals.iter().map(|r| r.residual));
    }
    compute_metrics_from_values(&values)
}

pub(crate) fn aggregate_whole_house_from_windows(
    windows: &[WindowValidation],
    top_n: usize,
) -> WholeHouseMetrics {
    let measured_w: f64 = windows.iter().map(|w| w.whole_house.measured_w).sum();
    let predicted_w: f64 = windows.iter().map(|w| w.whole_house.predicted_w).sum();
    let error_w = predicted_w - measured_w;
    let pred_over_meas = if measured_w.abs() > 1e-9 {
        predicted_w / measured_w
    } else {
        f64::NAN
    };

    let mut room_totals: HashMap<String, (f64, f64)> = HashMap::new();
    for window in windows {
        for residual in &window.residuals {
            let measured = residual.measured * residual.thermal_mass_kj_per_k / 3.6;
            let predicted = residual.predicted * residual.thermal_mass_kj_per_k / 3.6;
            let totals = room_totals
                .entry(residual.room.clone())
                .or_insert((0.0, 0.0));
            totals.0 += measured;
            totals.1 += predicted;
        }
    }

    let mut top_contributors: Vec<RoomHeatError> = room_totals
        .into_iter()
        .map(|(room, (measured_w, predicted_w))| RoomHeatError {
            room,
            measured_w,
            predicted_w,
            error_w: predicted_w - measured_w,
        })
        .collect();
    top_contributors.sort_by(|a, b| b.error_w.abs().total_cmp(&a.error_w.abs()));
    top_contributors.truncate(top_n);

    WholeHouseMetrics {
        measured_w,
        predicted_w,
        error_w,
        pred_over_meas,
        top_contributors,
    }
}

pub(crate) fn compute_metrics_from_values(values: &[f64]) -> Metrics {
    if values.is_empty() {
        return Metrics {
            rooms_count: 0,
            rmse: 999.0,
            mae: 999.0,
            bias: 0.0,
            max_abs_error: 999.0,
            within_0_5c: 0.0,
            within_1_0c: 0.0,
        };
    }

    let n = values.len() as f64;
    let sq = values.iter().map(|v| v * v).sum::<f64>();
    let abs_sum = values.iter().map(|v| v.abs()).sum::<f64>();
    let bias = values.iter().sum::<f64>() / n;
    let max_abs = values.iter().map(|v| v.abs()).fold(0.0, f64::max);
    let within_05 = values.iter().filter(|v| v.abs() <= 0.5).count() as f64 / n;
    let within_10 = values.iter().filter(|v| v.abs() <= 1.0).count() as f64 / n;

    Metrics {
        rooms_count: values.len(),
        rmse: (sq / n).sqrt(),
        mae: abs_sum / n,
        bias,
        max_abs_error: max_abs,
        within_0_5c: within_05,
        within_1_0c: within_10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn residual(room: &str, measured: f64, predicted: f64, mass: f64) -> RoomResidual {
        RoomResidual {
            room: room.to_string(),
            measured,
            predicted,
            residual: predicted - measured,
            abs_residual: (predicted - measured).abs(),
            thermal_mass_kj_per_k: mass,
        }
    }

    // @lat: [[tests#Thermal validation helpers#Residual aggregation skips excluded and missing predictions]]
    #[test]
    fn residuals_skip_exclusions_and_rooms_without_predictions() {
        let measured = HashMap::from([
            ("kept".to_string(), 1.0),
            ("excluded".to_string(), 2.0),
            ("missing_pred".to_string(), 3.0),
        ]);
        let predicted = HashMap::from([("kept".to_string(), 1.5), ("excluded".to_string(), 1.0)]);
        let masses = HashMap::from([("kept".to_string(), 10.0), ("excluded".to_string(), 20.0)]);
        let exclude = HashSet::from(["excluded".to_string()]);

        let residuals = residuals_for_rooms(&measured, &predicted, Some(&exclude), &masses);

        assert_eq!(residuals.len(), 1);
        assert_eq!(residuals[0].room, "kept");
        assert!((residuals[0].residual - 0.5).abs() < 1e-9);
        assert!((residuals[0].thermal_mass_kj_per_k - 10.0).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal validation helpers#Whole-house metrics weight errors by thermal mass]]
    #[test]
    fn whole_house_metrics_convert_rates_to_weighted_watts_and_sort_contributors() {
        let residuals = vec![
            residual("heavy", 1.0, 2.0, 36.0),
            residual("light", 1.0, 1.5, 7.2),
            residual("negative", 2.0, 1.5, 14.4),
        ];

        let metrics = whole_house_metrics(&residuals, 2);

        assert!((metrics.measured_w - 20.0).abs() < 1e-9);
        assert!((metrics.predicted_w - 29.0).abs() < 1e-9);
        assert!((metrics.error_w - 9.0).abs() < 1e-9);
        assert!((metrics.pred_over_meas - 1.45).abs() < 1e-9);
        assert_eq!(metrics.top_contributors.len(), 2);
        assert_eq!(metrics.top_contributors[0].room, "heavy");
        assert_eq!(metrics.top_contributors[1].room, "negative");
    }

    // @lat: [[tests#Thermal validation helpers#Metrics summaries handle empty inputs and tolerance buckets]]
    #[test]
    fn metrics_summary_handles_empty_inputs_and_bucket_thresholds() {
        let empty = compute_metrics_from_values(&[]);
        assert_eq!(empty.rooms_count, 0);
        assert_eq!(empty.rmse, 999.0);
        assert_eq!(empty.mae, 999.0);
        assert_eq!(empty.max_abs_error, 999.0);
        assert_eq!(empty.within_0_5c, 0.0);
        assert_eq!(empty.within_1_0c, 0.0);

        let metrics = compute_metrics_from_values(&[-1.0, -0.5, 0.25, 1.0]);
        assert_eq!(metrics.rooms_count, 4);
        assert!((metrics.rmse - 0.7603453162872774).abs() < 1e-9);
        assert!((metrics.mae - 0.6875).abs() < 1e-9);
        assert!((metrics.bias + 0.0625).abs() < 1e-9);
        assert!((metrics.max_abs_error - 1.0).abs() < 1e-9);
        assert!((metrics.within_0_5c - 0.5).abs() < 1e-9);
        assert!((metrics.within_1_0c - 1.0).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal validation helpers#Whole-house ratio stays undefined when measured load cancels out]]
    #[test]
    fn whole_house_ratio_stays_undefined_when_measured_load_cancels_out() {
        let residuals = vec![
            residual("gain", 1.0, 1.5, 36.0),
            residual("loss", -1.0, -0.5, 36.0),
        ];

        let metrics = whole_house_metrics(&residuals, 5);

        assert!(metrics.measured_w.abs() < 1e-9);
        assert!((metrics.predicted_w - 10.0).abs() < 1e-9);
        assert!((metrics.error_w - 10.0).abs() < 1e-9);
        assert!(metrics.pred_over_meas.is_nan());
        assert_eq!(metrics.top_contributors.len(), 2);
    }

    // @lat: [[tests#Thermal validation helpers#Residuals without thermal mass stay rate-only]]
    #[test]
    fn residuals_without_thermal_mass_stay_rate_only() {
        let measured = HashMap::from([("kept".to_string(), 1.0)]);
        let predicted = HashMap::from([("kept".to_string(), 1.5)]);
        let masses = HashMap::new();

        let residuals = residuals_for_rooms(&measured, &predicted, None, &masses);
        assert_eq!(residuals.len(), 1);
        assert_eq!(residuals[0].room, "kept");
        assert_eq!(residuals[0].thermal_mass_kj_per_k, 0.0);

        let whole_house = whole_house_metrics(&residuals, 5);
        assert_eq!(whole_house.measured_w, 0.0);
        assert_eq!(whole_house.predicted_w, 0.0);
        assert_eq!(whole_house.error_w, 0.0);
        assert!(whole_house.pred_over_meas.is_nan());
    }

    // @lat: [[tests#Thermal validation helpers#Aggregate metrics flatten residuals across windows]]
    #[test]
    fn aggregate_metrics_flattens_all_window_residuals() {
        let windows = vec![
            WindowValidation {
                name: "w1".to_string(),
                start: "2026-04-10T00:00:00+00:00".to_string(),
                end: "2026-04-10T01:00:00+00:00".to_string(),
                door_state: "normal".to_string(),
                outside_avg_c: 5.0,
                wind_avg_ms: 0.0,
                wind_multiplier: 1.0,
                metrics: compute_metrics_from_values(&[999.0]),
                whole_house: WholeHouseMetrics {
                    measured_w: 0.0,
                    predicted_w: 0.0,
                    error_w: 0.0,
                    pred_over_meas: f64::NAN,
                    top_contributors: vec![],
                },
                pass: false,
                residuals: vec![residual("a", 1.0, 1.5, 36.0), residual("b", 2.0, 1.0, 36.0)],
            },
            WindowValidation {
                name: "w2".to_string(),
                start: "2026-04-11T00:00:00+00:00".to_string(),
                end: "2026-04-11T01:00:00+00:00".to_string(),
                door_state: "closed".to_string(),
                outside_avg_c: 4.0,
                wind_avg_ms: 1.0,
                wind_multiplier: 1.1,
                metrics: compute_metrics_from_values(&[999.0]),
                whole_house: WholeHouseMetrics {
                    measured_w: 0.0,
                    predicted_w: 0.0,
                    error_w: 0.0,
                    pred_over_meas: f64::NAN,
                    top_contributors: vec![],
                },
                pass: false,
                residuals: vec![residual("c", -1.0, -0.25, 36.0)],
            },
        ];

        let metrics = aggregate_metrics_from_windows(&windows);
        let direct = compute_metrics_from_values(&[0.5, -1.0, 0.75]);

        assert_eq!(metrics.rooms_count, 3);
        assert!((metrics.rmse - direct.rmse).abs() < 1e-9);
        assert!((metrics.mae - direct.mae).abs() < 1e-9);
        assert!((metrics.bias - direct.bias).abs() < 1e-9);
        assert!((metrics.max_abs_error - direct.max_abs_error).abs() < 1e-9);
        assert!((metrics.within_0_5c - direct.within_0_5c).abs() < 1e-9);
        assert!((metrics.within_1_0c - direct.within_1_0c).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal validation helpers#Aggregate whole-house contributors merge repeated rooms across windows]]
    #[test]
    fn aggregate_whole_house_merges_repeated_rooms_across_windows() {
        let windows = vec![
            WindowValidation {
                name: "w1".to_string(),
                start: "2026-04-10T00:00:00+00:00".to_string(),
                end: "2026-04-10T01:00:00+00:00".to_string(),
                door_state: "normal".to_string(),
                outside_avg_c: 5.0,
                wind_avg_ms: 0.0,
                wind_multiplier: 1.0,
                metrics: compute_metrics_from_values(&[0.5]),
                whole_house: WholeHouseMetrics {
                    measured_w: 10.0,
                    predicted_w: 13.0,
                    error_w: 3.0,
                    pred_over_meas: 1.3,
                    top_contributors: vec![],
                },
                pass: true,
                residuals: vec![
                    residual("shared", 1.0, 2.0, 36.0),
                    residual("other", 1.0, 1.25, 36.0),
                ],
            },
            WindowValidation {
                name: "w2".to_string(),
                start: "2026-04-11T00:00:00+00:00".to_string(),
                end: "2026-04-11T01:00:00+00:00".to_string(),
                door_state: "closed".to_string(),
                outside_avg_c: 4.0,
                wind_avg_ms: 1.0,
                wind_multiplier: 1.1,
                metrics: compute_metrics_from_values(&[1.0]),
                whole_house: WholeHouseMetrics {
                    measured_w: 12.0,
                    predicted_w: 18.0,
                    error_w: 6.0,
                    pred_over_meas: 1.5,
                    top_contributors: vec![],
                },
                pass: false,
                residuals: vec![
                    residual("shared", 2.0, 3.0, 36.0),
                    residual("small", 1.0, 1.1, 36.0),
                ],
            },
        ];

        let aggregate = aggregate_whole_house_from_windows(&windows, 2);

        assert!((aggregate.measured_w - 22.0).abs() < 1e-9);
        assert!((aggregate.predicted_w - 31.0).abs() < 1e-9);
        assert!((aggregate.error_w - 9.0).abs() < 1e-9);
        assert!((aggregate.pred_over_meas - (31.0 / 22.0)).abs() < 1e-9);
        assert_eq!(aggregate.top_contributors.len(), 2);
        assert_eq!(aggregate.top_contributors[0].room, "shared");
        assert!((aggregate.top_contributors[0].measured_w - 30.0).abs() < 1e-9);
        assert!((aggregate.top_contributors[0].predicted_w - 50.0).abs() < 1e-9);
        assert!((aggregate.top_contributors[0].error_w - 20.0).abs() < 1e-9);
        assert_eq!(aggregate.top_contributors[1].room, "other");
    }

    proptest! {
        // @lat: [[tests#Thermal validation helpers#Metrics magnitudes are symmetric under sign inversion]]
        #[test]
        fn metrics_magnitudes_are_symmetric_under_sign_inversion(
            values in proptest::collection::vec(-20.0f64..20.0, 1..32)
        ) {
            let inverted: Vec<f64> = values.iter().map(|v| -*v).collect();

            let original = compute_metrics_from_values(&values);
            let mirrored = compute_metrics_from_values(&inverted);

            prop_assert_eq!(original.rooms_count, mirrored.rooms_count);
            prop_assert!((original.rmse - mirrored.rmse).abs() < 1e-9);
            prop_assert!((original.mae - mirrored.mae).abs() < 1e-9);
            prop_assert!((original.max_abs_error - mirrored.max_abs_error).abs() < 1e-9);
            prop_assert!((original.within_0_5c - mirrored.within_0_5c).abs() < 1e-9);
            prop_assert!((original.within_1_0c - mirrored.within_1_0c).abs() < 1e-9);
            prop_assert!((original.bias + mirrored.bias).abs() < 1e-9);
        }
    }
}
