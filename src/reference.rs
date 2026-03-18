//! Reference data from planning workbook and manufacturer specs.
//!
//! Static data for comparing actual heat pump performance against:
//! - Design heat loss calculations
//! - Manufacturer COP curve (Arotherm 5kW)
//! - Pre-HP gas consumption (Octopus billing data)
//!
//! Source: "Heating needs for the house.xlsx"

/// House thermal properties.
#[allow(dead_code)]
pub mod house {
    /// Heat Transfer Coefficient (W/°C) — total heat loss per degree of
    /// temperature difference between inside and outside.
    /// From regression on gas consumption data (EGWU weather station).
    pub const HTC_W_PER_C: f64 = 261.0;

    /// Floor area in m².
    pub const FLOOR_AREA_M2: f64 = 180.0;

    /// Design indoor temperature (°C) — whole-house average.
    pub const DESIGN_INDOOR_TEMP: f64 = 17.0;

    /// Design outdoor temperature (°C) — coldest expected.
    pub const DESIGN_OUTDOOR_TEMP: f64 = -2.0;

    /// Base temperature (°C) from gas consumption regression.
    /// Below this outside temp, heating is needed.
    /// Note: HP data suggests effective base ~12°C — difference may be due to
    /// boiler inefficiency inflating apparent gas-era demand, or heat pump
    /// providing less waste heat than gas boiler.
    pub const BASE_TEMP_GAS_ERA: f64 = 17.0;

    /// Design heat loss at design conditions (W).
    /// = HTC × (design_indoor - design_outdoor) = 261 × 19 = 4,959W
    pub const DESIGN_HEAT_LOSS_W: f64 = 4962.0;

    /// Ventilation heat loss at design conditions (W).
    pub const VENTILATION_LOSS_W: f64 = 3762.0;

    /// Figure of merit: kWh per m² per year (pre-insulation).
    /// AECB retrofit standard target is < 50.
    pub const KWH_PER_M2_YEAR: f64 = 70.3;

    /// kWh per heating degree day (from gas data, base 17°C).
    pub const KWH_PER_HDD: f64 = 6.27;

    /// Construction notes.
    pub const CONSTRUCTION: &str = "\
        Solid brick construction including internal walls. \
        Top floor (Aldora, Elvina, Shower) built to 2010 standards. \
        Solid wall insulation not yet installed (planned).";

    /// U-values for building elements (W/m²K).
    pub const U_VALUES: &[(&str, f64)] = &[
        ("New insulated wall", 0.15),
        ("Old solid wall", 2.11),
        ("Roof", 0.066),
        ("Ceiling (internal)", 0.44),
        ("Floor", 0.70),
        ("Window (single)", 4.80),
        ("Window (double, old)", 1.90),
        ("Window (double, new)", 1.20),
        ("Window (triple)", 1.00),
    ];
}

#[allow(dead_code)]
/// Arotherm Plus 5kW manufacturer performance data.
/// Source: arotherm-plus-spec-sheet-1892564.pdf, reproduced in workbook.
/// Conditions: outside air temp -3°C.
pub mod arotherm {
    /// (flow_temp_°C, heat_output_W, COP) at -3°C outside.
    pub const SPEC_AT_MINUS3: &[(f64, f64, f64)] = &[
        (55.0, 5800.0, 3.06),
        (50.0, 5900.0, 3.41),
        (45.0, 6100.0, 3.77),
        (40.0, 6400.0, 4.13),
        (35.0, 6800.0, 4.48),
    ];

    /// Interpolate expected COP for a given flow temperature (at -3°C outside).
    pub fn expected_cop_at_flow_temp(flow_t: f64) -> Option<f64> {
        let data = SPEC_AT_MINUS3;
        if flow_t >= data[0].0 {
            return Some(data[0].2);
        }
        if flow_t <= data[data.len() - 1].0 {
            return Some(data[data.len() - 1].2);
        }
        // Data is sorted descending by flow temp
        for i in 0..data.len() - 1 {
            let (t1, _, c1) = data[i];
            let (t2, _, c2) = data[i + 1];
            if flow_t <= t1 && flow_t >= t2 {
                let frac = (t1 - flow_t) / (t1 - t2);
                return Some(c1 + (c2 - c1) * frac);
            }
        }
        None
    }
}

/// Radiator inventory — all 15 radiators in the house.
#[allow(dead_code)]
pub mod radiators {
    pub struct Radiator {
        pub room: &'static str,
        pub number: u8,
        pub width_mm: u16,
        pub height_mm: u16,
        pub rad_type: &'static str,
        pub t50_watts: u16, // rated output at ΔT50
        pub model: &'static str,
        pub target_room_temp: f64,
    }

    /// Correction factor for radiator output at a given delta T vs rated ΔT50.
    /// Formula: (actual_dt / 50) ^ 1.3
    /// where actual_dt = mean_water_temp - room_temp
    ///       mean_water_temp = (flow + return) / 2
    ///
    /// For Arotherm 5kW at 860 L/h, DT ≈ heat_output / (flow_rate × SHC).
    /// At typical conditions: DT ~2.5-5°C, so mean water temp is ~2°C below flow.
    pub fn correction_factor(flow_temp: f64, return_temp: f64, room_temp: f64) -> f64 {
        let mean_water_temp = (flow_temp + return_temp) / 2.0;
        let actual_dt = mean_water_temp - room_temp;
        if actual_dt <= 0.0 {
            return 0.0;
        }
        (actual_dt / 50.0_f64).powf(1.3)
    }

    /// Calculate total radiator output at a given flow temperature.
    /// Uses the Arotherm 5kW typical DT (flow - return) to estimate return temp.
    /// DT varies with load but ~3°C is typical at moderate output.
    pub fn total_output_at_flow_temp(flow_temp: f64) -> f64 {
        // Estimate return temp from typical DT at this flow temp.
        // Higher flow temps → larger DT (more heat extracted).
        // From real data: DT ~2.5 at 25°C flow, ~3.5 at 30°C, ~5 at 35°C.
        let estimated_dt = 1.5 + (flow_temp - 20.0) * 0.15;
        let return_temp = flow_temp - estimated_dt.max(1.0);

        ALL.iter()
            .map(|r| {
                let cf = correction_factor(flow_temp, return_temp, r.target_room_temp);
                r.t50_watts as f64 * cf
            })
            .sum()
    }

    pub const ALL: &[Radiator] = &[
        Radiator { room: "Aldora", number: 1, width_mm: 500, height_mm: 900, rad_type: "Towel", t50_watts: 376, model: "Stelrad Classic Mini Towel Rail", target_room_temp: 18.0 },
        Radiator { room: "Bathroom", number: 2, width_mm: 1200, height_mm: 600, rad_type: "Towel", t50_watts: 382, model: "Stelrad Slimline Towelrail Chrome", target_room_temp: 18.0 },
        Radiator { room: "Bathroom", number: 1, width_mm: 1800, height_mm: 600, rad_type: "Towel", t50_watts: 614, model: "Stelrad Slimline Towelrail Chrome", target_room_temp: 18.0 },
        Radiator { room: "Carol & Jack", number: 1, width_mm: 1200, height_mm: 600, rad_type: "DP DF", t50_watts: 1950, model: "Stelrad Compact K2", target_room_temp: 18.0 },
        Radiator { room: "Conservatory", number: 2, width_mm: 1200, height_mm: 600, rad_type: "TP TF", t50_watts: 2867, model: "Stelrad Compact K3", target_room_temp: 18.0 },
        Radiator { room: "Conservatory", number: 1, width_mm: 2000, height_mm: 300, rad_type: "TP TF", t50_watts: 2833, model: "Stelrad Vita Compact K3", target_room_temp: 18.0 },
        Radiator { room: "Elvina", number: 1, width_mm: 500, height_mm: 600, rad_type: "DP DF", t50_watts: 909, model: "Stelrad Slimline K2", target_room_temp: 18.0 },
        Radiator { room: "Front", number: 1, width_mm: 1400, height_mm: 600, rad_type: "DP DF", t50_watts: 2425, model: "Stelrad Slimline K2", target_room_temp: 21.0 },
        Radiator { room: "Front", number: 2, width_mm: 600, height_mm: 1800, rad_type: "DP DF", t50_watts: 2376, model: "Stelrad Compact Vertical K2", target_room_temp: 21.0 },
        Radiator { room: "Hall", number: 1, width_mm: 600, height_mm: 1800, rad_type: "DP DF", t50_watts: 2376, model: "Stelrad Compact Vertical K2", target_room_temp: 18.0 },
        Radiator { room: "Leather", number: 1, width_mm: 600, height_mm: 1800, rad_type: "DP DF", t50_watts: 2376, model: "Stelrad Compact Vertical K2", target_room_temp: 21.0 },
        Radiator { room: "Leather", number: 2, width_mm: 600, height_mm: 1800, rad_type: "DP DF", t50_watts: 2376, model: "Stelrad Compact Vertical K2", target_room_temp: 21.0 },
        Radiator { room: "Office", number: 1, width_mm: 1000, height_mm: 600, rad_type: "DP SF", t50_watts: 1345, model: "Stelrad Slimline P+", target_room_temp: 18.0 },
        Radiator { room: "Shower", number: 1, width_mm: 500, height_mm: 900, rad_type: "Towel", t50_watts: 752, model: "Stelrad Classic Mini Towel Rail", target_room_temp: 18.0 },
        Radiator { room: "Sterling", number: 1, width_mm: 1170, height_mm: 620, rad_type: "SP SF", t50_watts: 1176, model: "Stelrad Slimline P+", target_room_temp: 18.0 },
    ];
}

/// Pre-heat-pump gas consumption data (Octopus billing).
/// Monthly kWh and HDD from EGWU (Northolt) at base 17°C.
pub mod gas_era {
    /// (month_start, hdd_17c, gas_kwh, hot_water_kwh, days)
    /// Hot water estimated at 11.82 kWh/day.
    pub const MONTHLY: &[(&str, f64, f64, f64, u32)] = &[
        ("2021-11", 280.0, 2647.0, 354.6, 30),
        ("2021-12", 289.8, 2949.0, 366.4, 31),
        ("2022-01", 371.1, 2952.0, 366.4, 31),
        ("2022-02", 258.1, 2468.0, 331.0, 28),
        ("2022-03", 260.8, 2069.0, 366.4, 31),
        ("2022-04", 204.7, 1449.0, 354.6, 30),
        // Gap: May-Sep 2022 (summer, minimal heating)
        ("2022-11", 207.6, 1796.0, 354.6, 30),
        ("2022-12", 386.9, 3951.0, 366.4, 31),
        ("2023-01", 364.1, 3957.0, 366.4, 31),
        ("2023-02", 295.5, 2973.0, 331.0, 28),
        ("2023-03", 281.6, 3075.0, 366.4, 31),
        ("2023-04", 226.4, 1993.0, 354.6, 30),
        ("2023-05", 119.1, 1263.0, 366.4, 31),
    ];

    /// Gas boiler efficiency estimate.
    pub const BOILER_EFFICIENCY: f64 = 0.90;

    /// Annual gas consumption estimate (kWh).
    pub const ANNUAL_GAS_KWH: f64 = 18_702.0;

    /// Annual heating-only gas consumption after subtracting hot water (kWh).
    pub const ANNUAL_HEATING_GAS_KWH: f64 = 14_052.0;

    /// Annual heating delivered (gas × efficiency) (kWh).
    pub const ANNUAL_HEATING_DELIVERED_KWH: f64 = 12_647.0;
}
