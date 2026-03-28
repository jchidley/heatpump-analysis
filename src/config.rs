//! Configuration loaded from `config.toml`.
//!
//! All domain constants, thresholds, and reference data live in `config.toml`
//! at the project root. This module deserializes that file into typed structs
//! and provides a global accessor via [`config()`].

use std::path::Path;

use anyhow::{Context, Result};
use once_cell::sync::OnceCell;
use serde::Deserialize;

/// Global singleton — initialised once by [`load`], then accessed via [`config`].
static CONFIG: OnceCell<Config> = OnceCell::new();

// All config structs are deserialized from TOML — fields may appear "unused"
// from a static-analysis perspective but are accessed at runtime.
#[allow(dead_code)]

/// Top-level configuration.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Config {
    pub emoncms: Emoncms,
    pub thresholds: Thresholds,
    pub house: House,
    pub arotherm: Arotherm,
    pub radiators: Vec<Radiator>,
    pub gas_era: GasEra,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Emoncms {
    pub base_url: String,
    pub default_sync_start_ms: i64,
    pub feeds: Vec<FeedDef>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FeedDef {
    pub id: String,
    pub name: String,
    /// DataFrame column name for analysis (None = feed not used in analysis).
    pub column: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Thresholds {
    pub hdd_base_temp_c: f64,
    pub elec_running_w: f64,
    pub dhw_enter_flow_rate: f64,
    pub dhw_exit_flow_rate: f64,
    pub defrost_dt_threshold: f64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct House {
    pub htc_w_per_c: f64,
    pub floor_area_m2: f64,
    pub design_indoor_temp_c: f64,
    pub design_outdoor_temp_c: f64,
    pub base_temp_gas_era_c: f64,
    pub design_heat_loss_w: f64,
    pub ventilation_loss_w: f64,
    pub kwh_per_m2_year: f64,
    pub kwh_per_hdd: f64,
    pub construction: String,
    pub u_values: Vec<UValue>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct UValue {
    pub element: String,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Arotherm {
    pub spec_at_minus3: Vec<SpecPoint>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SpecPoint {
    pub flow_temp_c: f64,
    pub heat_output_w: f64,
    pub cop: f64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Radiator {
    pub room: String,
    pub number: u8,
    pub width_mm: u16,
    pub height_mm: u16,
    pub rad_type: String,
    pub t50_watts: u16,
    pub model: String,
    pub target_room_temp_c: f64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GasEra {
    pub boiler_efficiency: f64,
    pub annual_gas_kwh: f64,
    pub annual_heating_gas_kwh: f64,
    pub annual_heating_delivered_kwh: f64,
    pub dhw_kwh_per_day: f64,
    pub monthly: Vec<GasMonth>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GasMonth {
    pub month: String,
    pub hdd_17c: f64,
    pub gas_kwh: f64,
    pub hot_water_kwh: f64,
    pub days: u32,
}

// --- Computed helpers (logic that was previously in reference.rs) ---

#[allow(dead_code)]
impl Arotherm {
    /// Interpolate expected COP for a given flow temperature (at -3°C outside).
    pub fn expected_cop_at_flow_temp(&self, flow_t: f64) -> Option<f64> {
        let data = &self.spec_at_minus3;
        if data.is_empty() {
            return None;
        }
        // Data sorted descending by flow temp (55, 50, 45, 40, 35)
        if flow_t >= data[0].flow_temp_c {
            return Some(data[0].cop);
        }
        if flow_t <= data[data.len() - 1].flow_temp_c {
            return Some(data[data.len() - 1].cop);
        }
        for i in 0..data.len() - 1 {
            let (t1, c1) = (data[i].flow_temp_c, data[i].cop);
            let (t2, c2) = (data[i + 1].flow_temp_c, data[i + 1].cop);
            if flow_t <= t1 && flow_t >= t2 {
                let frac = (t1 - flow_t) / (t1 - t2);
                return Some(c1 + (c2 - c1) * frac);
            }
        }
        None
    }
}

/// Correction factor for radiator output at a given delta T vs rated ΔT50.
/// Formula: (actual_dt / 50) ^ 1.3
pub fn radiator_correction_factor(flow_temp: f64, return_temp: f64, room_temp: f64) -> f64 {
    let mean_water_temp = (flow_temp + return_temp) / 2.0;
    let actual_dt = mean_water_temp - room_temp;
    if actual_dt <= 0.0 {
        return 0.0;
    }
    (actual_dt / 50.0_f64).powf(1.3)
}

/// Calculate total radiator output at a given flow temperature.
pub fn total_radiator_output_at_flow_temp(radiators: &[Radiator], flow_temp: f64) -> f64 {
    let estimated_dt = 1.5 + (flow_temp - 20.0) * 0.15;
    let return_temp = flow_temp - estimated_dt.max(1.0);

    radiators
        .iter()
        .map(|r| {
            let cf = radiator_correction_factor(flow_temp, return_temp, r.target_room_temp_c);
            r.t50_watts as f64 * cf
        })
        .sum()
}

/// Load configuration from a TOML file and store it as the global singleton.
pub fn load(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let cfg: Config =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    CONFIG
        .set(cfg)
        .map_err(|_| anyhow::anyhow!("Config already loaded"))?;
    Ok(())
}

/// Get a reference to the loaded config. Panics if [`load`] hasn't been called.
pub fn config() -> &'static Config {
    CONFIG
        .get()
        .expect("config::load() must be called before config::config()")
}

impl Emoncms {
    /// Look up a feed ID by its name. Panics if not found.
    pub fn feed_id(&self, name: &str) -> &str {
        self.feeds
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("Feed '{}' not found in config", name))
            .id
            .as_str()
    }
}
