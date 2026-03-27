#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

mod error;
mod influx;
mod report;

use error::{FitState, MeasuredRates, TempSeries, ThermalError, ThermalResult};

#[derive(Debug, Deserialize)]
struct ThermalConfig {
    influx: InfluxCfg,
    test_nights: TestNights,
    objective: ObjectiveCfg,
    priors: PriorsCfg,
    bounds: BoundsCfg,
}

#[derive(Debug, Deserialize)]
struct InfluxCfg {
    url: String,
    org: String,
    bucket: String,
    token_env: String,
}

#[derive(Debug, Deserialize)]
struct TestNights {
    night1_start: String,
    night1_end: String,
    night2_start: String,
    night2_end: String,
}

#[derive(Debug, Deserialize)]
struct ObjectiveCfg {
    #[serde(default)]
    exclude_rooms: Vec<String>,
    #[serde(default)]
    prior_weight: f64,
}

#[derive(Debug, Deserialize)]
struct PriorsCfg {
    landing_ach: f64,
    doorway_cd: f64,
}

#[derive(Debug, Deserialize)]
struct BoundsCfg {
    leather_ach_min: f64,
    leather_ach_max: f64,
    leather_ach_step: f64,

    landing_ach_min: f64,
    landing_ach_max: f64,
    landing_ach_step: f64,

    conservatory_ach_min: f64,
    conservatory_ach_max: f64,
    conservatory_ach_step: f64,

    office_ach_min: f64,
    office_ach_max: f64,
    office_ach_step: f64,

    doorway_cd_min: f64,
    doorway_cd_max: f64,
    doorway_cd_step: f64,
}


#[allow(dead_code)]
#[derive(Clone)]
struct RadiatorDef {
    t50: f64,
    active: bool,
}

#[derive(Clone)]
struct ExternalElement {
    description: &'static str,
    area: f64,
    u_value: f64,
    to_ground: bool,
}

#[allow(dead_code)]
#[derive(Clone)]
struct RoomDef {
    name: &'static str,
    floor: &'static str,
    floor_area: f64,
    ceiling_height: f64,
    construction: &'static str,
    radiators: Vec<RadiatorDef>,
    external_fabric: Vec<ExternalElement>,
    sensor_topic: &'static str,
    ventilation_ach: f64,
    heat_recovery: f64,
    overnight_occupants: i32,
}

#[derive(Clone)]
struct InternalConnection {
    room_a: &'static str,
    room_b: &'static str,
    ua: f64,
}

#[derive(Clone)]
struct Doorway {
    room_a: &'static str,
    room_b: &'static str,
    width: f64,
    height: f64,
    state: &'static str, // open/closed/partial/chimney
}

const AIR_DENSITY: f64 = 1.2;
const AIR_CP: f64 = 1005.0;
const VENT_FACTOR: f64 = AIR_DENSITY * AIR_CP / 3600.0;
const GROUND_TEMP_C: f64 = 10.5;
const RAD_EXPONENT: f64 = 1.3;
const U_INTERNAL_WALL: f64 = 2.37;
const DOORWAY_G: f64 = 9.81;

const BODY_HEAT_SLEEPING_W: f64 = 70.0;
const DHW_CYLINDER_UA: f64 = 1.6;
const DHW_CYLINDER_TEMP: f64 = 44.0;
const DHW_PIPE_LOSS_W: f64 = 42.0;
const DHW_SHOWER_W: f64 = 16.0;

fn thermal_mass_air(vol_m3: f64) -> f64 { 1.2 * vol_m3 }
fn thermal_mass_brick_int(area: f64) -> f64 { 72.0 * area }
fn thermal_mass_brick_ext(area: f64) -> f64 { 72.0 * area }
fn thermal_mass_concrete(area: f64) -> f64 { 200.0 * area }
fn thermal_mass_timber_floor(area: f64) -> f64 { 50.0 * area }
fn thermal_mass_plaster(area: f64) -> f64 { 17.0 * area }
fn thermal_mass_furniture(area: f64) -> f64 { 15.0 * area }
fn thermal_mass_timber_stud(area: f64) -> f64 { 10.0 * area }

pub fn calibrate(config_path: &Path) -> ThermalResult<()> {
    let cfg_txt = fs::read_to_string(config_path).map_err(|source| ThermalError::ConfigRead {
        path: config_path.display().to_string(),
        source,
    })?;
    let cfg: ThermalConfig = toml::from_str(&cfg_txt).map_err(|source| ThermalError::ConfigParse {
        path: config_path.display().to_string(),
        source,
    })?;

    let night1_start = influx::parse_dt(&cfg.test_nights.night1_start)?;
    let night1_end = influx::parse_dt(&cfg.test_nights.night1_end)?;
    let night2_start = influx::parse_dt(&cfg.test_nights.night2_start)?;
    let night2_end = influx::parse_dt(&cfg.test_nights.night2_end)?;

    let mut rooms = build_rooms();
    let connections = build_connections();
    let doors_n1 = build_doorways();
    let doors_n2 = doors_all_closed_except_chimney(&doors_n1);

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let earliest = night1_start.min(night2_start);
    let latest = night1_end.max(night2_end);

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

    let (meas1, avg1, outside1) = measured_rates(&room_series, &outside_rows, night1_start, night1_end)?;
    let (meas2, avg2, outside2) = measured_rates(&room_series, &outside_rows, night2_start, night2_end)?;

    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();

    println!("Config: {}", config_path.display());
    println!(
        "Night1: {} -> {} (outside avg {:.1}°C)",
        night1_start, night1_end, outside1
    );
    println!(
        "Night2: {} -> {} (outside avg {:.1}°C)",
        night2_start, night2_end, outside2
    );
    println!("Exclude rooms in objective: {:?}", cfg.objective.exclude_rooms);

    let mut best: Option<FitState> = None;

    for leather_ach in frange(cfg.bounds.leather_ach_min, cfg.bounds.leather_ach_max, cfg.bounds.leather_ach_step) {
        for landing_ach in frange(cfg.bounds.landing_ach_min, cfg.bounds.landing_ach_max, cfg.bounds.landing_ach_step) {
            for conservatory_ach in frange(cfg.bounds.conservatory_ach_min, cfg.bounds.conservatory_ach_max, cfg.bounds.conservatory_ach_step) {
                for office_ach in frange(cfg.bounds.office_ach_min, cfg.bounds.office_ach_max, cfg.bounds.office_ach_step) {
                    for doorway_cd in frange(cfg.bounds.doorway_cd_min, cfg.bounds.doorway_cd_max, cfg.bounds.doorway_cd_step) {
                        set_calibration_params(&mut rooms, leather_ach, landing_ach, conservatory_ach, office_ach)?;

                        let pred1 = predict_rates(&rooms, &connections, &doors_n1, &avg1, outside1, doorway_cd);
                        let pred2 = predict_rates(&rooms, &connections, &doors_n2, &avg2, outside2, doorway_cd);

                        let r1 = report::rmse(&meas1, &pred1, &exclude_rooms);
                        let r2 = report::rmse(&meas2, &pred2, &exclude_rooms);
                        let base_score = (r1 + r2) / 2.0;
                        let prior_penalty = cfg.objective.prior_weight * (
                            ((landing_ach - cfg.priors.landing_ach) / 0.3).powi(2)
                                + ((doorway_cd - cfg.priors.doorway_cd) / 0.08).powi(2)
                        );
                        let final_score = base_score + prior_penalty;

                        match &best {
                            None => {
                                best = Some((
                                    final_score, leather_ach, landing_ach, conservatory_ach, office_ach, doorway_cd,
                                    base_score, pred1, pred2,
                                ));
                            }
                            Some((best_score, ..)) if final_score < *best_score => {
                                best = Some((
                                    final_score, leather_ach, landing_ach, conservatory_ach, office_ach, doorway_cd,
                                    base_score, pred1, pred2,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let (final_score, leather_ach, landing_ach, conservatory_ach, office_ach, doorway_cd, base_score, pred1, pred2) =
        best.ok_or(ThermalError::NoCalibrationCandidates)?;

    let r1 = report::rmse(&meas1, &pred1, &exclude_rooms);
    let r2 = report::rmse(&meas2, &pred2, &exclude_rooms);

    println!("\n========================================================================");
    println!("BEST FIT (direct Influx + config-driven bounds)");
    println!("========================================================================");
    println!("leather_ach      = {:.2}", leather_ach);
    println!("landing_ach      = {:.2}", landing_ach);
    println!("conservatory_ach = {:.2}", conservatory_ach);
    println!("office_ach       = {:.2}", office_ach);
    println!("doorway_cd       = {:.2}", doorway_cd);
    println!("rmse_night1      = {:.4}", r1);
    println!("rmse_night2      = {:.4}", r2);
    println!("base_score       = {:.4}", base_score);
    println!("final_score      = {:.4}", final_score);

    report::print_table("Night 1 fit", &meas1, &pred1);
    report::print_table("Night 2 fit", &meas2, &pred2);

    Ok(())
}

fn set_calibration_params(
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

fn predict_rates(
    rooms: &BTreeMap<String, RoomDef>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    avg_temps: &HashMap<String, f64>,
    outside_temp: f64,
    doorway_cd: f64,
) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for (room_name, room) in rooms {
        if !avg_temps.contains_key(room_name) {
            continue;
        }
        let c = estimate_thermal_mass(room, connections);
        let bal = room_energy_balance(room, avg_temps[room_name], outside_temp, avg_temps, connections, doorways, doorway_cd);
        let rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };
        out.insert(room_name.clone(), rate);
    }
    out
}

fn measured_rates(
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

fn build_room_series(
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
            out.entry((*room).to_string()).or_default().push((*t, *value));
        }
    }

    for pts in out.values_mut() {
        pts.sort_by_key(|(t, _)| *t);
    }

    Ok(out)
}

fn frange(min: f64, max: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut x = min;
    while x <= max + 1e-12 {
        out.push(((x * 1_000_000.0).round()) / 1_000_000.0);
        x += step;
    }
    out
}

fn doors_all_closed_except_chimney(doors: &[Doorway]) -> Vec<Doorway> {
    doors
        .iter()
        .map(|d| {
            let mut d2 = d.clone();
            if d2.state != "chimney" {
                d2.state = "closed";
            }
            d2
        })
        .collect()
}

fn estimate_thermal_mass(room: &RoomDef, connections: &[InternalConnection]) -> f64 {
    let vol = room.floor_area * room.ceiling_height;
    let mut c = 0.0;

    c += thermal_mass_air(vol);

    for elem in &room.external_fabric {
        if elem.description.to_ascii_lowercase().contains("wall") {
            if room.construction == "brick" || room.construction == "brick_suspended" {
                c += thermal_mass_brick_ext(elem.area);
            } else {
                c += thermal_mass_timber_stud(elem.area);
            }
            c += thermal_mass_plaster(elem.area);
        }
    }

    for conn in connections {
        if (conn.room_a == room.name || conn.room_b == room.name) && conn.ua > 0.0 {
            let implied_area = conn.ua / U_INTERNAL_WALL;
            if room.construction == "brick" || room.construction == "brick_suspended" {
                c += thermal_mass_brick_int(implied_area);
            } else {
                c += thermal_mass_timber_stud(implied_area);
            }
            c += thermal_mass_plaster(implied_area);
        }
    }

    if room.floor == "Gnd" && room.construction != "brick_suspended" {
        c += thermal_mass_concrete(room.floor_area);
    } else {
        c += thermal_mass_timber_floor(room.floor_area);
    }

    c += thermal_mass_plaster(room.floor_area);
    c += thermal_mass_furniture(room.floor_area);

    c
}

fn room_energy_balance(
    room: &RoomDef,
    room_temp: f64,
    outside_temp: f64,
    all_temps: &HashMap<String, f64>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    doorway_cd: f64,
) -> f64 {
    let name = room.name;
    let vol = room.floor_area * room.ceiling_height;

    let q_ext = -external_loss(&room.external_fabric, room_temp, outside_temp);
    let q_vent = -ventilation_loss(room.ventilation_ach, vol, room_temp, outside_temp, room.heat_recovery);

    let q_rad = 0.0; // cooldown calibration assumes mwt=0
    let q_body = room.overnight_occupants as f64 * BODY_HEAT_SLEEPING_W;
    let q_solar = 0.0;

    let mut q_dhw = 0.0;
    if name == "bathroom" {
        q_dhw = DHW_CYLINDER_UA * (DHW_CYLINDER_TEMP - room_temp).max(0.0) + DHW_PIPE_LOSS_W + DHW_SHOWER_W;
    }

    let mut q_walls = 0.0;
    for conn in connections {
        if conn.room_a == name {
            if let Some(other_t) = all_temps.get(conn.room_b) {
                q_walls -= wall_conduction(conn.ua, room_temp, *other_t);
            }
        } else if conn.room_b == name {
            if let Some(other_t) = all_temps.get(conn.room_a) {
                q_walls -= wall_conduction(conn.ua, room_temp, *other_t);
            }
        }
    }

    let mut q_doors = 0.0;
    for door in doorways {
        if door.room_a == name {
            if let Some(other_t) = all_temps.get(door.room_b) {
                q_doors -= doorway_exchange(door, room_temp, *other_t, doorway_cd);
            }
        } else if door.room_b == name {
            if let Some(other_t) = all_temps.get(door.room_a) {
                q_doors -= doorway_exchange(door, room_temp, *other_t, doorway_cd);
            }
        }
    }

    q_ext + q_vent + q_rad + q_body + q_solar + q_dhw + q_walls + q_doors
}

fn external_loss(elements: &[ExternalElement], room_temp: f64, outside_temp: f64) -> f64 {
    elements
        .iter()
        .map(|e| {
            let ref_temp = if e.to_ground { GROUND_TEMP_C } else { outside_temp };
            e.u_value * e.area * (room_temp - ref_temp)
        })
        .sum()
}

fn ventilation_loss(ach: f64, volume: f64, room_temp: f64, outside_temp: f64, heat_recovery: f64) -> f64 {
    VENT_FACTOR * ach * volume * (room_temp - outside_temp) * (1.0 - heat_recovery)
}

fn wall_conduction(ua: f64, temp_a: f64, temp_b: f64) -> f64 {
    ua * (temp_a - temp_b)
}

fn doorway_exchange(door: &Doorway, temp_a: f64, temp_b: f64, doorway_cd: f64) -> f64 {
    if door.state == "closed" || door.state == "chimney" {
        return 0.0;
    }

    let dt = temp_a - temp_b;
    if dt.abs() < 0.01 {
        return 0.0;
    }

    let t_mean = (temp_a + temp_b) / 2.0 + 273.15;
    let mut width = door.width;
    if door.state == "partial" {
        width *= 0.5;
    }

    let flow = (doorway_cd / 3.0)
        * width
        * (DOORWAY_G * door.height.powi(3) * dt.abs() / t_mean).sqrt();

    flow * AIR_DENSITY * AIR_CP * dt
}

#[allow(dead_code)]
fn radiator_output(t50: f64, mwt: f64, room_temp: f64) -> f64 {
    let dt = mwt - room_temp;
    if dt <= 0.0 {
        0.0
    } else {
        t50 * (dt / 50.0).powf(RAD_EXPONENT)
    }
}

fn build_rooms() -> BTreeMap<String, RoomDef> {
    let mut rooms = BTreeMap::new();

    rooms.insert("hall".into(), RoomDef {
        name: "hall", floor: "Gnd", floor_area: 9.72, ceiling_height: 2.6, construction: "brick_suspended",
        sensor_topic: "zigbee2mqtt/hall_temp_humid", ventilation_ach: 0.10, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 2376.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 16.80, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Ground Floor", area: 9.72, u_value: 0.75, to_ground: true },
            ExternalElement { description: "Windows", area: 1.92, u_value: 1.9, to_ground: false },
            ExternalElement { description: "Loft Windows", area: 1.44, u_value: 1.5, to_ground: false },
        ],
    });

    rooms.insert("kitchen".into(), RoomDef {
        name: "kitchen", floor: "Gnd", floor_area: 8.8, ceiling_height: 2.6, construction: "brick",
        sensor_topic: "zigbee2mqtt/kitchen_temp_humid", ventilation_ach: 0.10, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 8.96, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Ground Floor", area: 8.8, u_value: 0.50, to_ground: true },
            ExternalElement { description: "Windows", area: 1.44, u_value: 1.9, to_ground: false },
        ],
    });

    rooms.insert("leather".into(), RoomDef {
        name: "leather", floor: "Gnd", floor_area: 17.0, ceiling_height: 2.6, construction: "brick_suspended",
        sensor_topic: "emon/emonth2_23/temperature", ventilation_ach: 0.67, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![
            RadiatorDef { t50: 2376.0, active: true },
            RadiatorDef { t50: 2376.0, active: true },
        ],
        external_fabric: vec![ExternalElement { description: "Ground Floor", area: 17.0, u_value: 0.50, to_ground: true }],
    });

    rooms.insert("front".into(), RoomDef {
        name: "front", floor: "Gnd", floor_area: 16.34, ceiling_height: 2.6, construction: "brick_suspended",
        sensor_topic: "zigbee2mqtt/front_temp_humid", ventilation_ach: 0.75, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 2425.0, active: true }, RadiatorDef { t50: 2376.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 8.14, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Ground Floor", area: 16.34, u_value: 0.75, to_ground: true },
            ExternalElement { description: "Windows", area: 7.2, u_value: 1.2, to_ground: false },
        ],
    });

    rooms.insert("conservatory".into(), RoomDef {
        name: "conservatory", floor: "Gnd", floor_area: 21.0, ceiling_height: 2.6, construction: "brick",
        sensor_topic: "zigbee2mqtt/conservatory_temp_humid", ventilation_ach: 1.00, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 2833.0, active: true }, RadiatorDef { t50: 2867.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 15.4, u_value: 0.5, to_ground: false },
            ExternalElement { description: "Ground Floor", area: 21.0, u_value: 0.40, to_ground: true },
            ExternalElement { description: "Glazed Roof", area: 21.0, u_value: 2.4, to_ground: false },
            ExternalElement { description: "Windows", area: 9.0, u_value: 1.9, to_ground: false },
        ],
    });

    rooms.insert("sterling".into(), RoomDef {
        name: "sterling", floor: "1st", floor_area: 18.0, ceiling_height: 2.4, construction: "brick",
        sensor_topic: "zigbee2mqtt/Sterling_temp_humid", ventilation_ach: 0.05, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 1176.0, active: false }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 6.12, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Windows", area: 2.52, u_value: 1.0, to_ground: false },
        ],
    });

    rooms.insert("jackcarol".into(), RoomDef {
        name: "jackcarol", floor: "1st", floor_area: 14.28, ceiling_height: 2.4, construction: "brick",
        sensor_topic: "zigbee2mqtt/jackcarol_temp_humid", ventilation_ach: 0.29, heat_recovery: 0.0, overnight_occupants: 2,
        radiators: vec![RadiatorDef { t50: 1950.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 6.69, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Windows", area: 6.75, u_value: 1.2, to_ground: false },
        ],
    });

    rooms.insert("bathroom".into(), RoomDef {
        name: "bathroom", floor: "1st", floor_area: 18.0, ceiling_height: 2.4, construction: "brick",
        sensor_topic: "zigbee2mqtt/bathroom_temp_humid", ventilation_ach: 0.75, heat_recovery: 0.78, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 614.0, active: true }, RadiatorDef { t50: 382.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 10.92, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Windows", area: 2.52, u_value: 1.0, to_ground: false },
        ],
    });

    rooms.insert("office".into(), RoomDef {
        name: "office", floor: "1st", floor_area: 5.28, ceiling_height: 2.4, construction: "brick",
        sensor_topic: "zigbee2mqtt/office_temp_humid", ventilation_ach: 1.20, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 1345.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 8.94, u_value: 2.11, to_ground: false },
            ExternalElement { description: "Windows", area: 2.1, u_value: 1.2, to_ground: false },
        ],
    });

    rooms.insert("landing".into(), RoomDef {
        name: "landing", floor: "1st", floor_area: 6.0, ceiling_height: 2.4, construction: "timber",
        sensor_topic: "zigbee2mqtt/landing_temp_humid", ventilation_ach: 1.30, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![],
        external_fabric: vec![ExternalElement { description: "External Wall", area: 3.0, u_value: 2.11, to_ground: false }],
    });

    rooms.insert("elvina".into(), RoomDef {
        name: "elvina", floor: "Loft", floor_area: 27.5, ceiling_height: 2.2, construction: "timber",
        sensor_topic: "zigbee2mqtt/elvina_temp_humid", ventilation_ach: 0.51, heat_recovery: 0.0, overnight_occupants: 1,
        radiators: vec![RadiatorDef { t50: 909.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 53.73, u_value: 0.15, to_ground: false },
            ExternalElement { description: "Roof", area: 26.64, u_value: 0.066, to_ground: false },
            ExternalElement { description: "Velux", area: 0.858, u_value: 1.0, to_ground: false },
            ExternalElement { description: "Windows", area: 2.37, u_value: 1.6, to_ground: false },
        ],
    });

    rooms.insert("aldora".into(), RoomDef {
        name: "aldora", floor: "Loft", floor_area: 14.0, ceiling_height: 2.2, construction: "timber",
        sensor_topic: "zigbee2mqtt/aldora_temp_humid", ventilation_ach: 0.30, heat_recovery: 0.0, overnight_occupants: 1,
        radiators: vec![RadiatorDef { t50: 376.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 30.84, u_value: 0.15, to_ground: false },
            ExternalElement { description: "Roof", area: 13.57, u_value: 0.066, to_ground: false },
            ExternalElement { description: "Velux", area: 0.429, u_value: 1.0, to_ground: false },
            ExternalElement { description: "Windows", area: 2.16, u_value: 1.5, to_ground: false },
        ],
    });

    rooms.insert("shower".into(), RoomDef {
        name: "shower", floor: "Loft", floor_area: 4.14, ceiling_height: 2.2, construction: "timber",
        sensor_topic: "zigbee2mqtt/shower_temp_humid", ventilation_ach: 0.05, heat_recovery: 0.0, overnight_occupants: 0,
        radiators: vec![RadiatorDef { t50: 752.0, active: true }],
        external_fabric: vec![
            ExternalElement { description: "External Wall", area: 19.62, u_value: 0.15, to_ground: false },
            ExternalElement { description: "Roof", area: 3.71, u_value: 0.066, to_ground: false },
            ExternalElement { description: "Velux", area: 0.429, u_value: 1.0, to_ground: false },
            ExternalElement { description: "Windows", area: 0.84, u_value: 1.5, to_ground: false },
        ],
    });

    rooms
}

fn build_connections() -> Vec<InternalConnection> {
    let u_w = 2.37;
    let u_f = 1.58;
    vec![
        InternalConnection { room_a: "hall", room_b: "kitchen", ua: u_w * 6.0 },
        InternalConnection { room_a: "hall", room_b: "leather", ua: u_w * 5.0 },
        InternalConnection { room_a: "hall", room_b: "front", ua: u_w * 7.72 },
        InternalConnection { room_a: "kitchen", room_b: "leather", ua: u_w * 8.0 },
        InternalConnection { room_a: "kitchen", room_b: "front", ua: u_w * 7.84 },
        InternalConnection { room_a: "front", room_b: "leather", ua: u_w * 10.0 },
        InternalConnection { room_a: "leather", room_b: "conservatory", ua: 4.4 * 4.8 },

        InternalConnection { room_a: "hall", room_b: "office", ua: 0.25 * 5.28 },
        InternalConnection { room_a: "kitchen", room_b: "bathroom", ua: u_f * 8.8 },
        InternalConnection { room_a: "front", room_b: "jackcarol", ua: u_f * 14.28 },
        InternalConnection { room_a: "leather", room_b: "sterling", ua: u_f * 17.0 },

        InternalConnection { room_a: "sterling", room_b: "bathroom", ua: u_w * 6.0 },
        InternalConnection { room_a: "sterling", room_b: "jackcarol", ua: u_w * 10.0 },
        InternalConnection { room_a: "sterling", room_b: "landing", ua: u_w * 4.0 },
        InternalConnection { room_a: "jackcarol", room_b: "office", ua: u_w * 6.0 },
        InternalConnection { room_a: "jackcarol", room_b: "landing", ua: u_w * 4.0 },
        InternalConnection { room_a: "bathroom", room_b: "landing", ua: u_w * 4.0 },
        InternalConnection { room_a: "office", room_b: "landing", ua: u_w * 3.0 },

        InternalConnection { room_a: "hall", room_b: "elvina", ua: 0.15 * 5.66 },

        InternalConnection { room_a: "bathroom", room_b: "shower", ua: 0.44 * 18.0 },
        InternalConnection { room_a: "sterling", room_b: "aldora", ua: 0.44 * 18.0 },
        InternalConnection { room_a: "jackcarol", room_b: "elvina", ua: 0.44 * 14.28 },
        InternalConnection { room_a: "office", room_b: "elvina", ua: 0.44 * 5.28 },
    ]
}

fn build_doorways() -> Vec<Doorway> {
    vec![
        Doorway { room_a: "hall", room_b: "kitchen", width: 0.8, height: 2.0, state: "open" },
        Doorway { room_a: "kitchen", room_b: "conservatory", width: 0.8, height: 2.0, state: "open" },
        Doorway { room_a: "hall", room_b: "front", width: 0.8, height: 2.0, state: "partial" },

        Doorway { room_a: "hall", room_b: "landing", width: 0.9, height: 2.5, state: "chimney" },
        Doorway { room_a: "landing", room_b: "shower", width: 0.7, height: 2.0, state: "chimney" },

        Doorway { room_a: "landing", room_b: "bathroom", width: 0.8, height: 2.0, state: "open" },
        Doorway { room_a: "landing", room_b: "office", width: 0.8, height: 2.0, state: "open" },
        Doorway { room_a: "landing", room_b: "jackcarol", width: 0.8, height: 2.0, state: "closed" },
        Doorway { room_a: "landing", room_b: "sterling", width: 0.8, height: 2.0, state: "closed" },
    ]
}
