use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, FixedOffset};

use super::artifact::{build_artifact, write_artifact};
use super::config::{load_thermal_config, ThermalConfig, ValidationWindowCfg};
use super::error::{MeasuredRates, TempSeries, ThermalError, ThermalResult};
use super::geometry::{
    build_connections, build_doorways, build_rooms, Doorway, InternalConnection, RoomDef,
};
use super::influx;
use super::physics::{doors_all_closed_except_chimney, estimate_thermal_mass, room_energy_balance};
use super::report;
use super::wind::{fetch_open_meteo_wind, wind_multiplier_for_window};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

pub(crate) struct CalibrationSetup {
    pub rooms: BTreeMap<String, RoomDef>,
    pub connections: Vec<InternalConnection>,
    pub doors_n1: Vec<Doorway>,
    pub doors_n2: Vec<Doorway>,
    pub night1_start: DateTime<FixedOffset>,
    pub night1_end: DateTime<FixedOffset>,
    pub night2_start: DateTime<FixedOffset>,
    pub night2_end: DateTime<FixedOffset>,
    pub wind_mult_n1: f64,
    pub wind_avg_n1: f64,
    pub wind_mult_n2: f64,
    pub wind_avg_n2: f64,
    pub meas1: HashMap<String, f64>,
    pub avg1: HashMap<String, f64>,
    pub outside1: f64,
    pub meas2: HashMap<String, f64>,
    pub avg2: HashMap<String, f64>,
    pub outside2: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct CalibrationResult {
    pub final_score: f64,
    pub base_score: f64,
    pub leather_ach: f64,
    pub landing_ach: f64,
    pub conservatory_ach: f64,
    pub office_ach: f64,
    pub doorway_cd: f64,
    pub pred1: HashMap<String, f64>,
    pub pred2: HashMap<String, f64>,
    pub r1: f64,
    pub r2: f64,
}

pub(crate) struct ParsedWindow {
    pub name: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub door_state: String,
}

/// Result of load → prepare → grid search → set params (the common preamble).
pub(crate) struct CalibratedModel {
    pub cfg_txt: String,
    pub cfg: ThermalConfig,
    pub setup: CalibrationSetup,
    pub result: CalibrationResult,
    pub rooms: BTreeMap<String, RoomDef>,
}

/// Load config, run calibration grid search, and return rooms with calibrated params set.
/// This is the shared preamble for validate, fit_diagnostics, and operational_validate.
pub(crate) fn calibrate_model(config_path: &Path) -> ThermalResult<CalibratedModel> {
    let (cfg_txt, cfg) = load_thermal_config(config_path)?;
    let setup = prepare_calibration(&cfg)?;
    let result = run_grid_search(
        &cfg,
        setup.rooms.clone(),
        &setup.connections,
        &setup.doors_n1,
        &setup.doors_n2,
        &setup.meas1,
        &setup.avg1,
        setup.outside1,
        setup.wind_mult_n1,
        &setup.meas2,
        &setup.avg2,
        setup.outside2,
        setup.wind_mult_n2,
    )?;

    let mut rooms = build_rooms()?;
    set_calibration_params(
        &mut rooms,
        result.leather_ach,
        result.landing_ach,
        result.conservatory_ach,
        result.office_ach,
    )?;

    Ok(CalibratedModel {
        cfg_txt,
        cfg,
        setup,
        result,
        rooms,
    })
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn calibrate(config_path: &Path) -> ThermalResult<()> {
    let (cfg_txt, cfg) = load_thermal_config(config_path)?;
    let setup = prepare_calibration(&cfg)?;
    let result = run_grid_search(
        &cfg,
        setup.rooms.clone(),
        &setup.connections,
        &setup.doors_n1,
        &setup.doors_n2,
        &setup.meas1,
        &setup.avg1,
        setup.outside1,
        setup.wind_mult_n1,
        &setup.meas2,
        &setup.avg2,
        setup.outside2,
        setup.wind_mult_n2,
    )?;

    println!("Config: {}", config_path.display());
    println!(
        "Night1: {} -> {} (outside avg {:.1}°C)",
        setup.night1_start, setup.night1_end, setup.outside1
    );
    println!(
        "Night2: {} -> {} (outside avg {:.1}°C)",
        setup.night2_start, setup.night2_end, setup.outside2
    );
    println!(
        "Exclude rooms in objective: {:?}",
        cfg.objective.exclude_rooms
    );
    if cfg.wind.enabled {
        println!(
            "Wind model: enabled lat={:.4} lon={:.4} ach_per_ms={:.3} max_mult={:.2}",
            cfg.wind.latitude, cfg.wind.longitude, cfg.wind.ach_per_ms, cfg.wind.max_multiplier
        );
        println!(
            "  Night1 wind avg={:.2} m/s -> vent multiplier x{:.3}",
            setup.wind_avg_n1, setup.wind_mult_n1
        );
        println!(
            "  Night2 wind avg={:.2} m/s -> vent multiplier x{:.3}",
            setup.wind_avg_n2, setup.wind_mult_n2
        );
    }

    println!("\n========================================================================");
    println!("BEST FIT (direct Influx + config-driven bounds)");
    println!("========================================================================");
    println!("leather_ach      = {:.2}", result.leather_ach);
    println!("landing_ach      = {:.2}", result.landing_ach);
    println!("conservatory_ach = {:.2}", result.conservatory_ach);
    println!("office_ach       = {:.2}", result.office_ach);
    println!("doorway_cd       = {:.2}", result.doorway_cd);
    println!("rmse_night1      = {:.4}", result.r1);
    println!("rmse_night2      = {:.4}", result.r2);
    println!("base_score       = {:.4}", result.base_score);
    println!("final_score      = {:.4}", result.final_score);

    report::print_table("Night 1 fit", &setup.meas1, &result.pred1);
    report::print_table("Night 2 fit", &setup.meas2, &result.pred2);

    let artifact = build_artifact(
        "thermal-calibrate",
        config_path,
        &cfg_txt,
        &cfg,
        &setup,
        &result,
        None,
    )?;
    let artifact_path = write_artifact("thermal-calibrate", &artifact)?;
    println!("\nWrote calibration artifact: {}", artifact_path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Calibration setup and grid search
// ---------------------------------------------------------------------------

pub(crate) fn prepare_calibration(cfg: &ThermalConfig) -> ThermalResult<CalibrationSetup> {
    let night1_start = influx::parse_dt(&cfg.test_nights.night1_start)?;
    let night1_end = influx::parse_dt(&cfg.test_nights.night1_end)?;
    let night2_start = influx::parse_dt(&cfg.test_nights.night2_start)?;
    let night2_end = influx::parse_dt(&cfg.test_nights.night2_end)?;

    let rooms = build_rooms()?;
    let connections = build_connections()?;
    let doors_n1 = build_doorways()?;
    let doors_n2 = doors_all_closed_except_chimney(&doors_n1);

    let earliest = night1_start.min(night2_start);
    let latest = night1_end.max(night2_end);

    let wind_points = if cfg.wind.enabled {
        fetch_open_meteo_wind(cfg.wind.latitude, cfg.wind.longitude, earliest, latest)
    } else {
        Vec::new()
    };
    let (wind_mult_n1, wind_avg_n1) =
        wind_multiplier_for_window(&cfg.wind, &wind_points, night1_start, night1_end);
    let (wind_mult_n2, wind_avg_n2) =
        wind_multiplier_for_window(&cfg.wind, &wind_points, night2_start, night2_end);

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = std::env::var(&cfg.influx.token_env)
        .map_err(|_| ThermalError::MissingEnv(cfg.influx.token_env.clone()))?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &earliest,
        &latest,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &earliest,
        &latest,
    )?;

    let room_series = build_room_series(&room_rows, &rooms)?;
    let (meas1, avg1, outside1) =
        measured_rates(&room_series, &outside_rows, night1_start, night1_end)?;
    let (meas2, avg2, outside2) =
        measured_rates(&room_series, &outside_rows, night2_start, night2_end)?;

    Ok(CalibrationSetup {
        rooms,
        connections,
        doors_n1,
        doors_n2,
        night1_start,
        night1_end,
        night2_start,
        night2_end,
        wind_mult_n1,
        wind_avg_n1,
        wind_mult_n2,
        wind_avg_n2,
        meas1,
        avg1,
        outside1,
        meas2,
        avg2,
        outside2,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_grid_search(
    cfg: &ThermalConfig,
    mut rooms: BTreeMap<String, RoomDef>,
    connections: &[InternalConnection],
    doors_n1: &[Doorway],
    doors_n2: &[Doorway],
    meas1: &HashMap<String, f64>,
    avg1: &HashMap<String, f64>,
    outside1: f64,
    wind_mult_n1: f64,
    meas2: &HashMap<String, f64>,
    avg2: &HashMap<String, f64>,
    outside2: f64,
    wind_mult_n2: f64,
) -> ThermalResult<CalibrationResult> {
    use super::error::FitState;

    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();
    let mut best: Option<FitState> = None;

    for leather_ach in frange(
        cfg.bounds.leather_ach_min,
        cfg.bounds.leather_ach_max,
        cfg.bounds.leather_ach_step,
    ) {
        for landing_ach in frange(
            cfg.bounds.landing_ach_min,
            cfg.bounds.landing_ach_max,
            cfg.bounds.landing_ach_step,
        ) {
            for conservatory_ach in frange(
                cfg.bounds.conservatory_ach_min,
                cfg.bounds.conservatory_ach_max,
                cfg.bounds.conservatory_ach_step,
            ) {
                for office_ach in frange(
                    cfg.bounds.office_ach_min,
                    cfg.bounds.office_ach_max,
                    cfg.bounds.office_ach_step,
                ) {
                    for doorway_cd in frange(
                        cfg.bounds.doorway_cd_min,
                        cfg.bounds.doorway_cd_max,
                        cfg.bounds.doorway_cd_step,
                    ) {
                        set_calibration_params(
                            &mut rooms,
                            leather_ach,
                            landing_ach,
                            conservatory_ach,
                            office_ach,
                        )?;

                        let pred1 = predict_rates(
                            &rooms,
                            connections,
                            doors_n1,
                            avg1,
                            outside1,
                            doorway_cd,
                            wind_mult_n1,
                        );
                        let pred2 = predict_rates(
                            &rooms,
                            connections,
                            doors_n2,
                            avg2,
                            outside2,
                            doorway_cd,
                            wind_mult_n2,
                        );

                        let r1 = report::rmse(meas1, &pred1, &exclude_rooms);
                        let r2 = report::rmse(meas2, &pred2, &exclude_rooms);
                        let base_score = (r1 + r2) / 2.0;
                        let prior_penalty = cfg.objective.prior_weight
                            * (((landing_ach - cfg.priors.landing_ach) / 0.3).powi(2)
                                + ((doorway_cd - cfg.priors.doorway_cd) / 0.08).powi(2));
                        let final_score = base_score + prior_penalty;

                        match &best {
                            None => {
                                best = Some((
                                    final_score,
                                    leather_ach,
                                    landing_ach,
                                    conservatory_ach,
                                    office_ach,
                                    doorway_cd,
                                    base_score,
                                    pred1,
                                    pred2,
                                ));
                            }
                            Some((best_score, ..)) if final_score < *best_score => {
                                best = Some((
                                    final_score,
                                    leather_ach,
                                    landing_ach,
                                    conservatory_ach,
                                    office_ach,
                                    doorway_cd,
                                    base_score,
                                    pred1,
                                    pred2,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let (
        final_score,
        leather_ach,
        landing_ach,
        conservatory_ach,
        office_ach,
        doorway_cd,
        base_score,
        pred1,
        pred2,
    ) = best.ok_or(ThermalError::NoCalibrationCandidates)?;

    let r1 = report::rmse(meas1, &pred1, &exclude_rooms);
    let r2 = report::rmse(meas2, &pred2, &exclude_rooms);

    Ok(CalibrationResult {
        final_score,
        base_score,
        leather_ach,
        landing_ach,
        conservatory_ach,
        office_ach,
        doorway_cd,
        pred1,
        pred2,
        r1,
        r2,
    })
}

// ---------------------------------------------------------------------------
// Support functions
// ---------------------------------------------------------------------------

pub(crate) fn set_calibration_params(
    rooms: &mut BTreeMap<String, RoomDef>,
    leather_ach: f64,
    landing_ach: f64,
    conservatory_ach: f64,
    office_ach: f64,
) -> ThermalResult<()> {
    rooms
        .get_mut("leather")
        .ok_or(ThermalError::MissingRoom("leather"))?
        .ventilation_ach = leather_ach;
    rooms
        .get_mut("landing")
        .ok_or(ThermalError::MissingRoom("landing"))?
        .ventilation_ach = landing_ach;
    rooms
        .get_mut("conservatory")
        .ok_or(ThermalError::MissingRoom("conservatory"))?
        .ventilation_ach = conservatory_ach;
    rooms
        .get_mut("office")
        .ok_or(ThermalError::MissingRoom("office"))?
        .ventilation_ach = office_ach;
    Ok(())
}

pub(crate) fn predict_rates(
    rooms: &BTreeMap<String, RoomDef>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    avg_temps: &HashMap<String, f64>,
    outside_temp: f64,
    doorway_cd: f64,
    wind_multiplier: f64,
) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for (room_name, room) in rooms {
        if !avg_temps.contains_key(room_name) {
            continue;
        }
        let c = estimate_thermal_mass(room, connections);
        let bal = room_energy_balance(
            room,
            avg_temps[room_name],
            outside_temp,
            avg_temps,
            connections,
            doorways,
            doorway_cd,
            wind_multiplier,
        );
        let rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };
        out.insert(room_name.clone(), rate);
    }
    out
}

pub(crate) fn measured_rates(
    room_series: &TempSeries,
    outside_series: &[(DateTime<FixedOffset>, f64)],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> ThermalResult<MeasuredRates> {
    let outside_vals: Vec<f64> = outside_series
        .iter()
        .filter(|(t, _)| *t >= start && *t <= end)
        .map(|(_, v)| *v)
        .collect();

    if outside_vals.is_empty() {
        return Err(ThermalError::NoOutsideData);
    }

    let outside_avg = outside_vals.iter().sum::<f64>() / outside_vals.len() as f64;

    let mut rates = HashMap::new();
    let mut avg_temps = HashMap::new();

    for (room, points) in room_series {
        let p: Vec<(DateTime<FixedOffset>, f64)> = points
            .iter()
            .cloned()
            .filter(|(t, _)| *t >= start && *t <= end)
            .collect();

        if p.len() < 2 {
            continue;
        }

        let (first, last) = match (p.first(), p.last()) {
            (Some(first), Some(last)) => (first, last),
            _ => continue,
        };

        let hours = (last.0 - first.0).num_seconds() as f64 / 3600.0;
        if hours < 0.5 {
            continue;
        }

        let rate = (first.1 - last.1) / hours;
        let avg = p.iter().map(|(_, v)| *v).sum::<f64>() / p.len() as f64;

        rates.insert(room.clone(), rate);
        avg_temps.insert(room.clone(), avg);
    }

    Ok((rates, avg_temps, outside_avg))
}

pub(crate) fn build_room_series(
    room_rows: &[(DateTime<FixedOffset>, String, f64)],
    rooms: &BTreeMap<String, RoomDef>,
) -> ThermalResult<TempSeries> {
    let mut by_topic: HashMap<&str, &str> = HashMap::new();
    for room in rooms.values() {
        by_topic.insert(room.sensor_topic, room.name);
    }

    let mut out: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>> = HashMap::new();
    for (t, topic, value) in room_rows {
        if let Some(room) = by_topic.get(topic.as_str()) {
            out.entry((*room).to_string())
                .or_default()
                .push((*t, *value));
        }
    }

    for pts in out.values_mut() {
        pts.sort_by_key(|(t, _)| *t);
    }

    Ok(out)
}

pub(crate) fn frange(min: f64, max: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut x = min;
    while x <= max + 1e-12 {
        out.push(((x * 1_000_000.0).round()) / 1_000_000.0);
        x += step;
    }
    out
}

pub(crate) fn parse_validation_windows(
    raw: &[ValidationWindowCfg],
) -> ThermalResult<Vec<ParsedWindow>> {
    let mut out = Vec::new();
    for w in raw {
        out.push(ParsedWindow {
            name: w.name.clone(),
            start: influx::parse_dt(&w.start)?,
            end: influx::parse_dt(&w.end)?,
            door_state: w.door_state.clone(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Time-series averaging helpers
// ---------------------------------------------------------------------------

/// Average a scalar time series within a time window. Returns `default` if empty.
pub(crate) fn avg_series_in_window(
    series: &[(DateTime<FixedOffset>, f64)],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    default: f64,
) -> f64 {
    let vals: Vec<f64> = series
        .iter()
        .filter(|(t, _)| *t >= start && *t <= end)
        .map(|(_, v)| *v)
        .collect();
    if vals.is_empty() {
        default
    } else {
        vals.iter().sum::<f64>() / vals.len() as f64
    }
}

/// Compute average room temperatures within a time window from room series.
pub(crate) fn avg_room_temps_in_window(
    room_series: &TempSeries,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> HashMap<String, f64> {
    let mut avg_temps = HashMap::new();
    for (room_name, series) in room_series {
        let vals: Vec<f64> = series
            .iter()
            .filter(|(t, _)| *t >= start && *t <= end)
            .map(|(_, v)| *v)
            .collect();
        if !vals.is_empty() {
            avg_temps.insert(
                room_name.clone(),
                vals.iter().sum::<f64>() / vals.len() as f64,
            );
        }
    }
    avg_temps
}
