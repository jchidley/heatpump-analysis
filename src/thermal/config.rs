use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::error::{ThermalError, ThermalResult};

#[derive(Debug, Deserialize)]
pub(crate) struct ThermalConfig {
    pub influx: InfluxCfg,
    #[serde(default)]
    pub postgres: Option<PostgresCfg>,
    pub test_nights: TestNights,
    pub objective: ObjectiveCfg,
    pub priors: PriorsCfg,
    pub bounds: BoundsCfg,
    #[serde(default)]
    pub wind: WindCfg,
    #[serde(default)]
    pub validation: ValidationCfg,
    #[serde(default)]
    pub fit_diagnostics: FitDiagnosticsCfg,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InfluxCfg {
    pub url: String,
    pub org: String,
    pub bucket: String,
    pub token_env: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PostgresCfg {
    pub conninfo_env: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TestNights {
    pub night1_start: String,
    pub night1_end: String,
    pub night2_start: String,
    pub night2_end: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ObjectiveCfg {
    #[serde(default)]
    pub exclude_rooms: Vec<String>,
    #[serde(default)]
    pub prior_weight: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PriorsCfg {
    pub landing_ach: f64,
    pub doorway_cd: f64,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct WindCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub ach_per_ms: f64,
    #[serde(default = "default_wind_max_multiplier")]
    pub max_multiplier: f64,
}

fn default_wind_max_multiplier() -> f64 {
    2.5
}

#[derive(Debug, Deserialize)]
pub(crate) struct BoundsCfg {
    pub leather_ach_min: f64,
    pub leather_ach_max: f64,
    pub leather_ach_step: f64,

    pub landing_ach_min: f64,
    pub landing_ach_max: f64,
    pub landing_ach_step: f64,

    pub conservatory_ach_min: f64,
    pub conservatory_ach_max: f64,
    pub conservatory_ach_step: f64,

    pub office_ach_min: f64,
    pub office_ach_max: f64,
    pub office_ach_step: f64,

    pub doorway_cd_min: f64,
    pub doorway_cd_max: f64,
    pub doorway_cd_step: f64,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ValidationCfg {
    #[serde(default)]
    pub windows: Vec<ValidationWindowCfg>,
    #[serde(default)]
    pub thresholds: ValidationThresholdsCfg,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidationWindowCfg {
    pub name: String,
    pub start: String,
    pub end: String,
    #[serde(default = "default_door_state")]
    pub door_state: String,
}

fn default_door_state() -> String {
    "normal".to_string()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidationThresholdsCfg {
    #[serde(default = "default_rmse_max")]
    pub rmse_max: f64,
    #[serde(default = "default_bias_abs_max")]
    pub bias_abs_max: f64,
    #[serde(default = "default_within_1c_min")]
    pub within_1c_min: f64,
}

impl Default for ValidationThresholdsCfg {
    fn default() -> Self {
        Self {
            rmse_max: default_rmse_max(),
            bias_abs_max: default_bias_abs_max(),
            within_1c_min: default_within_1c_min(),
        }
    }
}

fn default_rmse_max() -> f64 {
    0.5
}
fn default_bias_abs_max() -> f64 {
    0.3
}
fn default_within_1c_min() -> f64 {
    0.8
}

#[derive(Debug, Deserialize)]
pub(crate) struct FitDiagnosticsCfg {
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default = "default_door_state")]
    pub door_state: String,
    #[serde(default = "default_fit_min_period_hours")]
    pub min_period_hours: f64,
    #[serde(default = "default_fit_min_record_hours")]
    pub min_record_hours: f64,
    #[serde(default = "default_fit_min_meas_cooling")]
    pub min_meas_cooling: f64,
    #[serde(default = "default_fit_ratio_min_meas")]
    pub ratio_min_meas: f64,
    #[serde(default = "default_off_codes")]
    pub heating_off_codes: Vec<i32>,
}

impl Default for FitDiagnosticsCfg {
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            door_state: default_door_state(),
            min_period_hours: default_fit_min_period_hours(),
            min_record_hours: default_fit_min_record_hours(),
            min_meas_cooling: default_fit_min_meas_cooling(),
            ratio_min_meas: default_fit_ratio_min_meas(),
            heating_off_codes: default_off_codes(),
        }
    }
}

fn default_fit_min_period_hours() -> f64 {
    2.0
}
fn default_fit_min_record_hours() -> f64 {
    1.0
}
fn default_fit_min_meas_cooling() -> f64 {
    0.03
}
fn default_fit_ratio_min_meas() -> f64 {
    0.03
}
fn default_off_codes() -> Vec<i32> {
    vec![100, 101, 102, 103, 104, 105, 106, 107, 108, 110, 134]
}

/// Resolve the InfluxDB token from the environment variable named in config.
pub(crate) fn resolve_influx_token(cfg: &ThermalConfig) -> ThermalResult<String> {
    std::env::var(&cfg.influx.token_env)
        .map_err(|_| ThermalError::MissingEnv(cfg.influx.token_env.clone()))
}

/// Resolve optional PostgreSQL conninfo from the environment variable named in config.
pub(crate) fn resolve_postgres_conninfo(cfg: &ThermalConfig) -> ThermalResult<Option<String>> {
    let Some(pg) = &cfg.postgres else {
        return Ok(None);
    };
    std::env::var(&pg.conninfo_env)
        .map(Some)
        .map_err(|_| ThermalError::MissingEnv(pg.conninfo_env.clone()))
}

pub(crate) fn load_thermal_config(config_path: &Path) -> ThermalResult<(String, ThermalConfig)> {
    let cfg_txt = fs::read_to_string(config_path).map_err(|source| ThermalError::ConfigRead {
        path: config_path.display().to_string(),
        source,
    })?;
    let cfg: ThermalConfig =
        toml::from_str(&cfg_txt).map_err(|source| ThermalError::ConfigParse {
            path: config_path.display().to_string(),
            source,
        })?;
    Ok((cfg_txt, cfg))
}
