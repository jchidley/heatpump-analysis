use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;

use super::config::{load_thermal_config, resolve_influx_token};
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
    println!("{:<30} {:>8} {}", "A↔B", "UA W/K", "Description");
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
        &sensor_topics,
        &start,
        &now,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &start,
        &now,
    )?;

    let mwt_rows = influx::query_mwt(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
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
    let hp_heat = query_latest_ebusd(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        "ebusd/hmu/CurrentYieldPower",
        &start,
        &now,
    )
    .unwrap_or(0.0)
        * 1000.0; // kW → W
    let hp_elec = query_latest_ebusd(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        "ebusd/hmu/RunDataElectricPowerConsumption",
        &start,
        &now,
    )
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

/// Query a single latest value from an ebusd topic.
fn query_latest_ebusd(
    influx_url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    topic: &str,
    start: &chrono::DateTime<chrono::FixedOffset>,
    stop: &chrono::DateTime<chrono::FixedOffset>,
) -> Option<f64> {
    let flux = format!(
        "from(bucket: \"{bucket}\") |> range(start: {start}, stop: {stop}) |> filter(fn: (r) => r.topic == \"{topic}\") |> last() |> keep(columns: [\"_value\"])",
        bucket = bucket,
        start = start.to_rfc3339(),
        stop = stop.to_rfc3339(),
        topic = topic,
    );
    let rows = influx::query_flux_csv_pub(influx_url, org, token, &flux).ok()?;
    rows.last()?.get("_value")?.parse().ok()
}
