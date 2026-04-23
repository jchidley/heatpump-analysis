use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;

use super::config::{load_thermal_config, resolve_influx_token, resolve_postgres_conninfo};
use super::error::ThermalResult;
use super::geometry::{build_connections, build_doorways, build_rooms};
use super::influx;
use super::physics::{estimate_thermal_mass, full_room_energy_balance_components};

/// Print room summary table (equivalent to Python `cmd_rooms`).
pub fn print_rooms() -> ThermalResult<()> {
    let rooms = build_rooms()?;
    let connections = build_connections()?;

    println!(
        "{:<14} {:>4} {:>5} {:>5} {:>7} {:>6} {:>7} {:>5} {:>6} {:>3} {:>15}",
        "Room", "Flr", "Area", "Vol", "C kJ/K", "T50", "extUA", "ACH", "effACH", "Occ", "Pipe"
    );
    println!("{}", "─".repeat(100));

    let mut total_c = 0.0;
    for (name, room) in &rooms {
        let vol = room.floor_area * room.ceiling_height;
        let c = estimate_thermal_mass(room, &connections);
        total_c += c;
        let total_t50: f64 = room
            .radiators
            .iter()
            .filter(|r| r.active)
            .map(|r| r.t50)
            .sum();
        let total_t50 = if total_t50 == 0.0 { 0.0 } else { total_t50 };
        let ext_ua: f64 = room
            .external_fabric
            .iter()
            .map(|e| e.u_value * e.area)
            .sum();
        let ext_ua = if ext_ua == 0.0 { 0.0 } else { ext_ua };
        let eff_ach = room.ventilation_ach * (1.0 - room.heat_recovery);
        let pipe = room.radiators.first().map(|r| r.pipe).unwrap_or("none");
        println!(
            "{:<14} {:>4} {:>4.1}m² {:>4.0}m³ {:>6.0} {:>5.0}W {:>6.1}W/K {:>5.2} {:>6.2} {:>3} {:>15}",
            name, room.floor, room.floor_area, vol, c, total_t50, ext_ua,
            room.ventilation_ach, eff_ach, room.overnight_occupants, pipe
        );
    }

    println!("{}", "─".repeat(100));
    println!(
        "{:<14} {:>4} {:>5} {:>5} {:>6.0}",
        "Total", "", "", "", total_c
    );

    Ok(())
}

/// Print inter-room connections and doorways (equivalent to Python `cmd_connections`).
pub fn print_connections() -> ThermalResult<()> {
    let connections = build_connections()?;
    let doorways = build_doorways()?;

    println!("INTERNAL WALL/FLOOR CONNECTIONS (symmetric)");
    println!("{:<30} {:>8} Description", "A↔B", "UA W/K");
    println!("{}", "─".repeat(60));
    for c in &connections {
        println!(
            "{}↔{:<16} {:>8.1} {}",
            c.room_a, c.room_b, c.ua, c.description
        );
    }

    println!("\nDOORWAY EXCHANGES (buoyancy-driven)");
    println!("{:<30} {:>8} {:>8}", "A↔B", "W×H", "State");
    println!("{}", "─".repeat(50));
    for d in &doorways {
        println!(
            "{}↔{:<16} {:.1}×{:.1} {:>8}",
            d.room_a, d.room_b, d.width, d.height, d.state
        );
    }

    Ok(())
}

/// Live energy balance from InfluxDB (equivalent to Python `analyse`).
///
/// Queries the latest room temperatures, outside temperature, and MWT from
/// InfluxDB, then computes and prints the per-room energy balance.
pub fn print_analyse(config_path: &Path) -> ThermalResult<()> {
    let (_, cfg) = load_thermal_config(config_path)?;
    let token = resolve_influx_token(&cfg)?;
    let pg_conninfo = resolve_postgres_conninfo(&cfg)?;
    let rooms = build_rooms()?;
    let connections = build_connections()?;
    let doorways = build_doorways()?;

    // Query last 30 minutes to catch battery sensors (~5min reporting interval)
    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    let now = Utc::now().with_timezone(&utc);
    let start = now - chrono::Duration::minutes(30);

    // Collect sensor topics
    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &sensor_topics,
        &start,
        &now,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &start,
        &now,
    )?;

    let mwt_rows = influx::query_mwt(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &start,
        &now,
    )?;

    // Build topic → room name map
    let topic_to_room: HashMap<&str, &str> =
        rooms.values().map(|r| (r.sensor_topic, r.name)).collect();

    // Extract latest room temps
    let mut room_temps: HashMap<String, f64> = HashMap::new();
    for (_, topic, value) in &room_rows {
        if let Some(&room_name) = topic_to_room.get(topic.as_str()) {
            room_temps.insert(room_name.to_string(), *value);
        }
    }

    let outside_temp = outside_rows.last().map(|(_, v)| *v).unwrap_or(10.0);

    let mwt = mwt_rows.last().map(|(_, v)| *v).unwrap_or(0.0);

    // Calibrated params — use defaults from AGENTS.md
    let doorway_cd = 0.20;
    let wind_multiplier = 1.0;
    let sleeping = false;
    let sw_vert = 0.0; // No solar estimate for snapshot
    let ne_vert = 0.0;
    let ne_horiz = 0.0;

    // Query HP heat output and electrical consumption
    let hp_heat = influx::query_latest_topic_value(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        "ebusd/hmu/CurrentYieldPower",
        &start,
        &now,
    )?
    .unwrap_or(0.0)
        * 1000.0; // kW → W
    let hp_elec = influx::query_latest_topic_value(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        "ebusd/hmu/RunDataElectricPowerConsumption",
        &start,
        &now,
    )?
    .unwrap_or(0.0);

    println!("{}", "=".repeat(110));
    println!("STEADY-STATE ENERGY BALANCE");
    println!(
        "Outside: {:.1}°C | HP: {:.0}W heat, {:.0}W elec | MWT: {:.1}°C | Rooms: {}/{}",
        outside_temp,
        hp_heat,
        hp_elec,
        mwt,
        room_temps.len(),
        rooms.len()
    );
    println!("{}", "=".repeat(110));

    let hdr = format!(
        "{:<14} {:>5} {:>7} {:>6} {:>7} {:>6} {:>5} {:>5} {:>8} {:>6} {:>7}",
        "Room", "T°C", "ExtFab", "Vent", "Walls", "Doors", "Body", "DHW", "NetLoss", "Rad", "Resid"
    );
    println!("\n{hdr}");
    println!("{}", "─".repeat(hdr.len()));

    let mut tot_ext = 0.0;
    let mut tot_vent = 0.0;
    let mut tot_walls = 0.0;
    let mut tot_doors = 0.0;
    let mut tot_body = 0.0;
    let mut tot_dhw = 0.0;
    let mut tot_rad = 0.0;

    for (name, _room) in &rooms {
        let t = match room_temps.get(name.as_str()) {
            Some(v) => *v,
            None => continue,
        };
        let bal = full_room_energy_balance_components(
            _room,
            t,
            outside_temp,
            &room_temps,
            &connections,
            &doorways,
            doorway_cd,
            wind_multiplier,
            mwt,
            sleeping,
            sw_vert,
            ne_vert,
            ne_horiz,
        );

        tot_ext += bal.external;
        tot_vent += bal.ventilation;
        tot_walls += bal.walls;
        tot_doors += bal.doorways;
        tot_body += bal.body;
        tot_dhw += bal.dhw;
        tot_rad += bal.radiator;

        let net_loss = -(bal.external + bal.ventilation + bal.walls + bal.doorways);
        let dhw_str = if bal.dhw > 0.0 {
            format!("{:>5.0}", bal.dhw)
        } else {
            "     ".to_string()
        };
        println!(
            "{:<14} {:>5.1} {:>7.0} {:>6.0} {:>7.0} {:>6.0} {:>5.0} {} {:>8.0} {:>6.0} {:>+7.0}",
            name,
            t,
            -bal.external,
            -bal.ventilation,
            -bal.walls,
            -bal.doorways,
            bal.body,
            dhw_str,
            net_loss - bal.body - bal.dhw,
            bal.radiator,
            bal.total
        );
    }

    println!("{}", "─".repeat(hdr.len()));
    let total_loss = -(tot_ext + tot_vent + tot_walls + tot_doors);
    println!(
        "{:<14} {:>5} {:>7.0} {:>6.0} {:>7.0} {:>6.0} {:>5.0} {:>5.0} {:>8.0} {:>6.0}",
        "Total",
        "",
        -tot_ext,
        -tot_vent,
        -tot_walls,
        -tot_doors,
        tot_body,
        tot_dhw,
        total_loss - tot_body - tot_dhw,
        tot_rad
    );
    println!(
        "{:<14} {:>5} {:>7} {:>6} {:>7} {:>6} {:>5} {:>5} {:>8} {:>6.0}",
        "HP meter", "", "", "", "", "", "", "", "", hp_heat
    );

    Ok(())
}

/// Pure equilibrium solver: returns a map of room name → equilibrium temperature.
///
/// No I/O, no printing — suitable for programmatic use (e.g. control table generation).
pub fn solve_equilibrium_temps(
    outside_temp: f64,
    mwt: f64,
    irr_sw: f64,
    irr_ne: f64,
) -> ThermalResult<std::collections::BTreeMap<String, f64>> {
    let rooms = build_rooms()?;
    let connections = build_connections()?;
    let doorways = build_doorways()?;
    let doorway_cd = 0.20;
    let wind_multiplier = 1.0;
    let sleeping = false;
    let ne_horiz = 0.0;

    let room_names: Vec<String> = rooms.keys().cloned().collect();
    let mut temps: HashMap<String, f64> = room_names.iter().map(|n| (n.clone(), 19.0)).collect();

    let max_iter = 200;
    let tol = 1e-4;
    for _iter in 0..max_iter {
        let mut max_change: f64 = 0.0;
        for name in &room_names {
            let room = &rooms[name];
            let mut lo = -10.0_f64;
            let mut hi = 50.0_f64;
            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                temps.insert(name.clone(), mid);
                let bal = full_room_energy_balance_components(
                    room,
                    mid,
                    outside_temp,
                    &temps,
                    &connections,
                    &doorways,
                    doorway_cd,
                    wind_multiplier,
                    mwt,
                    sleeping,
                    irr_sw,
                    irr_ne,
                    ne_horiz,
                );
                if bal.total > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
                if (hi - lo) < tol * 0.01 {
                    break;
                }
            }
            let new_t = (lo + hi) / 2.0;
            let old_t = temps.insert(name.clone(), new_t).unwrap_or(new_t);
            max_change = max_change.max((new_t - old_t).abs());
        }
        if max_change < tol {
            break;
        }
    }

    Ok(temps.into_iter().collect())
}

/// Bisect MWT to find the value that produces a target room temperature.
///
/// Returns `None` if not achievable in range 15..60°C.
pub fn bisect_mwt_for_room(
    room_name: &str,
    target_temp: f64,
    outside_temp: f64,
    irr_sw: f64,
    irr_ne: f64,
) -> ThermalResult<Option<f64>> {
    let mut lo = 15.0_f64;
    let mut hi = 60.0_f64;

    // Check if target is achievable at all
    let temps_at_hi = solve_equilibrium_temps(outside_temp, hi, irr_sw, irr_ne)?;
    if let Some(&t) = temps_at_hi.get(room_name) {
        if t < target_temp {
            return Ok(None); // can't reach target even at max MWT
        }
    } else {
        return Ok(None);
    }

    // Check if target is already met with no heating
    let temps_at_lo = solve_equilibrium_temps(outside_temp, lo, irr_sw, irr_ne)?;
    if let Some(&t) = temps_at_lo.get(room_name) {
        if t >= target_temp {
            return Ok(Some(lo)); // already warm enough
        }
    }

    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        let temps = solve_equilibrium_temps(outside_temp, mid, irr_sw, irr_ne)?;
        if let Some(&t) = temps.get(room_name) {
            if t < target_temp {
                lo = mid;
            } else {
                hi = mid;
            }
        } else {
            return Ok(None);
        }
        if (hi - lo) < 0.05 {
            break;
        }
    }
    Ok(Some((lo + hi) / 2.0))
}

/// Generate a control lookup table for the adaptive heating controller.
///
/// Grid: outside temps from -5 to 20°C (1°C steps), solar 0/100/300/500 W/m².
/// For each point, bisects MWT to find Leather = 20.5°C.
pub fn generate_control_table(config_path: &Path) -> ThermalResult<()> {
    let _ = load_thermal_config(config_path)?; // validate config exists

    let target_leather = 20.5;
    let outside_range: Vec<i32> = (-5..=20).collect();
    let solar_levels = [0.0, 100.0, 300.0, 500.0];

    #[derive(serde::Serialize)]
    struct ControlPoint {
        outside_c: f64,
        solar_w_m2: f64,
        required_mwt: Option<f64>,
    }

    let mut table = Vec::new();
    for &outside in &outside_range {
        for &solar in &solar_levels {
            let mwt = bisect_mwt_for_room("leather", target_leather, outside as f64, solar, 0.0)?;
            table.push(ControlPoint {
                outside_c: outside as f64,
                solar_w_m2: solar,
                required_mwt: mwt,
            });
            let mwt_str = mwt
                .map(|v| format!("{:.1}", v))
                .unwrap_or("N/A".to_string());
            println!(
                "outside={:>3}°C  solar={:>3}W/m²  → MWT={}",
                outside, solar as i32, mwt_str
            );
        }
    }

    let out_path = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("control-table.json");
    let json = serde_json::to_string_pretty(&table)
        .map_err(|e| super::error::ThermalError::ArtifactSerialize(e))?;
    std::fs::write(&out_path, &json).map_err(|e| super::error::ThermalError::ArtifactWrite {
        path: out_path.display().to_string(),
        source: e,
    })?;
    println!("\nWritten to {}", out_path.display());
    Ok(())
}

/// Solve for equilibrium room temperatures at given outside temp and MWT.
///
/// Uses Gauss-Seidel iteration with bisection per room: for each room in turn,
/// find the temperature where total energy balance = 0 (holding other rooms
/// fixed), then sweep repeatedly until all rooms converge.
pub fn print_equilibrium(
    config_path: &Path,
    outside_temp_override: Option<f64>,
    mwt_override: Option<f64>,
    irr_sw: f64,
    irr_ne: f64,
) -> ThermalResult<()> {
    let (_, cfg) = load_thermal_config(config_path)?;
    let token = resolve_influx_token(&cfg)?;
    let pg_conninfo = resolve_postgres_conninfo(&cfg)?;
    let rooms = build_rooms()?;
    let connections = build_connections()?;
    let doorways = build_doorways()?;

    // Get current conditions from InfluxDB if not overridden
    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    let now = Utc::now().with_timezone(&utc);
    let start = now - chrono::Duration::minutes(30);

    let outside_temp = if let Some(v) = outside_temp_override {
        v
    } else {
        let rows = influx::query_outside_temp(
            &cfg.influx.url,
            &cfg.influx.org,
            &cfg.influx.bucket,
            &token,
            pg_conninfo.as_deref(),
            &start,
            &now,
        )?;
        rows.last().map(|(_, v)| *v).unwrap_or(10.0)
    };

    let mwt = if let Some(v) = mwt_override {
        v
    } else {
        let rows = influx::query_mwt(
            &cfg.influx.url,
            &cfg.influx.org,
            &cfg.influx.bucket,
            &token,
            pg_conninfo.as_deref(),
            &start,
            &now,
        )?;
        rows.last().map(|(_, v)| *v).unwrap_or(0.0)
    };

    let doorway_cd = 0.20;
    let wind_multiplier = 1.0;
    let sleeping = false;
    let ne_horiz = 0.0;

    let room_names: Vec<String> = rooms.keys().cloned().collect();

    // Initial guess: all rooms at 19°C
    let mut temps: HashMap<String, f64> = room_names.iter().map(|n| (n.clone(), 19.0)).collect();

    // Gauss-Seidel iteration with bisection per room
    let max_iter = 200;
    let tol = 1e-4; // °C
    for _iter in 0..max_iter {
        let mut max_change: f64 = 0.0;

        for name in &room_names {
            let room = &rooms[name];
            // Bisection: find T where energy_balance_total(T) = 0
            // Energy balance is monotonically decreasing with T (hotter = more loss)
            let mut lo = -10.0_f64;
            let mut hi = 50.0_f64;

            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                temps.insert(name.clone(), mid);
                let bal = full_room_energy_balance_components(
                    room,
                    mid,
                    outside_temp,
                    &temps,
                    &connections,
                    &doorways,
                    doorway_cd,
                    wind_multiplier,
                    mwt,
                    sleeping,
                    irr_sw,
                    irr_ne,
                    ne_horiz,
                );
                if bal.total > 0.0 {
                    lo = mid; // too cold, room gaining heat → raise temp
                } else {
                    hi = mid; // too hot, room losing heat → lower temp
                }
                if (hi - lo) < tol * 0.01 {
                    break;
                }
            }

            let new_t = (lo + hi) / 2.0;
            let old_t = temps.insert(name.clone(), new_t).unwrap_or(new_t);
            max_change = max_change.max((new_t - old_t).abs());
        }

        if max_change < tol {
            break;
        }
    }

    // Print results
    println!("{}", "=".repeat(70));
    println!(
        "EQUILIBRIUM TEMPERATURES (T_out={:.1}°C, MWT={:.1}°C)",
        outside_temp, mwt
    );
    println!("{}", "=".repeat(70));

    println!(
        "\n{:<14} {:>6} {:>7} {:>8} {:>9} Notes",
        "Room", "Temp", "Rad_in", "Ext_out", "Vent_out"
    );
    println!("{}", "─".repeat(60));

    for name in &room_names {
        let t = temps[name];
        let room = &rooms[name];
        let bal = full_room_energy_balance_components(
            room,
            t,
            outside_temp,
            &temps,
            &connections,
            &doorways,
            doorway_cd,
            wind_multiplier,
            mwt,
            true, // sleeping=true for display
            irr_sw,
            irr_ne,
            ne_horiz,
        );
        let mut notes = String::new();
        let has_active_rad = room.radiators.iter().any(|r| r.active);
        if !has_active_rad {
            notes = "no active rad".to_string();
        } else if t < 18.0 {
            notes = "COLD".to_string();
        }
        println!(
            "{:<14} {:>5.1}° {:>6.0}W {:>7.0}W {:>8.0}W  {}",
            name, t, bal.radiator, -bal.external, -bal.ventilation, notes
        );
    }

    // Design summary
    let heated: Vec<&String> = room_names
        .iter()
        .filter(|n| rooms[*n].radiators.iter().any(|r| r.active))
        .collect();
    if !heated.is_empty() {
        let coldest = heated
            .iter()
            .min_by(|a, b| temps[a.as_str()].partial_cmp(&temps[b.as_str()]).unwrap())
            .unwrap();
        println!(
            "\nColdest heated room: {} at {:.1}°C",
            coldest,
            temps[coldest.as_str()]
        );
        if temps[coldest.as_str()] < 18.0 {
            println!("  → needs higher MWT to reach 18°C");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Moisture analysis
// ---------------------------------------------------------------------------

const MOISTURE_PERSON_SLEEPING: f64 = 40.0; // g/h per person
const RSI: f64 = 0.13; // Internal surface resistance m²K/W

/// Absolute humidity in g/m³ from T (°C) and RH (%) using Magnus formula.
fn absolute_humidity(temp_c: f64, rh_pct: f64) -> f64 {
    let es = 6.112 * (17.67 * temp_c / (temp_c + 243.5)).exp();
    217.0 * (rh_pct / 100.0) * es / (temp_c + 273.15)
}

/// RH at a surface colder than room air.
fn surface_rh(air_temp: f64, air_rh: f64, surface_temp: f64) -> f64 {
    let es_air = 6.112 * (17.67 * air_temp / (air_temp + 243.5)).exp();
    let e = (air_rh / 100.0) * es_air;
    let es_surface = 6.112 * (17.67 * surface_temp / (surface_temp + 243.5)).exp();
    (e / es_surface * 100.0).min(100.0)
}

/// Fetch outside humidity from Open-Meteo (overnight hours). Falls back to 75% RH.
fn fetch_outside_humidity(avg_outside: f64) -> (f64, f64) {
    let date = Utc::now().format("%Y-%m-%d");
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude=51.59&longitude=-0.14\
         &hourly=relative_humidity_2m,temperature_2m\
         &timezone=Europe/London\
         &start_date={date}&end_date={date}"
    );
    if let Ok(resp) = reqwest::blocking::get(&url) {
        if let Ok(body) = resp.text() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                let times = val["hourly"]["time"].as_array();
                let temps = val["hourly"]["temperature_2m"].as_array();
                let rhs = val["hourly"]["relative_humidity_2m"].as_array();
                if let (Some(ts), Some(tvs), Some(rvs)) = (times, temps, rhs) {
                    let mut ah_vals = Vec::new();
                    let mut rh_vals = Vec::new();
                    for i in 0..ts.len() {
                        let h_str = ts[i].as_str().unwrap_or("");
                        let h: u32 = h_str.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(12);
                        if h >= 22 || h <= 7 {
                            let t = tvs[i].as_f64().unwrap_or(avg_outside);
                            let rh = rvs[i].as_f64().unwrap_or(75.0);
                            ah_vals.push(absolute_humidity(t, rh));
                            rh_vals.push(rh);
                        }
                    }
                    if !ah_vals.is_empty() {
                        let avg_ah = ah_vals.iter().sum::<f64>() / ah_vals.len() as f64;
                        let avg_rh = rh_vals.iter().sum::<f64>() / rh_vals.len() as f64;
                        return (avg_ah, avg_rh);
                    }
                }
            }
        }
    }
    (absolute_humidity(avg_outside, 75.0), 75.0)
}

/// Moisture analysis: current snapshot + overnight moisture balance.
pub fn print_moisture(config_path: &Path) -> ThermalResult<()> {
    let (_, cfg) = load_thermal_config(config_path)?;
    let token = resolve_influx_token(&cfg)?;
    let pg_conninfo = resolve_postgres_conninfo(&cfg)?;
    let rooms = build_rooms()?;

    // Query last 24h of data for overnight analysis
    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    let now = Utc::now().with_timezone(&utc);
    let start_24h = now - chrono::Duration::hours(24);

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();

    // Query room temps (includes humidity for _temp_humid sensors)
    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &sensor_topics,
        &start_24h,
        &now,
    )?;

    // Also query humidity — need to build humidity queries
    let humidity_rows = influx::query_room_humidity(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &sensor_topics,
        &start_24h,
        &now,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        pg_conninfo.as_deref(),
        &start_24h,
        &now,
    )?;

    let avg_outside = if outside_rows.is_empty() {
        10.0
    } else {
        outside_rows.iter().map(|(_, v)| v).sum::<f64>() / outside_rows.len() as f64
    };

    // Build topic → room name map
    let topic_to_room: HashMap<&str, &str> =
        rooms.values().map(|r| (r.sensor_topic, r.name)).collect();

    // Build per-room time series: {room -> [(time_minute_key, temp, rh)]}
    let mut room_temps_map: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    let mut room_humid_map: HashMap<String, Vec<(String, f64)>> = HashMap::new();

    for (t, topic, value) in &room_rows {
        if let Some(&room_name) = topic_to_room.get(topic.as_str()) {
            let key = t.format("%Y-%m-%dT%H:%M").to_string();
            room_temps_map
                .entry(room_name.to_string())
                .or_default()
                .push((key, *value));
        }
    }
    for (t, topic, value) in &humidity_rows {
        if let Some(&room_name) = topic_to_room.get(topic.as_str()) {
            let key = t.format("%Y-%m-%dT%H:%M").to_string();
            room_humid_map
                .entry(room_name.to_string())
                .or_default()
                .push((key, *value));
        }
    }

    // Get outside humidity from Open-Meteo
    let (outside_ah, outside_rh) = fetch_outside_humidity(avg_outside);

    println!("{}", "=".repeat(100));
    println!("MOISTURE ANALYSIS");
    println!(
        "Outside: {:.1}°C, ~{:.0}% RH → AH {:.1} g/m³",
        avg_outside, outside_rh, outside_ah
    );
    println!("{}", "=".repeat(100));

    // Current snapshot
    println!(
        "\n{:<14} {:>5} {:>5} {:>8} {:>6} {:>6} {:>7} {:>6}",
        "Room", "T°C", "RH%", "AH g/m³", "U_max", "T_surf", "SurfRH", "Risk"
    );
    println!("{}", "─".repeat(65));

    for (name, room) in &rooms {
        // Get latest temp and humidity
        let latest_temp = room_temps_map
            .get(name.as_str())
            .and_then(|v| v.last().map(|(_, t)| *t));
        let latest_rh = room_humid_map
            .get(name.as_str())
            .and_then(|v| v.last().map(|(_, h)| *h));

        let (temp, rh) = match (latest_temp, latest_rh) {
            (Some(t), Some(h)) => (t, h),
            _ => continue,
        };

        let ah = absolute_humidity(temp, rh);
        let u_max = room
            .external_fabric
            .iter()
            .filter(|e| !e.to_ground)
            .map(|e| e.u_value)
            .fold(0.0_f64, f64::max);
        let t_surface = if u_max > 0.0 {
            temp - u_max * RSI * (temp - avg_outside)
        } else {
            temp - 1.0
        };
        let s_rh = surface_rh(temp, rh, t_surface);
        let risk = if s_rh > 80.0 {
            "HIGH"
        } else if s_rh > 70.0 {
            "WARN"
        } else if rh > 60.0 {
            "watch"
        } else {
            "OK"
        };
        println!(
            "{:<14} {:>5.1} {:>5.1} {:>8.1} {:>6.2} {:>6.1} {:>7.1} {:>6}",
            name, temp, rh, ah, u_max, t_surface, s_rh, risk
        );
    }

    // Overnight moisture balance
    println!("\n{}", "─".repeat(100));
    println!("OVERNIGHT MOISTURE BALANCE");
    println!("{}", "─".repeat(100));
    println!(
        "\n{:<14} {:>3} {:>7} {:>7} {:>6} {:>10} {:>10} {:>6}",
        "Room", "Occ", "AH_23", "AH_06", "ΔAH", "ACH_moist", "ACH_therm", "Match"
    );
    println!(
        "{:<14} {:>3} {:>7} {:>7} {:>6} {:>10} {:>10}",
        "", "", "g/m³", "g/m³", "g/m³", "(total)", "(to out)"
    );
    println!("{}", "─".repeat(75));

    for (name, room) in &rooms {
        let vol = room.floor_area * room.ceiling_height;
        let occ = room.overnight_occupants;

        // Find AH at ~23:00 and ~06:00
        let temps_series = room_temps_map.get(name.as_str());
        let humid_series = room_humid_map.get(name.as_str());
        let (temps_s, humid_s) = match (temps_series, humid_series) {
            (Some(t), Some(h)) => (t, h),
            _ => continue,
        };

        // Build minute-keyed map for matching
        let temp_by_key: HashMap<&str, f64> =
            temps_s.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let humid_by_key: HashMap<&str, f64> =
            humid_s.iter().map(|(k, v)| (k.as_str(), *v)).collect();

        let mut ah_23: Option<f64> = None;
        let mut ah_06: Option<f64> = None;

        let mut all_keys: Vec<&str> = temp_by_key.keys().copied().collect();
        all_keys.sort();
        for key in &all_keys {
            let h: u32 = key.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(99);
            if let (Some(&t), Some(&rh)) = (temp_by_key.get(key), humid_by_key.get(key)) {
                let ah = absolute_humidity(t, rh);
                if h == 23 && ah_23.is_none() {
                    ah_23 = Some(ah);
                }
                if h == 6 {
                    ah_06 = Some(ah);
                }
            }
        }

        let (a23, a06) = match (ah_23, ah_06) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };

        let delta_ah = a06 - a23;
        let hours = 7.0;
        let moisture_rate = occ as f64 * MOISTURE_PERSON_SLEEPING / vol;
        let observed_rate = delta_ah / hours;
        let vent_removal = moisture_rate - observed_rate;
        let ah_avg = (a23 + a06) / 2.0;
        let ah_diff = ah_avg - outside_ah;
        let ach_moisture = if ah_diff > 0.5 {
            vent_removal / ah_diff
        } else {
            0.0
        };

        let ach_thermal = room.ventilation_ach * (1.0 - room.heat_recovery);
        let m = if occ > 0 && ach_moisture > 0.0 {
            if (ach_moisture - ach_thermal).abs() < 0.3 {
                "✓"
            } else {
                "≠"
            }
        } else {
            "-"
        };

        println!(
            "{:<14} {:>3} {:>7.2} {:>7.2} {:>+6.2} {:>10.2} {:>10.2} {:>6}",
            name, occ, a23, a06, delta_ah, ach_moisture, ach_thermal, m
        );
    }

    println!("\n  ACH_moist = total air exchange (to outside + inter-room), from humidity change");
    println!("  ACH_therm = to outside only, from thermal model");
    println!(
        "  ACH_moist ≥ ACH_therm expected (doorway exchange adds to moisture but not thermal)"
    );
    println!(
        "  Moisture rate: {} g/h/person (±25% → ±50% ACH uncertainty)",
        MOISTURE_PERSON_SLEEPING
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn leather_temp_at(outside_c: f64, mwt: f64) -> f64 {
        *solve_equilibrium_temps(outside_c, mwt, 0.0, 0.0)
            .expect("solver should succeed")
            .get("leather")
            .expect("leather room should exist")
    }

    // @lat: [[tests#Thermal solver#Leather equilibrium rises with MWT]]
    #[test]
    fn leather_equilibrium_rises_with_higher_mwt() {
        let low = leather_temp_at(8.0, 25.0);
        let high = leather_temp_at(8.0, 35.0);

        assert!(
            high > low,
            "expected higher MWT to warm leather: low={low}, high={high}"
        );
    }

    // @lat: [[tests#Thermal solver#MWT bisection hits the requested room target]]
    #[test]
    fn bisected_mwt_hits_leather_target_with_small_error() {
        let target = 20.5;
        let mwt = bisect_mwt_for_room("leather", target, 10.0, 0.0, 0.0)
            .expect("bisection should succeed")
            .expect("target should be achievable");

        let temp = leather_temp_at(10.0, mwt);
        assert!(
            (temp - target).abs() <= 0.05,
            "target miss too large: mwt={mwt}, temp={temp}"
        );

        let slightly_lower = leather_temp_at(10.0, mwt - 0.2);
        assert!(
            slightly_lower < target,
            "bisection should return a near-minimum MWT: mwt={mwt}, lower_temp={slightly_lower}, target={target}"
        );
    }

    // @lat: [[tests#Thermal solver#Unreachable targets return no MWT]]
    #[test]
    fn unreachable_target_returns_none() {
        let max_temp = leather_temp_at(-5.0, 60.0);
        let unreachable = max_temp + 0.5;

        let mwt = bisect_mwt_for_room("leather", unreachable, -5.0, 0.0, 0.0)
            .expect("bisection should succeed for unreachable case");

        assert_eq!(
            mwt, None,
            "target above max achievable temp should return None"
        );
    }

    // @lat: [[tests#Thermal solver#Already warm targets return the minimum MWT]]
    #[test]
    fn already_warm_target_returns_lower_bound_mwt() {
        let temp_at_min_mwt = leather_temp_at(18.0, 15.0);
        let target = temp_at_min_mwt - 0.2;

        let mwt = bisect_mwt_for_room("leather", target, 18.0, 0.0, 0.0)
            .expect("bisection should succeed for already-warm case");

        assert_eq!(
            mwt,
            Some(15.0),
            "target already met at minimum MWT should short-circuit to the lower bound"
        );
    }

    #[test]
    fn already_warm_target_at_exact_lower_bound_still_returns_minimum_mwt() {
        let target = leather_temp_at(18.0, 15.0);

        let mwt = bisect_mwt_for_room("leather", target, 18.0, 0.0, 0.0)
            .expect("bisection should succeed for exact lower-bound case");

        assert_eq!(
            mwt,
            Some(15.0),
            "target exactly met at minimum MWT should still short-circuit to the lower bound"
        );
    }

    // @lat: [[tests#Thermal solver#Unknown rooms return no MWT]]
    #[test]
    fn unknown_room_returns_none() {
        let mwt = bisect_mwt_for_room("not_a_real_room", 20.0, 10.0, 0.0, 0.0)
            .expect("bisection should gracefully handle an unknown room");

        assert_eq!(mwt, None, "unknown rooms should not produce an MWT");
    }

    // @lat: [[tests#Thermal physics primitives#Absolute humidity rises with temperature]]
    #[test]
    fn absolute_humidity_rises_with_temperature() {
        let low = absolute_humidity(10.0, 50.0);
        let high = absolute_humidity(20.0, 50.0);
        assert!(
            high > low,
            "warmer air holds more moisture: {low} vs {high}"
        );
        // Known reference: ~20°C, 50% RH → ~8.6 g/m³
        let ref_val = absolute_humidity(20.0, 50.0);
        assert!(
            ref_val > 7.0 && ref_val < 10.0,
            "reference check: {ref_val}"
        );
    }

    // @lat: [[tests#Thermal physics primitives#Surface RH reaches 100 pct at dew point]]
    #[test]
    fn surface_rh_saturates_at_cold_surface() {
        // Very cold surface relative to room → should saturate at 100%
        let rh = surface_rh(20.0, 60.0, 5.0);
        assert_eq!(rh, 100.0, "cold surface should saturate");
        // Surface at air temperature → should equal air RH
        let rh = surface_rh(20.0, 60.0, 20.0);
        assert!((rh - 60.0).abs() < 0.1, "same-temp surface = air RH: {rh}");
    }

    proptest! {
        // @lat: [[tests#Thermal solver#Leather equilibrium is monotonic in MWT]]
        #[test]
        fn leather_equilibrium_is_monotonic_in_mwt(
            low in 15.0f64..55.0,
            delta in 0.1f64..5.0,
            outside in -2.0f64..16.0,
        ) {
            let high = (low + delta).min(60.0);
            prop_assume!(high > low);

            let low_temp = leather_temp_at(outside, low);
            let high_temp = leather_temp_at(outside, high);

            prop_assert!(
                high_temp + 1e-6 >= low_temp,
                "equilibrium regressed: outside={outside}, low={low}, high={high}, low_temp={low_temp}, high_temp={high_temp}"
            );
        }

        // @lat: [[tests#Thermal physics primitives#Absolute humidity is monotonic in temperature]]
        #[test]
        fn absolute_humidity_monotonic_in_temp(
            t_low in -10.0f64..35.0,
            delta in 0.1f64..10.0,
            rh in 10.0f64..100.0,
        ) {
            let t_high = t_low + delta;
            prop_assert!(
                absolute_humidity(t_high, rh) >= absolute_humidity(t_low, rh),
                "humidity should rise with temperature"
            );
        }

        // @lat: [[tests#Thermal physics primitives#Surface RH equals air RH at same temperature]]
        #[test]
        fn surface_rh_identity_at_same_temp(
            temp in 5.0f64..30.0,
            rh in 10.0f64..90.0,
        ) {
            let result = surface_rh(temp, rh, temp);
            prop_assert!(
                (result - rh).abs() < 0.1,
                "surface at air temp should give air RH: result={result}, rh={rh}"
            );
        }
    }

    // ── Migration routing contracts ────────────────────────────────────────

    // @lat: [[tests#Display migration contracts#Humidity query skips emonth2 topic]]
    #[test]
    fn humidity_query_skips_emonth2() {
        // query_room_humidity explicitly skips emon/emonth2_23/temperature
        // because that sensor doesn't report humidity. This skip must survive
        // the PostgreSQL migration.
        let sensor_topics: &[&str] = &[
            "zigbee2mqtt/Leather",
            "emon/emonth2_23/temperature",
            "zigbee2mqtt/Aldora",
        ];

        let mut conditions = Vec::new();
        for t in sensor_topics {
            if *t == "emon/emonth2_23/temperature" {
                continue; // skip — no humidity
            }
            conditions.push(format!("topic={t}"));
        }

        assert_eq!(
            conditions.len(),
            2,
            "emonth2_23 must be skipped for humidity"
        );
        assert!(conditions[0].contains("Leather"));
        assert!(conditions[1].contains("Aldora"));
    }

    // @lat: [[tests#Display migration contracts#Humidity uses humidity field not temperature]]
    #[test]
    fn humidity_field_name_contract() {
        // query_room_humidity uses _field == "humidity" for all topics.
        // This is different from query_room_temps which uses "temperature" or "value".
        // In PG: Zigbee sensors → zigbee table, "humidity" column.
        let topic = "zigbee2mqtt/Leather";
        let field = "humidity";
        let condition = format!("(r.topic == \"{topic}\" and r._field == \"{field}\")");
        assert!(
            condition.contains("humidity"),
            "humidity query must use humidity field"
        );
    }
}
