#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Deserialize)]
struct GeometryFile {
    rooms: Vec<GeometryRoom>,
    connections: Vec<GeometryConnection>,
    doorways: Vec<GeometryDoorway>,
}

#[derive(Debug, Deserialize)]
struct GeometryRoom {
    name: String,
    floor: String,
    floor_area: f64,
    ceiling_height: f64,
    construction: String,
    sensor: String,
    ventilation_ach: f64,
    heat_recovery: f64,
    overnight_occupants: i32,
    radiators: Vec<GeometryRadiator>,
    external_fabric: Vec<GeometryExternalElement>,
}

#[derive(Debug, Deserialize)]
struct GeometryRadiator {
    t50: f64,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
struct GeometryExternalElement {
    description: String,
    area: f64,
    u_value: f64,
    #[serde(default)]
    to_ground: bool,
}

#[derive(Debug, Deserialize)]
struct GeometryConnection {
    room_a: String,
    room_b: String,
    ua: f64,
}

#[derive(Debug, Deserialize)]
struct GeometryDoorway {
    room_a: String,
    room_b: String,
    width: f64,
    height: f64,
    state: String,
}

fn thermal_geometry_path() -> PathBuf {
    Path::new("data/canonical/thermal_geometry.json").to_path_buf()
}

fn load_thermal_geometry() -> ThermalResult<GeometryFile> {
    let path = thermal_geometry_path();
    let txt = fs::read_to_string(&path).map_err(|source| ThermalError::ConfigRead {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&txt).map_err(|source| ThermalError::GeometryParse {
        path: path.display().to_string(),
        source,
    })
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

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

    let mut rooms = build_rooms()?;
    let connections = build_connections()?;
    let doors_n1 = build_doorways()?;
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

fn build_rooms() -> ThermalResult<BTreeMap<String, RoomDef>> {
    let geo = load_thermal_geometry()?;
    let mut rooms = BTreeMap::new();

    for r in geo.rooms {
        let name = leak(r.name);
        let room = RoomDef {
            name,
            floor: leak(r.floor),
            floor_area: r.floor_area,
            ceiling_height: r.ceiling_height,
            construction: leak(r.construction),
            radiators: r
                .radiators
                .into_iter()
                .map(|rad| RadiatorDef {
                    t50: rad.t50,
                    active: rad.active,
                })
                .collect(),
            external_fabric: r
                .external_fabric
                .into_iter()
                .map(|e| ExternalElement {
                    description: leak(e.description),
                    area: e.area,
                    u_value: e.u_value,
                    to_ground: e.to_ground,
                })
                .collect(),
            sensor_topic: leak(r.sensor),
            ventilation_ach: r.ventilation_ach,
            heat_recovery: r.heat_recovery,
            overnight_occupants: r.overnight_occupants,
        };
        rooms.insert(name.to_string(), room);
    }

    Ok(rooms)
}

fn build_connections() -> ThermalResult<Vec<InternalConnection>> {
    let geo = load_thermal_geometry()?;
    Ok(geo
        .connections
        .into_iter()
        .map(|c| InternalConnection {
            room_a: leak(c.room_a),
            room_b: leak(c.room_b),
            ua: c.ua,
        })
        .collect())
}

fn build_doorways() -> ThermalResult<Vec<Doorway>> {
    let geo = load_thermal_geometry()?;
    Ok(geo
        .doorways
        .into_iter()
        .map(|d| Doorway {
            room_a: leak(d.room_a),
            room_b: leak(d.room_b),
            width: d.width,
            height: d.height,
            state: leak(d.state),
        })
        .collect())
}

