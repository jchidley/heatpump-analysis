use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use chrono::{DateTime, FixedOffset, Timelike, Utc};
use serde::Serialize;

use super::artifact::{config_sha256, git_meta, ArtifactCalibrationParams, GitMeta};
use super::calibration::{
    avg_room_temps_in_window, avg_series_in_window, build_room_series, calibrate_model,
};
use super::config::{resolve_influx_token, resolve_postgres_conninfo};
use super::error::{ThermalError, ThermalResult};
use super::geometry::build_doorways;
use super::influx;
use super::physics::{
    compute_thermal_masses, full_room_energy_balance, pv_to_sw_vertical_irradiance, radiator_output,
};
use super::solar::{avg_irradiance_in_window, fetch_surface_irradiance};
use super::validation::{whole_house_metrics, RoomResidual, WholeHouseMetrics};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum HpState {
    Heating,
    Dhw,
    Off,
}

impl std::fmt::Display for HpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HpState::Heating => write!(f, "heating"),
            HpState::Dhw => write!(f, "dhw"),
            HpState::Off => write!(f, "off"),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct OperationalRecord {
    pub room: String,
    pub period_start: String,
    pub period_end: String,
    pub hp_state: String,
    pub mwt_avg_c: f64,
    pub outside_avg_c: f64,
    pub start_temp_c: f64,
    pub end_temp_c: f64,
    pub meas_rate_c_per_hr: f64,
    pub pred_rate_c_per_hr: f64,
    pub radiator_w: f64,
    pub loss_w: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OperationalSummary {
    pub n: usize,
    pub rmse: f64,
    pub mae: f64,
    pub bias: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct PerRoomOperationalSummary {
    pub room: String,
    pub n: usize,
    pub rmse: f64,
    pub mae: f64,
    pub bias: f64,
}

#[derive(Debug, Serialize)]
struct OperationalArtifact {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    config_path: String,
    config_sha256: String,
    git: GitMeta,
    range_start: String,
    range_end: String,
    calibrated_params: ArtifactCalibrationParams,
    summary_all: OperationalSummary,
    summary_by_state: Vec<(String, OperationalSummary)>,
    per_room: Vec<PerRoomOperationalSummary>,
    whole_house: WholeHouseMetrics,
    records: Vec<OperationalRecord>,
}

// ---------------------------------------------------------------------------
// HP state classification
// ---------------------------------------------------------------------------

fn classify_hp_state_from_flow(flow_lph: f64) -> HpState {
    if flow_lph > 900.0 {
        HpState::Dhw
    } else if flow_lph >= 780.0 {
        HpState::Heating
    } else {
        HpState::Off
    }
}

fn segment_by_flow(
    flow_rows: &[(DateTime<FixedOffset>, f64)],
    min_period_hours: f64,
) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>, HpState)> {
    if flow_rows.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut seg_start = flow_rows[0].0;
    let mut seg_state = classify_hp_state_from_flow(flow_rows[0].1);

    for &(t, flow) in &flow_rows[1..] {
        let state = classify_hp_state_from_flow(flow);
        if state != seg_state {
            let hours = (t - seg_start).num_seconds() as f64 / 3600.0;
            if hours >= min_period_hours {
                segments.push((seg_start, t, seg_state));
            }
            seg_start = t;
            seg_state = state;
        }
    }

    if let Some(&(t, _)) = flow_rows.last() {
        let hours = (t - seg_start).num_seconds() as f64 / 3600.0;
        if hours >= min_period_hours {
            segments.push((seg_start, t, seg_state));
        }
    }

    segments
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn operational_validate(config_path: &Path) -> ThermalResult<()> {
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
    let pg_conninfo = resolve_postgres_conninfo(cfg)?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &sensor_topics,
        &range_start,
        &range_end,
    )?;
    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &range_start,
        &range_end,
    )?;
    let bcf_rows = influx::query_building_circuit_flow(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &range_start,
        &range_end,
    )?;
    let mwt_rows = influx::query_mwt(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &range_start,
        &range_end,
    )?;
    let pv_rows = influx::query_pv_power(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &range_start,
        &range_end,
    )?;

    let solar_irradiance = fetch_surface_irradiance(51.60, -0.11, range_start, range_end);

    if bcf_rows.is_empty() {
        return Err(ThermalError::NoStatusData);
    }

    let room_series = build_room_series(&room_rows, rooms)?;
    let connections = &setup.connections;
    let doorways = build_doorways()?;

    let thermal_masses = compute_thermal_masses(rooms, connections);

    let segments = segment_by_flow(&bcf_rows, 5.0 / 60.0);

    println!("Config: {}", config_path.display());
    println!(
        "Operational range: {} -> {}",
        range_start.to_rfc3339(),
        range_end.to_rfc3339()
    );
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );
    println!(
        "Found {} segments ({} heating, {} DHW, {} off)",
        segments.len(),
        segments.iter().filter(|s| s.2 == HpState::Heating).count(),
        segments.iter().filter(|s| s.2 == HpState::Dhw).count(),
        segments.iter().filter(|s| s.2 == HpState::Off).count(),
    );

    println!(
        "\n{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8} {:>16}",
        "Room", "Start", "End", "Meas", "Pred", "Rad", "Loss", "State", "Period"
    );
    println!(
        "{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8}",
        "", "°C", "°C", "°C/hr", "°C/hr", "W", "W", ""
    );
    println!("{}", "─".repeat(100));

    let mut records = Vec::new();

    for &(seg_start, seg_end, hp_state) in &segments {
        let avg_mwt = avg_series_in_window(&mwt_rows, seg_start, seg_end, 0.0);
        let effective_mwt = match hp_state {
            HpState::Heating => avg_mwt,
            _ => 0.0,
        };

        let avg_outside = avg_series_in_window(&outside_rows, seg_start, seg_end, 8.0);
        let avg_temps = avg_room_temps_in_window(&room_series, seg_start, seg_end);

        let sleeping = {
            let hour = seg_start.hour();
            !(7..22).contains(&hour)
        };

        let avg_pv = avg_series_in_window(&pv_rows, seg_start, seg_end, 0.0);
        let pv_sw_vert = if avg_pv != 0.0 {
            pv_to_sw_vertical_irradiance(avg_pv)
        } else {
            0.0
        };

        let (_meteo_sw, ne_vert, ne_horiz, _se_vert) =
            avg_irradiance_in_window(&solar_irradiance, seg_start, seg_end);

        let sw_vert = if pv_sw_vert > 0.0 {
            pv_sw_vert
        } else {
            _meteo_sw
        };

        let period_str = format!(
            "{}→{}",
            seg_start.format("%m-%d %H:%M"),
            seg_end.format("%H:%M")
        );

        for (room_name, series) in &room_series {
            let temps_in_seg: Vec<(DateTime<FixedOffset>, f64)> = series
                .iter()
                .cloned()
                .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
                .collect();
            if temps_in_seg.len() < 2 {
                continue;
            }
            let (first, last) = match (temps_in_seg.first(), temps_in_seg.last()) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let hours = (last.0 - first.0).num_seconds() as f64 / 3600.0;
            if hours < 0.25 {
                continue;
            }

            let meas_rate = (first.1 - last.1) / hours;
            let Some(room) = rooms.get(room_name) else {
                continue;
            };

            let c = thermal_masses.get(room_name).copied().unwrap_or(0.0);

            let bal = full_room_energy_balance(
                room,
                avg_temps.get(room_name).copied().unwrap_or(first.1),
                avg_outside,
                &avg_temps,
                connections,
                &doorways,
                result.doorway_cd,
                1.0,
                effective_mwt,
                sleeping,
                sw_vert,
                ne_vert,
                ne_horiz,
            );
            let pred_rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };

            let rad_w: f64 = if effective_mwt > 0.0 {
                let rt = avg_temps.get(room_name).copied().unwrap_or(first.1);
                room.radiators
                    .iter()
                    .filter(|r| r.active)
                    .map(|r| radiator_output(r.t50, effective_mwt, rt))
                    .sum()
            } else {
                0.0
            };
            let loss_w = bal - rad_w;

            println!(
                "{:<14} {:>7.2} {:>7.2} {:>7.3} {:>7.3} {:>6.0} {:>6.0} {:>8} {:>16}",
                room_name,
                first.1,
                last.1,
                meas_rate,
                pred_rate,
                rad_w,
                loss_w,
                hp_state,
                period_str
            );

            records.push(OperationalRecord {
                room: room_name.clone(),
                period_start: seg_start.to_rfc3339(),
                period_end: seg_end.to_rfc3339(),
                hp_state: hp_state.to_string(),
                mwt_avg_c: effective_mwt,
                outside_avg_c: avg_outside,
                start_temp_c: first.1,
                end_temp_c: last.1,
                meas_rate_c_per_hr: meas_rate,
                pred_rate_c_per_hr: pred_rate,
                radiator_w: rad_w,
                loss_w,
            });
        }
    }

    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();
    let scored_records: Vec<&OperationalRecord> = records
        .iter()
        .filter(|r| !exclude_rooms.contains(&r.room))
        .collect();
    let scored_owned: Vec<OperationalRecord> =
        scored_records.iter().map(|r| (*r).clone()).collect();

    if !exclude_rooms.is_empty() {
        println!("\nExcluded from scoring: {:?}", cfg.objective.exclude_rooms);
    }

    let summary_all = operational_summary(&scored_owned);
    println!("\nSummary (scored rooms, all segments):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  bias={:+.3} °C/hr",
        summary_all.n, summary_all.rmse, summary_all.mae, summary_all.bias,
    );

    let mut summary_by_state = Vec::new();
    for state_name in ["heating", "off", "dhw"] {
        let subset: Vec<OperationalRecord> = scored_owned
            .iter()
            .filter(|r| r.hp_state == state_name)
            .cloned()
            .collect();
        let s = operational_summary(&subset);
        if s.n > 0 {
            println!(
                "  {}: N={}  RMSE={:.3}  MAE={:.3}  bias={:+.3}",
                state_name, s.n, s.rmse, s.mae, s.bias,
            );
        }
        summary_by_state.push((state_name.to_string(), s));
    }

    let per_room = operational_summary_by_room(&records);
    println!(
        "\n{:<14} {:>4} {:>8} {:>8} {:>8}",
        "Room", "N", "RMSE", "MAE", "Bias"
    );
    println!("{}", "─".repeat(52));
    for row in &per_room {
        let marker = if exclude_rooms.contains(&row.room) {
            " (excluded)"
        } else {
            ""
        };
        println!(
            "{:<14} {:>4} {:>8.3} {:>8.3} {:>+8.3}{}",
            row.room, row.n, row.rmse, row.mae, row.bias, marker
        );
    }

    let wh_entries: Vec<RoomResidual> = {
        let mut by_room: HashMap<String, (f64, f64, f64)> = HashMap::new();
        for r in &scored_owned {
            let c = thermal_masses.get(&r.room).copied().unwrap_or(0.0);
            let entry = by_room.entry(r.room.clone()).or_insert((0.0, 0.0, 0.0));
            entry.0 += r.meas_rate_c_per_hr;
            entry.1 += r.pred_rate_c_per_hr;
            entry.2 = c;
        }
        by_room
            .into_iter()
            .map(|(room, (m, p, c))| RoomResidual {
                room,
                measured: m,
                predicted: p,
                residual: p - m,
                abs_residual: (p - m).abs(),
                thermal_mass_kj_per_k: c,
            })
            .collect()
    };
    let wh = whole_house_metrics(&wh_entries, 5);
    println!(
        "\nWhole-house (scored rooms, thermal-mass weighted): meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
        wh.measured_w, wh.predicted_w, wh.error_w, wh.pred_over_meas
    );
    println!("Top error contributors:");
    for c in &wh.top_contributors {
        println!(
            "  {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
            c.room, c.measured_w, c.predicted_w, c.error_w
        );
    }

    let artifact = OperationalArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-operational".to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(cfg_txt),
        git: git_meta(),
        range_start: range_start.to_rfc3339(),
        range_end: range_end.to_rfc3339(),
        calibrated_params: ArtifactCalibrationParams {
            leather_ach: result.leather_ach,
            landing_ach: result.landing_ach,
            conservatory_ach: result.conservatory_ach,
            office_ach: result.office_ach,
            doorway_cd: result.doorway_cd,
        },
        summary_all,
        summary_by_state,
        per_room,
        whole_house: wh,
        records,
    };

    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("thermal-operational-{}.json", ts));
    let json = serde_json::to_string_pretty(&artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    println!("\nWrote operational artifact: {}", path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Summary helpers
// ---------------------------------------------------------------------------

fn operational_summary(records: &[OperationalRecord]) -> OperationalSummary {
    if records.is_empty() {
        return OperationalSummary {
            n: 0,
            rmse: f64::NAN,
            mae: f64::NAN,
            bias: f64::NAN,
        };
    }
    let errs: Vec<f64> = records
        .iter()
        .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
        .collect();
    let n = errs.len();
    let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / n as f64).sqrt();
    let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / n as f64;
    let bias = errs.iter().sum::<f64>() / n as f64;
    OperationalSummary { n, rmse, mae, bias }
}

fn operational_summary_by_room(records: &[OperationalRecord]) -> Vec<PerRoomOperationalSummary> {
    let mut by_room: BTreeMap<String, Vec<&OperationalRecord>> = BTreeMap::new();
    for r in records {
        by_room.entry(r.room.clone()).or_default().push(r);
    }
    by_room
        .into_iter()
        .map(|(room, rows)| {
            let errs: Vec<f64> = rows
                .iter()
                .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
                .collect();
            let n = errs.len();
            let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / n as f64).sqrt();
            let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / n as f64;
            let bias = errs.iter().sum::<f64>() / n as f64;
            PerRoomOperationalSummary {
                room,
                n,
                rmse,
                mae,
                bias,
            }
        })
        .collect()
}
