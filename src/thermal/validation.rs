use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use super::artifact::{build_artifact, write_artifact};
use super::calibration::{
    calibrate_model, measured_rates, parse_validation_windows, predict_rates,
};
use super::config::resolve_influx_token;
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

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &earliest_val,
        &latest_val,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
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

    let mut all_residuals = Vec::new();
    for w in &window_results {
        all_residuals.extend(w.residuals.iter().map(|r| r.residual));
    }
    let aggregate_metrics = compute_metrics_from_values(&all_residuals);

    let agg_measured_w: f64 = window_results
        .iter()
        .map(|w| w.whole_house.measured_w)
        .sum();
    let agg_predicted_w: f64 = window_results
        .iter()
        .map(|w| w.whole_house.predicted_w)
        .sum();
    let agg_error_w = agg_predicted_w - agg_measured_w;
    let agg_pred_over_meas = if agg_measured_w.abs() > 1e-9 {
        agg_predicted_w / agg_measured_w
    } else {
        f64::NAN
    };

    let mut agg_room_errors: HashMap<String, (f64, f64)> = HashMap::new();
    for w in &window_results {
        for r in &w.residuals {
            let meas_w = r.measured * r.thermal_mass_kj_per_k / 3.6;
            let pred_w = r.predicted * r.thermal_mass_kj_per_k / 3.6;
            let entry = agg_room_errors.entry(r.room.clone()).or_insert((0.0, 0.0));
            entry.0 += meas_w;
            entry.1 += pred_w;
        }
    }
    let mut agg_contributors: Vec<RoomHeatError> = agg_room_errors
        .into_iter()
        .map(|(room, (m, p))| RoomHeatError {
            room,
            measured_w: m,
            predicted_w: p,
            error_w: p - m,
        })
        .collect();
    agg_contributors.sort_by(|a, b| b.error_w.abs().total_cmp(&a.error_w.abs()));
    let agg_top: Vec<RoomHeatError> = agg_contributors.into_iter().take(5).collect();

    let aggregate_whole_house = WholeHouseMetrics {
        measured_w: agg_measured_w,
        predicted_w: agg_predicted_w,
        error_w: agg_error_w,
        pred_over_meas: agg_pred_over_meas,
        top_contributors: agg_top.clone(),
    };

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
        agg_measured_w, agg_predicted_w, agg_error_w, agg_pred_over_meas
    );
    println!("  top aggregate error contributors:");
    for c in &agg_top {
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
