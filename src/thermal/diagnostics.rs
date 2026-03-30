use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;

use super::artifact::{config_sha256, git_meta, ArtifactCalibrationParams, FitDiagnosticsArtifact};
use super::calibration::{
    avg_room_temps_in_window, avg_series_in_window, build_room_series, calibrate_model,
};
use super::config::resolve_influx_token;
use super::error::{ThermalError, ThermalResult};
use super::geometry::build_doorways;
use super::influx;
use super::physics::{
    doors_all_closed_except_chimney, estimate_thermal_mass, room_energy_balance,
    BODY_HEAT_SLEEPING_W,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct FitPeriod {
    pub start: String,
    pub end: String,
    pub hours: f64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct FitRecord {
    pub room: String,
    pub period_start: String,
    pub period_end: String,
    pub start_temp_c: f64,
    pub end_temp_c: f64,
    pub meas_rate_c_per_hr: f64,
    pub pred_rate_c_per_hr: f64,
    pub ratio_pred_over_meas: Option<f64>,
    pub body_w: f64,
    pub true_cooling: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct FitSummary {
    pub n: usize,
    pub rmse: f64,
    pub mae: f64,
    pub med_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PerRoomFitSummary {
    pub room: String,
    pub n: usize,
    pub rmse: f64,
    pub mae: f64,
    pub med_ratio: Option<f64>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn fit_diagnostics(config_path: &Path) -> ThermalResult<()> {
    let m = calibrate_model(config_path)?;
    let (cfg_txt, cfg, setup, result, rooms) = (&m.cfg_txt, &m.cfg, &m.setup, &m.result, &m.rooms);

    let fit_cfg = &cfg.fit_diagnostics;
    let range_start = fit_cfg
        .start
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| setup.night1_start - chrono::Duration::hours(24));
    let range_end = fit_cfg
        .end
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| Utc::now().fixed_offset());

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = resolve_influx_token(cfg)?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &range_start,
        &range_end,
    )?;
    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;
    let status_rows = influx::query_status_codes(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;

    if status_rows.is_empty() {
        return Err(ThermalError::NoStatusData);
    }

    let room_series = build_room_series(&room_rows, rooms)?;
    let cooldown_periods = detect_cooldown_periods(
        &status_rows,
        &fit_cfg.heating_off_codes,
        fit_cfg.min_period_hours,
    );
    if cooldown_periods.is_empty() {
        return Err(ThermalError::NoCooldownPeriods);
    }

    println!("Config: {}", config_path.display());
    println!(
        "Diagnostics range: {} -> {}",
        range_start.to_rfc3339(),
        range_end.to_rfc3339()
    );
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );

    println!("Found {} cooldown periods:", cooldown_periods.len());
    for (start, end) in &cooldown_periods {
        let hours = (*end - *start).num_seconds() as f64 / 3600.0;
        println!(
            "  {} -> {} ({:.1}h)",
            start.format("%H:%M"),
            end.format("%H:%M"),
            hours
        );
    }

    println!(
        "\n{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>5} {:>16}",
        "Room", "Start", "End", "Meas", "Pred", "Ratio", "Body", "Period"
    );
    println!(
        "{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>5}",
        "", "°C", "°C", "°C/hr", "°C/hr", "P/M", "W"
    );
    println!("{}", "─".repeat(92));

    let doorways_base = build_doorways()?;
    let doorways_closed = doors_all_closed_except_chimney(&doorways_base);
    let doorways = match fit_cfg.door_state.as_str() {
        "closed_except_chimney" | "all_closed_except_chimney" | "closed" => &doorways_closed,
        _ => &doorways_base,
    };

    let mut records = Vec::new();
    for (period_start, period_end) in &cooldown_periods {
        let avg_outside = avg_series_in_window(&outside_rows, *period_start, *period_end, 8.0);
        let avg_temps = avg_room_temps_in_window(&room_series, *period_start, *period_end);

        for (room_name, series) in &room_series {
            let temps_in_period: Vec<(DateTime<FixedOffset>, f64)> = series
                .iter()
                .cloned()
                .filter(|(t, _)| *t >= *period_start && *t <= *period_end)
                .collect();
            if temps_in_period.len() < 2 {
                continue;
            }
            let (first, last) = match (temps_in_period.first(), temps_in_period.last()) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let hours = (last.0 - first.0).num_seconds() as f64 / 3600.0;
            if hours < fit_cfg.min_record_hours {
                continue;
            }

            let meas_rate = (first.1 - last.1) / hours;
            let avg_t = avg_temps
                .get(room_name)
                .copied()
                .unwrap_or((first.1 + last.1) / 2.0);
            let Some(room) = rooms.get(room_name) else {
                continue;
            };
            let c = estimate_thermal_mass(room, &setup.connections);
            let bal = room_energy_balance(
                room,
                avg_t,
                avg_outside,
                &avg_temps,
                &setup.connections,
                doorways,
                result.doorway_cd,
                1.0,
            );
            let pred_rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };
            let body_w = room.overnight_occupants as f64 * BODY_HEAT_SLEEPING_W;
            let ratio = if meas_rate.abs() > fit_cfg.ratio_min_meas {
                Some(pred_rate / meas_rate)
            } else {
                None
            };
            let true_cooling = meas_rate >= fit_cfg.min_meas_cooling;
            let marker = if true_cooling { "" } else { "*" };
            let period_str = format!(
                "{}→{}{}",
                period_start.format("%H:%M"),
                period_end.format("%H:%M"),
                marker
            );
            let ratio_str = ratio.map_or("   nan".to_string(), |v| format!("{:>6.2}", v));

            println!(
                "{:<14} {:>7.2} {:>7.2} {:>7.3} {:>7.3} {} {:>5.0} {:>16}",
                room_name, first.1, last.1, meas_rate, pred_rate, ratio_str, body_w, period_str
            );

            records.push(FitRecord {
                room: room_name.clone(),
                period_start: period_start.to_rfc3339(),
                period_end: period_end.to_rfc3339(),
                start_temp_c: first.1,
                end_temp_c: last.1,
                meas_rate_c_per_hr: meas_rate,
                pred_rate_c_per_hr: pred_rate,
                ratio_pred_over_meas: ratio,
                body_w,
                true_cooling,
            });
        }
    }

    println!("\n* marks weak/non-cooling measured periods (meas_rate < 0.03 °C/hr)\n");

    let summary_all = summarize_fit_records(&records);
    let good: Vec<FitRecord> = records.iter().filter(|r| r.true_cooling).cloned().collect();
    let summary_good = summarize_fit_records(&good);

    println!("Summary (all records):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  Median ratio={}",
        summary_all.n,
        summary_all.rmse,
        summary_all.mae,
        summary_all
            .med_ratio
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "nan".to_string())
    );

    println!("Summary (true cooling only):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  Median ratio={}",
        summary_good.n,
        summary_good.rmse,
        summary_good.mae,
        summary_good
            .med_ratio
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "nan".to_string())
    );

    let per_room = summarize_fit_by_room(&good);
    println!("\nPer-room summary (true cooling only):");
    println!(
        "{:<14} {:>4} {:>8} {:>8} {:>8}",
        "Room", "N", "RMSE", "MAE", "MedRat"
    );
    println!("{}", "─".repeat(46));
    for row in &per_room {
        println!(
            "{:<14} {:>4} {:>8.3} {:>8.3} {:>8}",
            row.room,
            row.n,
            row.rmse,
            row.mae,
            row.med_ratio
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "nan".to_string())
        );
    }

    let periods_json: Vec<FitPeriod> = cooldown_periods
        .iter()
        .map(|(s, e)| FitPeriod {
            start: s.to_rfc3339(),
            end: e.to_rfc3339(),
            hours: (*e - *s).num_seconds() as f64 / 3600.0,
        })
        .collect();

    let artifact = FitDiagnosticsArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-fit-diagnostics".to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(cfg_txt),
        git: git_meta(),
        range_start: range_start.to_rfc3339(),
        range_end: range_end.to_rfc3339(),
        door_state: fit_cfg.door_state.clone(),
        cooldown_periods: periods_json,
        records: records.clone(),
        summary_all,
        summary_true_cooling: summary_good,
        per_room_true_cooling: per_room,
        calibrated_params: ArtifactCalibrationParams {
            leather_ach: result.leather_ach,
            landing_ach: result.landing_ach,
            conservatory_ach: result.conservatory_ach,
            office_ach: result.office_ach,
            doorway_cd: result.doorway_cd,
        },
    };

    let artifact_path = super::artifact::write_fit_artifact("thermal-fit-diagnostics", &artifact)?;
    println!(
        "\nWrote fit diagnostics artifact: {}",
        artifact_path.display()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Cooldown detection and summarization
// ---------------------------------------------------------------------------

pub(crate) fn detect_cooldown_periods(
    status_rows: &[(DateTime<FixedOffset>, i32)],
    heating_off_codes: &[i32],
    min_period_hours: f64,
) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    let off: HashSet<i32> = heating_off_codes.iter().copied().collect();
    let mut periods = Vec::new();
    let mut in_cooldown = false;
    let mut start: Option<DateTime<FixedOffset>> = None;

    for (t, code) in status_rows {
        if off.contains(code) {
            if !in_cooldown {
                start = Some(*t);
                in_cooldown = true;
            }
        } else if in_cooldown {
            if let Some(s) = start {
                let h = (*t - s).num_seconds() as f64 / 3600.0;
                if h > min_period_hours {
                    periods.push((s, *t));
                }
            }
            in_cooldown = false;
            start = None;
        }
    }

    if in_cooldown {
        if let (Some(s), Some((end, _))) = (start, status_rows.last()) {
            let h = (*end - s).num_seconds() as f64 / 3600.0;
            if h > min_period_hours {
                periods.push((s, *end));
            }
        }
    }

    periods
}

pub(crate) fn summarize_fit_records(records: &[FitRecord]) -> FitSummary {
    if records.is_empty() {
        return FitSummary {
            n: 0,
            rmse: f64::NAN,
            mae: f64::NAN,
            med_ratio: None,
        };
    }

    let errs: Vec<f64> = records
        .iter()
        .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
        .collect();
    let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len() as f64).sqrt();
    let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / errs.len() as f64;

    let mut ratios: Vec<f64> = records
        .iter()
        .filter_map(|r| r.ratio_pred_over_meas)
        .collect();
    ratios.sort_by(|a, b| a.total_cmp(b));
    let med_ratio = median(&ratios);

    FitSummary {
        n: records.len(),
        rmse,
        mae,
        med_ratio,
    }
}

pub(crate) fn summarize_fit_by_room(records: &[FitRecord]) -> Vec<PerRoomFitSummary> {
    let mut by_room: BTreeMap<String, Vec<FitRecord>> = BTreeMap::new();
    for r in records {
        by_room.entry(r.room.clone()).or_default().push(r.clone());
    }

    by_room
        .into_iter()
        .map(|(room, rows)| {
            let st = summarize_fit_records(&rows);
            PerRoomFitSummary {
                room,
                n: st.n,
                rmse: st.rmse,
                mae: st.mae,
                med_ratio: st.med_ratio,
            }
        })
        .collect()
}

pub(crate) fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let n = values.len();
    if n % 2 == 1 {
        Some(values[n / 2])
    } else {
        Some((values[n / 2 - 1] + values[n / 2]) / 2.0)
    }
}
