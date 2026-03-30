use super::error::ThermalResult;
use super::geometry::{build_connections, build_doorways, build_rooms};
use super::physics::estimate_thermal_mass;

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
