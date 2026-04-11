use std::collections::{BTreeMap, HashMap};

use super::geometry::{Doorway, ExternalElement, InternalConnection, RoomDef, SolarGlazingDef};

/// Per-room energy balance broken down by component (all values in Watts).
/// Positive = heat into room, negative = heat out.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct EnergyBalanceComponents {
    pub external: f64,
    pub ventilation: f64,
    pub radiator: f64,
    pub body: f64,
    pub solar: f64,
    pub dhw: f64,
    pub walls: f64,
    pub doorways: f64,
    pub total: f64,
}

// ---------------------------------------------------------------------------
// Physical constants
// ---------------------------------------------------------------------------

pub(crate) const AIR_DENSITY: f64 = 1.2;
pub(crate) const AIR_CP: f64 = 1005.0;
pub(crate) const VENT_FACTOR: f64 = AIR_DENSITY * AIR_CP / 3600.0;
pub(crate) const GROUND_TEMP_C: f64 = 10.5;
pub(crate) const RAD_EXPONENT: f64 = 1.3;
pub(crate) const U_INTERNAL_WALL: f64 = 2.37;
pub(crate) const DOORWAY_G: f64 = 9.81;

pub(crate) const BODY_HEAT_SLEEPING_W: f64 = 70.0;
pub(crate) const DHW_CYLINDER_UA: f64 = 1.6;
pub(crate) const DHW_CYLINDER_TEMP: f64 = 44.0;
pub(crate) const DHW_PIPE_LOSS_W: f64 = 42.0;
pub(crate) const DHW_SHOWER_W: f64 = 16.0;

/// Convert PV power (W, negative = generating) to SW **vertical** irradiance (W/m²).
pub(crate) const PV_TO_SLOPING_IRRADIANCE: f64 = 0.087;
pub(crate) const SLOPING_TO_VERTICAL_RATIO: f64 = 1.4;

// ---------------------------------------------------------------------------
// Thermal mass functions (kJ/K)
// ---------------------------------------------------------------------------

pub(crate) fn thermal_mass_air(vol_m3: f64) -> f64 {
    1.2 * vol_m3
}
pub(crate) fn thermal_mass_brick_int(area: f64) -> f64 {
    72.0 * area
}
pub(crate) fn thermal_mass_brick_ext(area: f64) -> f64 {
    72.0 * area
}
pub(crate) fn thermal_mass_concrete(area: f64) -> f64 {
    200.0 * area
}
pub(crate) fn thermal_mass_timber_floor(area: f64) -> f64 {
    50.0 * area
}
pub(crate) fn thermal_mass_plaster(area: f64) -> f64 {
    17.0 * area
}
pub(crate) fn thermal_mass_furniture(area: f64) -> f64 {
    15.0 * area
}
pub(crate) fn thermal_mass_timber_stud(area: f64) -> f64 {
    10.0 * area
}

pub(crate) fn estimate_thermal_mass(room: &RoomDef, connections: &[InternalConnection]) -> f64 {
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

/// Compute thermal masses (kJ/K) for all rooms.
pub(crate) fn compute_thermal_masses(
    rooms: &BTreeMap<String, RoomDef>,
    connections: &[InternalConnection],
) -> HashMap<String, f64> {
    rooms
        .iter()
        .map(|(name, room)| (name.clone(), estimate_thermal_mass(room, connections)))
        .collect()
}

// ---------------------------------------------------------------------------
// Heat transfer functions
// ---------------------------------------------------------------------------

pub(crate) fn external_loss(
    elements: &[ExternalElement],
    room_temp: f64,
    outside_temp: f64,
) -> f64 {
    elements
        .iter()
        .map(|e| {
            let ref_temp = if e.to_ground {
                GROUND_TEMP_C
            } else {
                outside_temp
            };
            e.u_value * e.area * (room_temp - ref_temp)
        })
        .sum()
}

pub(crate) fn ventilation_loss(
    ach: f64,
    volume: f64,
    room_temp: f64,
    outside_temp: f64,
    heat_recovery: f64,
    wind_multiplier: f64,
) -> f64 {
    VENT_FACTOR
        * ach
        * wind_multiplier
        * volume
        * (room_temp - outside_temp)
        * (1.0 - heat_recovery)
}

pub(crate) fn wall_conduction(ua: f64, temp_a: f64, temp_b: f64) -> f64 {
    ua * (temp_a - temp_b)
}

pub(crate) fn doorway_exchange(door: &Doorway, temp_a: f64, temp_b: f64, doorway_cd: f64) -> f64 {
    if door.state == "closed" {
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

    let flow =
        (doorway_cd / 3.0) * width * (DOORWAY_G * door.height.powi(3) * dt.abs() / t_mean).sqrt();

    flow * AIR_DENSITY * AIR_CP * dt
}

pub(crate) fn radiator_output(t50: f64, mwt: f64, room_temp: f64) -> f64 {
    let dt = mwt - room_temp;
    if dt <= 0.0 {
        0.0
    } else {
        t50 * (dt / 50.0).powf(RAD_EXPONENT)
    }
}

/// Solar gain through glazing in Watts.
#[allow(dead_code)]
pub(crate) fn solar_gain_full(
    solar: &[SolarGlazingDef],
    sw_vert: f64,
    ne_vert: f64,
    ne_horiz: f64,
) -> f64 {
    solar
        .iter()
        .map(|sg| {
            let irr = match (sg.orientation, sg.tilt) {
                ("SW", "vertical") => sw_vert,
                ("SW", "sloping") => sw_vert * 1.4,
                ("SW", "horizontal") => sw_vert * 1.2,
                ("NE", "horizontal") => ne_horiz,
                ("NE", "vertical") => ne_vert,
                ("NE", "sloping") => ne_vert * 1.4,
                ("SE", "vertical") => (sw_vert + ne_vert) / 2.0,
                ("SE", _) => (sw_vert + ne_vert) / 2.0,
                _ => ne_vert,
            };
            irr * sg.area * sg.g_value * sg.shading
        })
        .sum()
}

pub(crate) fn pv_to_sw_vertical_irradiance(pv_watts: f64) -> f64 {
    let gen = (-pv_watts).max(0.0);
    gen * PV_TO_SLOPING_IRRADIANCE / SLOPING_TO_VERTICAL_RATIO
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub(crate) fn virtual_room_temp(name: &str, all_temps: &HashMap<String, f64>) -> Option<f64> {
    if let Some(t) = all_temps.get(name) {
        return Some(*t);
    }
    if name == "top_landing" {
        match (all_temps.get("landing"), all_temps.get("shower")) {
            (Some(a), Some(b)) => Some((a + b) / 2.0),
            (Some(a), None) => Some(*a),
            (None, Some(b)) => Some(*b),
            _ => None,
        }
    } else {
        None
    }
}

pub(crate) fn doors_all_closed_except_chimney(doors: &[Doorway]) -> Vec<Doorway> {
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

// ---------------------------------------------------------------------------
// Energy balance (cooldown — no radiator, no solar)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn room_energy_balance(
    room: &RoomDef,
    room_temp: f64,
    outside_temp: f64,
    all_temps: &HashMap<String, f64>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    doorway_cd: f64,
    wind_multiplier: f64,
) -> f64 {
    let name = room.name;
    let vol = room.floor_area * room.ceiling_height;

    let q_ext = -external_loss(&room.external_fabric, room_temp, outside_temp);
    let q_vent = -ventilation_loss(
        room.ventilation_ach,
        vol,
        room_temp,
        outside_temp,
        room.heat_recovery,
        wind_multiplier,
    );

    let q_rad = 0.0;
    let q_body = room.overnight_occupants as f64 * BODY_HEAT_SLEEPING_W;
    let q_solar = 0.0;

    let mut q_dhw = 0.0;
    if name == "bathroom" {
        q_dhw = DHW_CYLINDER_UA * (DHW_CYLINDER_TEMP - room_temp).max(0.0)
            + DHW_PIPE_LOSS_W
            + DHW_SHOWER_W;
    }

    let mut q_walls = 0.0;
    for conn in connections {
        if conn.room_a == name {
            if let Some(other_t) = virtual_room_temp(conn.room_b, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        } else if conn.room_b == name {
            if let Some(other_t) = virtual_room_temp(conn.room_a, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        }
    }

    let mut q_doors = 0.0;
    for door in doorways {
        if door.room_a == name {
            if let Some(other_t) = virtual_room_temp(door.room_b, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        } else if door.room_b == name {
            if let Some(other_t) = virtual_room_temp(door.room_a, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        }
    }

    q_ext + q_vent + q_rad + q_body + q_solar + q_dhw + q_walls + q_doors
}

// ---------------------------------------------------------------------------
// Full energy balance (including radiator + solar)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn full_room_energy_balance(
    room: &RoomDef,
    room_temp: f64,
    outside_temp: f64,
    all_temps: &HashMap<String, f64>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    doorway_cd: f64,
    wind_multiplier: f64,
    mwt: f64,
    sleeping: bool,
    sw_vert: f64,
    ne_vert: f64,
    ne_horiz: f64,
) -> f64 {
    let name = room.name;
    let vol = room.floor_area * room.ceiling_height;

    let q_ext = -external_loss(&room.external_fabric, room_temp, outside_temp);
    let q_vent = -ventilation_loss(
        room.ventilation_ach,
        vol,
        room_temp,
        outside_temp,
        room.heat_recovery,
        wind_multiplier,
    );

    let q_rad = if mwt > 0.0 {
        room.radiators
            .iter()
            .filter(|r| r.active)
            .map(|r| radiator_output(r.t50, mwt, room_temp))
            .sum::<f64>()
    } else {
        0.0
    };

    let body_rate = if sleeping {
        BODY_HEAT_SLEEPING_W
    } else {
        100.0
    };
    let q_body = room.overnight_occupants as f64 * body_rate;
    let q_solar = solar_gain_full(&room.solar, sw_vert, ne_vert, ne_horiz);

    let mut q_dhw = 0.0;
    if name == "bathroom" {
        q_dhw = DHW_CYLINDER_UA * (DHW_CYLINDER_TEMP - room_temp).max(0.0)
            + DHW_PIPE_LOSS_W
            + DHW_SHOWER_W;
    }

    let mut q_walls = 0.0;
    for conn in connections {
        if conn.room_a == name {
            if let Some(other_t) = virtual_room_temp(conn.room_b, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        } else if conn.room_b == name {
            if let Some(other_t) = virtual_room_temp(conn.room_a, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        }
    }

    let mut q_doors = 0.0;
    for door in doorways {
        if door.room_a == name {
            if let Some(other_t) = virtual_room_temp(door.room_b, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        } else if door.room_b == name {
            if let Some(other_t) = virtual_room_temp(door.room_a, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        }
    }

    q_ext + q_vent + q_rad + q_body + q_solar + q_dhw + q_walls + q_doors
}

/// Like `full_room_energy_balance` but returns individual components.
#[allow(clippy::too_many_arguments)]
pub(crate) fn full_room_energy_balance_components(
    room: &RoomDef,
    room_temp: f64,
    outside_temp: f64,
    all_temps: &HashMap<String, f64>,
    connections: &[InternalConnection],
    doorways: &[Doorway],
    doorway_cd: f64,
    wind_multiplier: f64,
    mwt: f64,
    sleeping: bool,
    sw_vert: f64,
    ne_vert: f64,
    ne_horiz: f64,
) -> EnergyBalanceComponents {
    let name = room.name;
    let vol = room.floor_area * room.ceiling_height;

    let q_ext = -external_loss(&room.external_fabric, room_temp, outside_temp);
    let q_vent = -ventilation_loss(
        room.ventilation_ach,
        vol,
        room_temp,
        outside_temp,
        room.heat_recovery,
        wind_multiplier,
    );

    let q_rad = if mwt > 0.0 {
        room.radiators
            .iter()
            .filter(|r| r.active)
            .map(|r| radiator_output(r.t50, mwt, room_temp))
            .sum::<f64>()
    } else {
        0.0
    };

    let body_rate = if sleeping {
        BODY_HEAT_SLEEPING_W
    } else {
        100.0
    };
    let q_body = room.overnight_occupants as f64 * body_rate;
    let q_solar = solar_gain_full(&room.solar, sw_vert, ne_vert, ne_horiz);

    let mut q_dhw = 0.0;
    if name == "bathroom" {
        q_dhw = DHW_CYLINDER_UA * (DHW_CYLINDER_TEMP - room_temp).max(0.0)
            + DHW_PIPE_LOSS_W
            + DHW_SHOWER_W;
    }

    let mut q_walls = 0.0;
    for conn in connections {
        if conn.room_a == name {
            if let Some(other_t) = virtual_room_temp(conn.room_b, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        } else if conn.room_b == name {
            if let Some(other_t) = virtual_room_temp(conn.room_a, all_temps) {
                q_walls -= wall_conduction(conn.ua, room_temp, other_t);
            }
        }
    }

    let mut q_doors = 0.0;
    for door in doorways {
        if door.room_a == name {
            if let Some(other_t) = virtual_room_temp(door.room_b, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        } else if door.room_b == name {
            if let Some(other_t) = virtual_room_temp(door.room_a, all_temps) {
                q_doors -= doorway_exchange(door, room_temp, other_t, doorway_cd);
            }
        }
    }

    let total = q_ext + q_vent + q_rad + q_body + q_solar + q_dhw + q_walls + q_doors;
    EnergyBalanceComponents {
        external: q_ext,
        ventilation: q_vent,
        radiator: q_rad,
        body: q_body,
        solar: q_solar,
        dhw: q_dhw,
        walls: q_walls,
        doorways: q_doors,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thermal::geometry::RadiatorDef;
    use proptest::prelude::*;

    fn test_room(name: &'static str) -> RoomDef {
        RoomDef {
            name,
            floor: "Gnd",
            floor_area: 12.0,
            ceiling_height: 2.4,
            construction: "brick",
            radiators: vec![],
            external_fabric: vec![ExternalElement {
                description: "wall",
                area: 8.0,
                u_value: 0.5,
                to_ground: false,
            }],
            solar: vec![],
            sensor_topic: "test/topic",
            ventilation_ach: 0.5,
            heat_recovery: 0.0,
            overnight_occupants: 1,
        }
    }

    // @lat: [[tests#Thermal physics primitives#Doorway exchange scales with opening state]]
    #[test]
    fn doorway_exchange_respects_opening_state() {
        let closed = Doorway {
            room_a: "a",
            room_b: "b",
            width: 1.0,
            height: 2.0,
            state: "closed",
        };
        let partial = Doorway {
            state: "partial",
            ..closed.clone()
        };
        let open = Doorway {
            state: "open",
            ..closed.clone()
        };

        let closed_q = doorway_exchange(&closed, 22.0, 18.0, 0.2);
        let partial_q = doorway_exchange(&partial, 22.0, 18.0, 0.2);
        let open_q = doorway_exchange(&open, 22.0, 18.0, 0.2);

        assert_eq!(closed_q, 0.0);
        assert!(partial_q.abs() > 0.0);
        assert!(open_q.abs() > partial_q.abs());
    }

    // @lat: [[tests#Thermal physics primitives#Top landing falls back to adjacent sensors]]
    #[test]
    fn virtual_top_landing_prefers_direct_then_adjacent_sensors() {
        let mut temps = HashMap::new();
        temps.insert("landing".to_string(), 19.0);
        temps.insert("shower".to_string(), 21.0);

        assert_eq!(virtual_room_temp("top_landing", &temps), Some(20.0));

        temps.remove("shower");
        assert_eq!(virtual_room_temp("top_landing", &temps), Some(19.0));

        temps.insert("top_landing".to_string(), 23.5);
        assert_eq!(virtual_room_temp("top_landing", &temps), Some(23.5));
        assert_eq!(virtual_room_temp("missing", &temps), None);
    }

    // @lat: [[tests#Thermal physics primitives#Energy balance breakdown matches scalar helper]]
    #[test]
    fn energy_balance_components_match_scalar_helper_and_ignore_inactive_radiators() {
        let mut room = test_room("bathroom");
        room.radiators = vec![
            RadiatorDef {
                t50: 1500.0,
                active: true,
                pipe: "flow",
            },
            RadiatorDef {
                t50: 900.0,
                active: false,
                pipe: "return",
            },
        ];
        room.solar = vec![SolarGlazingDef {
            area: 3.0,
            orientation: "SE",
            tilt: "vertical",
            g_value: 0.5,
            shading: 0.8,
        }];

        let connections = vec![InternalConnection {
            room_a: "bathroom",
            room_b: "bedroom",
            ua: 12.0,
            description: "party wall",
        }];
        let doorways = vec![Doorway {
            room_a: "bathroom",
            room_b: "top_landing",
            width: 0.9,
            height: 2.0,
            state: "open",
        }];
        let all_temps = HashMap::from([
            ("bedroom".to_string(), 18.0),
            ("landing".to_string(), 17.0),
            ("shower".to_string(), 19.0),
        ]);

        let room_temp = 20.0;
        let outside_temp = 5.0;
        let mwt = 45.0;
        let sleeping = false;
        let sw_vert = 300.0;
        let ne_vert = 100.0;
        let ne_horiz = 80.0;
        let doorway_cd = 0.2;
        let wind_multiplier = 1.1;

        let scalar = full_room_energy_balance(
            &room,
            room_temp,
            outside_temp,
            &all_temps,
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
        let components = full_room_energy_balance_components(
            &room,
            room_temp,
            outside_temp,
            &all_temps,
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

        let expected_radiator = radiator_output(1500.0, mwt, room_temp);
        let expected_solar = solar_gain_full(&room.solar, sw_vert, ne_vert, ne_horiz);

        assert!((components.radiator - expected_radiator).abs() < 1e-9);
        assert!((components.solar - expected_solar).abs() < 1e-9);
        assert_eq!(components.body, 100.0);
        assert!(components.external < 0.0);
        assert!(components.ventilation < 0.0);
        assert!(components.dhw > 0.0);
        assert!(components.walls < 0.0);
        assert!(components.doorways < 0.0);
        assert!((components.total - scalar).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal physics primitives#Thermal mass primitives scale with area]]
    #[test]
    fn thermal_mass_air_scales_with_volume() {
        assert!((thermal_mass_air(10.0) - 12.0).abs() < 1e-9);
        assert!((thermal_mass_air(0.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_brick_int_scales_with_area() {
        assert!((thermal_mass_brick_int(5.0) - 360.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_brick_ext_scales_with_area() {
        assert!((thermal_mass_brick_ext(5.0) - 360.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_concrete_scales_with_area() {
        assert!((thermal_mass_concrete(3.0) - 600.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_timber_floor_scales_with_area() {
        assert!((thermal_mass_timber_floor(4.0) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_plaster_scales_with_area() {
        assert!((thermal_mass_plaster(10.0) - 170.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_furniture_scales_with_area() {
        assert!((thermal_mass_furniture(10.0) - 150.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_mass_timber_stud_scales_with_area() {
        assert!((thermal_mass_timber_stud(6.0) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn estimate_thermal_mass_brick_ground_floor() {
        let room = test_room("living");
        let connections = vec![InternalConnection {
            room_a: "living",
            room_b: "hall",
            ua: 4.74, // implies area = 4.74 / 2.37 = 2.0
            description: "internal wall",
        }];
        let c = estimate_thermal_mass(&room, &connections);

        // air: 1.2 * 12.0 * 2.4 = 34.56
        // ext wall (brick): 72.0 * 8.0 = 576.0, plaster on ext wall: 17.0 * 8.0 = 136.0
        // int wall (brick): 72.0 * 2.0 = 144.0, plaster on int wall: 17.0 * 2.0 = 34.0
        // concrete floor (Gnd, brick): 200.0 * 12.0 = 2400.0
        // ceiling plaster: 17.0 * 12.0 = 204.0
        // furniture: 15.0 * 12.0 = 180.0
        let expected = 34.56 + 576.0 + 136.0 + 144.0 + 34.0 + 2400.0 + 204.0 + 180.0;
        assert!((c - expected).abs() < 1e-9);
    }

    #[test]
    fn estimate_thermal_mass_timber_upper_floor() {
        let mut room = test_room("bedroom");
        room.construction = "timber";
        room.floor = "1st";
        let c = estimate_thermal_mass(&room, &[]);

        // air: 1.2 * 12.0 * 2.4 = 34.56
        // ext wall (timber_stud): 10.0 * 8.0 = 80.0, plaster: 17.0 * 8.0 = 136.0
        // no internal connections
        // timber floor (not Gnd): 50.0 * 12.0 = 600.0
        // ceiling plaster: 17.0 * 12.0 = 204.0
        // furniture: 15.0 * 12.0 = 180.0
        let expected = 34.56 + 80.0 + 136.0 + 600.0 + 204.0 + 180.0;
        assert!((c - expected).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal physics primitives#External and ventilation loss follow temperature difference]]
    #[test]
    fn external_loss_positive_when_room_warmer() {
        let elements = vec![ExternalElement {
            description: "wall",
            area: 10.0,
            u_value: 0.3,
            to_ground: false,
        }];
        let loss = external_loss(&elements, 20.0, 5.0);
        // 0.3 * 10.0 * (20.0 - 5.0) = 45.0
        assert!((loss - 45.0).abs() < 1e-9);
    }

    #[test]
    fn external_loss_uses_ground_temp_for_ground_elements() {
        let elements = vec![ExternalElement {
            description: "floor",
            area: 12.0,
            u_value: 0.25,
            to_ground: true,
        }];
        let loss = external_loss(&elements, 20.0, 0.0);
        // Uses GROUND_TEMP_C (10.5), not outside_temp (0.0)
        // 0.25 * 12.0 * (20.0 - 10.5) = 28.5
        assert!((loss - 28.5).abs() < 1e-9);
    }

    #[test]
    fn ventilation_loss_basic() {
        let loss = ventilation_loss(0.5, 30.0, 20.0, 5.0, 0.0, 1.0);
        // VENT_FACTOR * 0.5 * 1.0 * 30.0 * 15.0 * 1.0
        let expected = VENT_FACTOR * 0.5 * 30.0 * 15.0;
        assert!((loss - expected).abs() < 1e-9);
    }

    #[test]
    fn ventilation_loss_with_heat_recovery() {
        let no_recovery = ventilation_loss(1.0, 20.0, 20.0, 5.0, 0.0, 1.0);
        let with_recovery = ventilation_loss(1.0, 20.0, 20.0, 5.0, 0.5, 1.0);
        assert!((with_recovery - no_recovery * 0.5).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal physics primitives#Wall conduction is proportional to temperature difference]]
    #[test]
    fn wall_conduction_proportional_to_dt() {
        assert!((wall_conduction(5.0, 20.0, 18.0) - 10.0).abs() < 1e-9);
        assert!((wall_conduction(5.0, 18.0, 20.0) - -10.0).abs() < 1e-9);
        assert!((wall_conduction(5.0, 20.0, 20.0) - 0.0).abs() < 1e-9);
    }

    // @lat: [[tests#Thermal physics primitives#Solar gain follows orientation and PV irradiance conversion]]
    #[test]
    fn solar_gain_full_sw_vertical() {
        let glazing = vec![SolarGlazingDef {
            area: 2.0,
            orientation: "SW",
            tilt: "vertical",
            g_value: 0.7,
            shading: 1.0,
        }];
        let gain = solar_gain_full(&glazing, 200.0, 50.0, 40.0);
        // 200.0 * 2.0 * 0.7 * 1.0 = 280.0
        assert!((gain - 280.0).abs() < 1e-9);
    }

    #[test]
    fn solar_gain_full_ne_vertical_and_empty() {
        let glazing = vec![SolarGlazingDef {
            area: 3.0,
            orientation: "NE",
            tilt: "vertical",
            g_value: 0.5,
            shading: 0.8,
        }];
        let gain = solar_gain_full(&glazing, 200.0, 80.0, 60.0);
        // ne_vert=80, 80 * 3.0 * 0.5 * 0.8 = 96.0
        assert!((gain - 96.0).abs() < 1e-9);

        // Empty glazing returns zero
        assert_eq!(solar_gain_full(&[], 200.0, 80.0, 60.0), 0.0);
    }

    #[test]
    fn pv_to_sw_vertical_irradiance_negative_pv_generates() {
        // pv_watts negative means generation; gen = -pv_watts
        let irr = pv_to_sw_vertical_irradiance(-1000.0);
        let expected = 1000.0 * PV_TO_SLOPING_IRRADIANCE / SLOPING_TO_VERTICAL_RATIO;
        assert!((irr - expected).abs() < 1e-9);
    }

    #[test]
    fn pv_to_sw_vertical_irradiance_positive_pv_returns_zero() {
        // Positive pv_watts means consuming, no generation
        assert_eq!(pv_to_sw_vertical_irradiance(500.0), 0.0);
        assert_eq!(pv_to_sw_vertical_irradiance(0.0), 0.0);
    }

    // @lat: [[tests#Thermal physics primitives#Door state override preserves chimney state]]
    #[test]
    fn doors_all_closed_except_chimney_preserves_chimney() {
        let doors = vec![
            Doorway {
                room_a: "a",
                room_b: "b",
                width: 0.9,
                height: 2.0,
                state: "open",
            },
            Doorway {
                room_a: "b",
                room_b: "c",
                width: 0.8,
                height: 2.0,
                state: "chimney",
            },
            Doorway {
                room_a: "c",
                room_b: "d",
                width: 0.9,
                height: 2.0,
                state: "partial",
            },
        ];
        let result = doors_all_closed_except_chimney(&doors);
        assert_eq!(result[0].state, "closed");
        assert_eq!(result[1].state, "chimney");
        assert_eq!(result[2].state, "closed");
    }

    // @lat: [[tests#Thermal physics primitives#Radiator output regression anchor at dt50]]
    #[test]
    fn radiator_output_regression_anchor_at_dt50() {
        // At dt=50 (the reference point), output should exactly equal t50.
        // This pins the exponent: 1500 * (50/50)^1.3 == 1500.
        let out = radiator_output(1500.0, 70.0, 20.0);
        assert!((out - 1500.0).abs() < 1e-9, "at dt=50, output should equal t50");

        // At dt=25 (half reference), output depends on the exponent.
        // (25/50)^1.3 ≈ 0.406, so 1500 * 0.406 ≈ 609.
        let out_half = radiator_output(1500.0, 45.0, 20.0);
        assert!(
            (out_half - 609.0).abs() < 2.0,
            "at dt=25, expected ~609 W but got {:.1}",
            out_half
        );
    }

    proptest! {
        // @lat: [[tests#Thermal physics primitives#Radiator output is monotonic above room temperature]]
        #[test]
        fn radiator_output_is_monotonic_above_room_temp(
            room_temp in 5.0f64..25.0,
            t50 in 100.0f64..3000.0,
            low_delta in 0.1f64..20.0,
            extra_delta in 0.1f64..20.0,
        ) {
            let low_mwt = room_temp + low_delta;
            let high_mwt = low_mwt + extra_delta;

            let low = radiator_output(t50, low_mwt, room_temp);
            let high = radiator_output(t50, high_mwt, room_temp);

            prop_assert!(high >= low);
            prop_assert!(low > 0.0);
        }

        // @lat: [[tests#Thermal physics primitives#Ventilation loss scales with temperature difference]]
        #[test]
        fn ventilation_loss_scales_with_temperature_difference(
            ach in 0.1f64..2.0,
            volume in 10.0f64..100.0,
            room_temp in 15.0f64..25.0,
            small_dt in 1.0f64..10.0,
            extra_dt in 0.1f64..10.0,
        ) {
            let close_outside = room_temp - small_dt;
            let far_outside = room_temp - small_dt - extra_dt;

            let loss_close = ventilation_loss(ach, volume, room_temp, close_outside, 0.0, 1.0);
            let loss_far = ventilation_loss(ach, volume, room_temp, far_outside, 0.0, 1.0);

            prop_assert!(loss_far > loss_close,
                "larger dT should produce more ventilation loss: {loss_far} vs {loss_close}");
            prop_assert!(loss_close > 0.0);
        }
    }
}
