#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, TimeZone, Timelike, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    #[serde(default)]
    wind: WindCfg,
    #[serde(default)]
    validation: ValidationCfg,
    #[serde(default)]
    fit_diagnostics: FitDiagnosticsCfg,
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

#[derive(Debug, Deserialize, Default)]
struct WindCfg {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    latitude: f64,
    #[serde(default)]
    longitude: f64,
    #[serde(default)]
    ach_per_ms: f64,
    #[serde(default = "default_wind_max_multiplier")]
    max_multiplier: f64,
}

fn default_wind_max_multiplier() -> f64 {
    2.5
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

#[derive(Debug, Deserialize, Default)]
struct ValidationCfg {
    #[serde(default)]
    windows: Vec<ValidationWindowCfg>,
    #[serde(default)]
    thresholds: ValidationThresholdsCfg,
}

#[derive(Debug, Deserialize, Clone)]
struct ValidationWindowCfg {
    name: String,
    start: String,
    end: String,
    #[serde(default = "default_door_state")]
    door_state: String,
}

fn default_door_state() -> String {
    "normal".to_string()
}

#[derive(Debug, Deserialize)]
struct ValidationThresholdsCfg {
    #[serde(default = "default_rmse_max")]
    rmse_max: f64,
    #[serde(default = "default_bias_abs_max")]
    bias_abs_max: f64,
    #[serde(default = "default_within_1c_min")]
    within_1c_min: f64,
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
    0.7
}
fn default_bias_abs_max() -> f64 {
    0.3
}
fn default_within_1c_min() -> f64 {
    0.8
}

#[derive(Debug, Deserialize)]
struct FitDiagnosticsCfg {
    start: Option<String>,
    end: Option<String>,
    #[serde(default = "default_fit_min_period_hours")]
    min_period_hours: f64,
    #[serde(default = "default_fit_min_record_hours")]
    min_record_hours: f64,
    #[serde(default = "default_fit_min_meas_cooling")]
    min_meas_cooling: f64,
    #[serde(default = "default_fit_ratio_min_meas")]
    ratio_min_meas: f64,
    #[serde(default = "default_door_state")]
    door_state: String,
    #[serde(default = "default_off_codes")]
    heating_off_codes: Vec<i32>,
}

impl Default for FitDiagnosticsCfg {
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            min_period_hours: default_fit_min_period_hours(),
            min_record_hours: default_fit_min_record_hours(),
            min_meas_cooling: default_fit_min_meas_cooling(),
            ratio_min_meas: default_fit_ratio_min_meas(),
            door_state: default_door_state(),
            heating_off_codes: default_off_codes(),
        }
    }
}

fn default_fit_min_period_hours() -> f64 {
    0.25
}
fn default_fit_min_record_hours() -> f64 {
    0.25
}
fn default_fit_min_meas_cooling() -> f64 {
    0.03
}
fn default_fit_ratio_min_meas() -> f64 {
    0.01
}
fn default_off_codes() -> Vec<i32> {
    vec![100, 101, 103, 134]
}

#[derive(Debug, Clone)]
struct ParsedWindow {
    name: String,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    door_state: String,
}

#[derive(Debug, Clone)]
struct CalibrationResult {
    final_score: f64,
    base_score: f64,
    leather_ach: f64,
    landing_ach: f64,
    conservatory_ach: f64,
    office_ach: f64,
    doorway_cd: f64,
    pred1: HashMap<String, f64>,
    pred2: HashMap<String, f64>,
    r1: f64,
    r2: f64,
}

#[derive(Debug, Serialize)]
struct RoomResidual {
    room: String,
    measured: f64,
    predicted: f64,
    residual: f64,
    abs_residual: f64,
    thermal_mass_kj_per_k: f64,
}

#[derive(Debug, Serialize, Clone)]
struct RoomHeatError {
    room: String,
    measured_w: f64,
    predicted_w: f64,
    error_w: f64,
}

#[derive(Debug, Serialize, Clone)]
struct WholeHouseMetrics {
    measured_w: f64,
    predicted_w: f64,
    error_w: f64,
    pred_over_meas: f64,
    top_contributors: Vec<RoomHeatError>,
}

#[derive(Debug, Serialize)]
struct Metrics {
    rooms_count: usize,
    rmse: f64,
    mae: f64,
    bias: f64,
    max_abs_error: f64,
    within_0_5c: f64,
    within_1_0c: f64,
}

#[derive(Debug, Serialize)]
struct WindowValidation {
    name: String,
    start: String,
    end: String,
    door_state: String,
    outside_avg_c: f64,
    wind_avg_ms: f64,
    wind_multiplier: f64,
    metrics: Metrics,
    whole_house: WholeHouseMetrics,
    pass: bool,
    residuals: Vec<RoomResidual>,
}

#[derive(Debug, Serialize)]
struct ThresholdResult {
    rmse_max: f64,
    bias_abs_max: f64,
    within_1c_min: f64,
}

#[derive(Debug, Serialize)]
struct ValidationSummary {
    thresholds: ThresholdResult,
    aggregate_metrics: Metrics,
    aggregate_whole_house: WholeHouseMetrics,
    aggregate_pass: bool,
    windows: Vec<WindowValidation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GitMeta {
    sha: Option<String>,
    dirty: bool,
}

#[derive(Debug, Serialize)]
struct CalibrationArtifact {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    config_path: String,
    config_sha256: String,
    git: GitMeta,
    calibration_windows: Vec<ArtifactWindow>,
    calibration: ArtifactCalibration,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<ValidationSummary>,
}

#[derive(Debug, Serialize)]
struct ArtifactWindow {
    name: String,
    start: String,
    end: String,
}

#[derive(Debug, Serialize)]
struct ArtifactCalibration {
    leather_ach: f64,
    landing_ach: f64,
    conservatory_ach: f64,
    office_ach: f64,
    doorway_cd: f64,
    rmse_night1: f64,
    rmse_night2: f64,
    base_score: f64,
    final_score: f64,
    night1: Vec<RoomResidual>,
    night2: Vec<RoomResidual>,
}

#[derive(Debug, Serialize)]
struct FitDiagnosticsArtifact {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    config_path: String,
    config_sha256: String,
    git: GitMeta,
    range_start: String,
    range_end: String,
    door_state: String,
    cooldown_periods: Vec<FitPeriod>,
    records: Vec<FitRecord>,
    summary_all: FitSummary,
    summary_true_cooling: FitSummary,
    per_room_true_cooling: Vec<PerRoomFitSummary>,
    calibrated_params: ArtifactCalibrationParams,
}

#[derive(Debug, Serialize)]
struct ArtifactCalibrationParams {
    leather_ach: f64,
    landing_ach: f64,
    conservatory_ach: f64,
    office_ach: f64,
    doorway_cd: f64,
}

// --- Operational validation structs ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum HpState {
    Heating,
    Dhw,
    Off,
}

impl std::fmt::Display for HpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HpState::Heating => write!(f, "heating"),
            HpState::Dhw => write!(f, "dhw"),
            HpState::Off => write!(f, "off"),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct OperationalRecord {
    room: String,
    period_start: String,
    period_end: String,
    hp_state: String,
    mwt_avg_c: f64,
    outside_avg_c: f64,
    start_temp_c: f64,
    end_temp_c: f64,
    meas_rate_c_per_hr: f64,
    pred_rate_c_per_hr: f64,
    radiator_w: f64,
    loss_w: f64,
}

#[derive(Debug, Serialize)]
struct OperationalSummary {
    n: usize,
    rmse: f64,
    mae: f64,
    bias: f64,
}

#[derive(Debug, Serialize)]
struct PerRoomOperationalSummary {
    room: String,
    n: usize,
    rmse: f64,
    mae: f64,
    bias: f64,
}

#[derive(Debug, Serialize)]
struct OperationalArtifact {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    config_path: String,
    config_sha256: String,
    git: GitMeta,
    range_start: String,
    range_end: String,
    calibrated_params: ArtifactCalibrationParams,
    summary_all: OperationalSummary,
    summary_by_state: Vec<(String, OperationalSummary)>,
    per_room: Vec<PerRoomOperationalSummary>,
    whole_house: WholeHouseMetrics,
    records: Vec<OperationalRecord>,
}

#[derive(Debug, Serialize)]
struct FitPeriod {
    start: String,
    end: String,
    hours: f64,
}

#[derive(Debug, Serialize, Clone)]
struct FitRecord {
    room: String,
    period_start: String,
    period_end: String,
    start_temp_c: f64,
    end_temp_c: f64,
    meas_rate_c_per_hr: f64,
    pred_rate_c_per_hr: f64,
    ratio_pred_over_meas: Option<f64>,
    body_w: f64,
    true_cooling: bool,
}

#[derive(Debug, Serialize)]
struct FitSummary {
    n: usize,
    rmse: f64,
    mae: f64,
    med_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct PerRoomFitSummary {
    room: String,
    n: usize,
    rmse: f64,
    mae: f64,
    med_ratio: Option<f64>,
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

#[derive(Clone)]
struct SolarGlazingDef {
    area: f64,
    orientation: &'static str, // "SW" or "NE"
    tilt: &'static str,        // "vertical", "sloping", "horizontal"
    g_value: f64,
    shading: f64,
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
    solar: Vec<SolarGlazingDef>,
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

fn thermal_mass_air(vol_m3: f64) -> f64 {
    1.2 * vol_m3
}
fn thermal_mass_brick_int(area: f64) -> f64 {
    72.0 * area
}
fn thermal_mass_brick_ext(area: f64) -> f64 {
    72.0 * area
}
fn thermal_mass_concrete(area: f64) -> f64 {
    200.0 * area
}
fn thermal_mass_timber_floor(area: f64) -> f64 {
    50.0 * area
}
fn thermal_mass_plaster(area: f64) -> f64 {
    17.0 * area
}
fn thermal_mass_furniture(area: f64) -> f64 {
    15.0 * area
}
fn thermal_mass_timber_stud(area: f64) -> f64 {
    10.0 * area
}

#[derive(Debug, Deserialize)]
struct GeometryFile {
    rooms: Vec<GeometryRoom>,
    connections: Vec<GeometryConnection>,
    doorways: Vec<GeometryDoorway>,
}

#[derive(Debug, Deserialize)]
struct GeometrySolarGlazing {
    area: f64,
    orientation: String,
    #[serde(default = "default_vertical")]
    tilt: String,
    #[serde(default = "default_g_value")]
    g_value: f64,
    #[serde(default = "default_shading")]
    shading: f64,
}

fn default_vertical() -> String {
    "vertical".to_string()
}
fn default_g_value() -> f64 {
    0.7
}
fn default_shading() -> f64 {
    1.0
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
    #[serde(default)]
    solar: Vec<GeometrySolarGlazing>,
}

#[derive(Debug, Deserialize)]
struct GeometryRadiator {
    t50: f64,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_true() -> bool {
    true
}

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

#[derive(Debug, Deserialize)]
struct OpenMeteoHourly {
    time: Vec<String>,
    wind_speed_10m: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    hourly: OpenMeteoHourly,
}

fn fetch_open_meteo_wind(
    latitude: f64,
    longitude: f64,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<(DateTime<FixedOffset>, f64)> {
    let start_date = start.date_naive().to_string();
    let end_date = end.date_naive().to_string();
    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={latitude}&longitude={longitude}&start_date={start_date}&end_date={end_date}&hourly=wind_speed_10m&wind_speed_unit=ms&timezone=UTC"
    );

    let resp = match Client::new().get(&url).send() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let parsed: OpenMeteoResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    parsed
        .hourly
        .time
        .iter()
        .zip(parsed.hourly.wind_speed_10m.iter())
        .filter_map(|(t, v)| {
            let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()?;
            let utc = Utc.from_utc_datetime(&naive);
            let fixed: DateTime<FixedOffset> = utc.with_timezone(&FixedOffset::east_opt(0)?);
            Some((fixed, *v))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Solar geometry: compute irradiance on any oriented surface from DNI + DHI
// ---------------------------------------------------------------------------

/// Solar position (altitude and azimuth) from date/time and location.
/// Uses simplified Spencer (1971) equations — accurate to ~1° for our purposes.
/// Returns (altitude_rad, azimuth_rad) where azimuth is clockwise from north.
fn solar_position(dt: DateTime<FixedOffset>, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
    let lat = lat_deg.to_radians();

    // Day of year
    let doy = dt.ordinal() as f64;
    // Fractional hour (UTC)
    let hour_utc = dt.hour() as f64 + dt.minute() as f64 / 60.0;

    // Equation of time (minutes) — Spencer approximation
    let b = (360.0_f64 / 365.0 * (doy - 81.0)).to_radians();
    let eot = 9.87 * (2.0 * b).sin() - 7.53 * b.cos() - 1.5 * b.sin();

    // Solar declination (radians)
    let decl = 23.45_f64.to_radians() * (360.0_f64 / 365.0 * (doy + 284.0)).to_radians().sin();

    // Solar time
    let solar_time = hour_utc + (lon_deg / 15.0) + (eot / 60.0);
    let hour_angle = ((solar_time - 12.0) * 15.0).to_radians();

    // Altitude
    let sin_alt = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos();
    let altitude = sin_alt.asin();

    // Azimuth (clockwise from north)
    let cos_az = if altitude.cos().abs() > 1e-10 {
        (decl.sin() - altitude.sin() * lat.sin()) / (altitude.cos() * lat.cos())
    } else {
        0.0
    };
    let mut azimuth = cos_az.clamp(-1.0, 1.0).acos();
    if hour_angle > 0.0 {
        azimuth = std::f64::consts::TAU - azimuth; // afternoon: azimuth > 180°
    }

    (altitude, azimuth)
}

/// Irradiance on a surface with given tilt and azimuth from DNI + DHI.
/// surface_tilt: 0 = horizontal, π/2 = vertical.
/// surface_azimuth: clockwise from north (e.g., SW = 225°, NE = 45°).
/// Returns W/m² on the surface.
fn surface_irradiance(
    dni: f64,
    dhi: f64,
    solar_altitude: f64,
    solar_azimuth: f64,
    surface_tilt: f64,
    surface_azimuth: f64,
) -> f64 {
    if solar_altitude <= 0.0 {
        return 0.0; // Sun below horizon
    }

    // Angle of incidence on tilted surface
    let cos_aoi = solar_altitude.sin() * surface_tilt.cos()
        + solar_altitude.cos() * surface_tilt.sin() * (solar_azimuth - surface_azimuth).cos();
    let cos_aoi = cos_aoi.max(0.0); // Surface facing away from sun

    // Direct component on surface
    let direct = dni * cos_aoi;

    // Diffuse: isotropic sky model (Perez is better but overkill here)
    // Sky view factor for tilted surface = (1 + cos(tilt)) / 2
    let svf = (1.0 + surface_tilt.cos()) / 2.0;
    let diffuse = dhi * svf;

    direct + diffuse
}

/// Surface azimuth constants (clockwise from north, radians).
const AZ_SW: f64 = 225.0 * std::f64::consts::PI / 180.0;
const AZ_NE: f64 = 45.0 * std::f64::consts::PI / 180.0;
const AZ_SE: f64 = 135.0 * std::f64::consts::PI / 180.0;
const TILT_VERTICAL: f64 = std::f64::consts::FRAC_PI_2;
const TILT_HORIZONTAL: f64 = 0.0;
/// Sloping roof tilt (45°) — used by future per-surface irradiance refinements.
#[allow(dead_code)]
const TILT_SLOPING_45: f64 = std::f64::consts::FRAC_PI_4;

/// Hourly solar data from Open-Meteo: (time, sw_vertical, ne_vertical, ne_horizontal)
/// SW vertical is computed but only used as fallback when PV data is unavailable.
struct HourlySolarIrradiance {
    time: DateTime<FixedOffset>,
    sw_vertical: f64,
    ne_vertical: f64,
    ne_horizontal: f64,
    se_vertical: f64, // For future SE sensor array
}

/// Fetch hourly DNI + DHI from Open-Meteo and compute irradiance on SW/NE/SE surfaces.
fn fetch_surface_irradiance(
    latitude: f64,
    longitude: f64,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<HourlySolarIrradiance> {
    let start_date = start.date_naive().to_string();
    let end_date = end.date_naive().to_string();
    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={latitude}&longitude={longitude}&start_date={start_date}&end_date={end_date}&hourly=direct_normal_irradiance,diffuse_radiation&timezone=UTC"
    );

    let resp = match Client::new().get(&url).send() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    #[derive(Debug, Deserialize)]
    struct SolarHourly {
        time: Vec<String>,
        direct_normal_irradiance: Vec<f64>,
        diffuse_radiation: Vec<f64>,
    }
    #[derive(Debug, Deserialize)]
    struct SolarResponse {
        hourly: SolarHourly,
    }

    let parsed: SolarResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    parsed
        .hourly
        .time
        .iter()
        .zip(
            parsed
                .hourly
                .direct_normal_irradiance
                .iter()
                .zip(parsed.hourly.diffuse_radiation.iter()),
        )
        .filter_map(|(t, (dni, dhi))| {
            let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()?;
            let utc = Utc.from_utc_datetime(&naive);
            let fixed: DateTime<FixedOffset> = utc.with_timezone(&FixedOffset::east_opt(0)?);

            let (alt, az) = solar_position(fixed, latitude, longitude);

            let sw_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_SW);
            let ne_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_NE);
            let ne_h = surface_irradiance(*dni, *dhi, alt, az, TILT_HORIZONTAL, AZ_NE);
            let se_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_SE);

            Some(HourlySolarIrradiance {
                time: fixed,
                sw_vertical: sw_v,
                ne_vertical: ne_v,
                ne_horizontal: ne_h,
                se_vertical: se_v,
            })
        })
        .collect()
}

/// Interpolate surface irradiance for a time window from hourly data.
fn avg_irradiance_in_window(
    solar: &[HourlySolarIrradiance],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> (f64, f64, f64, f64) {
    let in_window: Vec<&HourlySolarIrradiance> = solar
        .iter()
        .filter(|s| s.time >= start && s.time <= end)
        .collect();

    if in_window.is_empty() {
        // Nearest point fallback
        if let Some(nearest) = solar.iter().min_by_key(|s| {
            let mid = start + (end - start) / 2;
            (s.time - mid).num_seconds().unsigned_abs()
        }) {
            return (
                nearest.sw_vertical,
                nearest.ne_vertical,
                nearest.ne_horizontal,
                nearest.se_vertical,
            );
        }
        return (0.0, 0.0, 0.0, 0.0);
    }

    let n = in_window.len() as f64;
    (
        in_window.iter().map(|s| s.sw_vertical).sum::<f64>() / n,
        in_window.iter().map(|s| s.ne_vertical).sum::<f64>() / n,
        in_window.iter().map(|s| s.ne_horizontal).sum::<f64>() / n,
        in_window.iter().map(|s| s.se_vertical).sum::<f64>() / n,
    )
}

fn wind_multiplier_for_window(
    wind_cfg: &WindCfg,
    wind_points: &[(DateTime<FixedOffset>, f64)],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> (f64, f64) {
    if !wind_cfg.enabled {
        return (1.0, 0.0);
    }
    let vals: Vec<f64> = wind_points
        .iter()
        .filter(|(t, _)| *t >= start && *t <= end)
        .map(|(_, v)| *v)
        .collect();

    if vals.is_empty() {
        return (1.0, 0.0);
    }

    let avg = vals.iter().sum::<f64>() / vals.len() as f64;
    let mult = (1.0 + wind_cfg.ach_per_ms * avg).clamp(0.1, wind_cfg.max_multiplier);
    (mult, avg)
}

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

pub fn validate(config_path: &Path) -> ThermalResult<()> {
    let (cfg_txt, cfg) = load_thermal_config(config_path)?;
    if cfg.validation.windows.is_empty() {
        return Err(ThermalError::NoValidationWindows);
    }

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

    let parsed_windows = parse_validation_windows(&cfg.validation.windows)?;
    let earliest_val = parsed_windows.iter().map(|w| w.start).min().unwrap();
    let latest_val = parsed_windows.iter().map(|w| w.end).max().unwrap();

    let wind_points = if cfg.wind.enabled {
        fetch_open_meteo_wind(
            cfg.wind.latitude,
            cfg.wind.longitude,
            earliest_val,
            latest_val,
        )
    } else {
        Vec::new()
    };

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = std::env::var(&cfg.influx.token_env)
        .map_err(|_| ThermalError::MissingEnv(cfg.influx.token_env.clone()))?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &earliest_val,
        &latest_val,
    )?;

    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &earliest_val,
        &latest_val,
    )?;

    let room_series = build_room_series(&room_rows, &rooms)?;
    let doors_normal = build_doorways()?;
    let doors_closed = doors_all_closed_except_chimney(&doors_normal);

    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();
    let mut window_results = Vec::new();

    // Pre-compute thermal masses for all rooms (kJ/K)
    let thermal_masses: HashMap<String, f64> = rooms
        .iter()
        .map(|(name, room)| {
            (
                name.clone(),
                estimate_thermal_mass(room, &setup.connections),
            )
        })
        .collect();

    println!("Config: {}", config_path.display());
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );

    for window in parsed_windows {
        let (wind_mult, wind_avg) =
            wind_multiplier_for_window(&cfg.wind, &wind_points, window.start, window.end);
        let (measured, avg_temps, outside_avg) =
            measured_rates(&room_series, &outside_rows, window.start, window.end)?;
        let doorways = match window.door_state.as_str() {
            "closed_except_chimney" | "all_closed_except_chimney" | "closed" => &doors_closed,
            _ => &doors_normal,
        };
        let predicted = predict_rates(
            &rooms,
            &setup.connections,
            doorways,
            &avg_temps,
            outside_avg,
            result.doorway_cd,
            wind_mult,
        );

        report::print_table(
            &format!("Validation {}", window.name),
            &measured,
            &predicted,
        );
        let residuals =
            residuals_for_rooms(&measured, &predicted, Some(&exclude_rooms), &thermal_masses);
        let metrics = compute_metrics(&residuals);
        let wh = whole_house_metrics(&residuals, 5);
        let pass = metrics.rmse <= cfg.validation.thresholds.rmse_max
            && metrics.bias.abs() <= cfg.validation.thresholds.bias_abs_max
            && metrics.within_1_0c >= cfg.validation.thresholds.within_1c_min;

        println!(
            "  {}: rmse={:.3}, mae={:.3}, bias={:+.3}, within_1C={:.1}% => {}",
            window.name,
            metrics.rmse,
            metrics.mae,
            metrics.bias,
            metrics.within_1_0c * 100.0,
            if pass { "PASS" } else { "FAIL" }
        );
        println!(
            "  whole-house: meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
            wh.measured_w, wh.predicted_w, wh.error_w, wh.pred_over_meas
        );
        println!("  top error contributors:");
        for c in &wh.top_contributors {
            println!(
                "    {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
                c.room, c.measured_w, c.predicted_w, c.error_w
            );
        }

        window_results.push(WindowValidation {
            name: window.name,
            start: window.start.to_rfc3339(),
            end: window.end.to_rfc3339(),
            door_state: window.door_state,
            outside_avg_c: outside_avg,
            wind_avg_ms: wind_avg,
            wind_multiplier: wind_mult,
            metrics,
            whole_house: wh,
            pass,
            residuals,
        });
    }

    let mut all_residuals = Vec::new();
    for w in &window_results {
        all_residuals.extend(w.residuals.iter().map(|r| r.residual));
    }
    let aggregate_metrics = compute_metrics_from_values(&all_residuals);

    // Aggregate whole-house: sum across all windows
    let agg_measured_w: f64 = window_results
        .iter()
        .map(|w| w.whole_house.measured_w)
        .sum();
    let agg_predicted_w: f64 = window_results
        .iter()
        .map(|w| w.whole_house.predicted_w)
        .sum();
    let agg_error_w = agg_predicted_w - agg_measured_w;
    let agg_pred_over_meas = if agg_measured_w.abs() > 1e-9 {
        agg_predicted_w / agg_measured_w
    } else {
        f64::NAN
    };

    // Aggregate top contributors across all windows
    let mut agg_room_errors: HashMap<String, (f64, f64)> = HashMap::new();
    for w in &window_results {
        for r in &w.residuals {
            let meas_w = r.measured * r.thermal_mass_kj_per_k / 3.6;
            let pred_w = r.predicted * r.thermal_mass_kj_per_k / 3.6;
            let entry = agg_room_errors.entry(r.room.clone()).or_insert((0.0, 0.0));
            entry.0 += meas_w;
            entry.1 += pred_w;
        }
    }
    let mut agg_contributors: Vec<RoomHeatError> = agg_room_errors
        .into_iter()
        .map(|(room, (m, p))| RoomHeatError {
            room,
            measured_w: m,
            predicted_w: p,
            error_w: p - m,
        })
        .collect();
    agg_contributors.sort_by(|a, b| b.error_w.abs().total_cmp(&a.error_w.abs()));
    let agg_top: Vec<RoomHeatError> = agg_contributors.into_iter().take(5).collect();

    let aggregate_whole_house = WholeHouseMetrics {
        measured_w: agg_measured_w,
        predicted_w: agg_predicted_w,
        error_w: agg_error_w,
        pred_over_meas: agg_pred_over_meas,
        top_contributors: agg_top.clone(),
    };

    let aggregate_pass = aggregate_metrics.rmse <= cfg.validation.thresholds.rmse_max
        && aggregate_metrics.bias.abs() <= cfg.validation.thresholds.bias_abs_max
        && aggregate_metrics.within_1_0c >= cfg.validation.thresholds.within_1c_min;

    println!("\nValidation aggregate:");
    println!(
        "  rmse={:.3}, mae={:.3}, bias={:+.3}, within_1C={:.1}% => {}",
        aggregate_metrics.rmse,
        aggregate_metrics.mae,
        aggregate_metrics.bias,
        aggregate_metrics.within_1_0c * 100.0,
        if aggregate_pass { "PASS" } else { "FAIL" }
    );
    println!(
        "  whole-house: meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
        agg_measured_w, agg_predicted_w, agg_error_w, agg_pred_over_meas
    );
    println!("  top aggregate error contributors:");
    for c in &agg_top {
        println!(
            "    {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
            c.room, c.measured_w, c.predicted_w, c.error_w
        );
    }

    let validation = ValidationSummary {
        thresholds: ThresholdResult {
            rmse_max: cfg.validation.thresholds.rmse_max,
            bias_abs_max: cfg.validation.thresholds.bias_abs_max,
            within_1c_min: cfg.validation.thresholds.within_1c_min,
        },
        aggregate_metrics,
        aggregate_whole_house,
        aggregate_pass,
        windows: window_results,
    };

    let artifact = build_artifact(
        "thermal-validate",
        config_path,
        &cfg_txt,
        &cfg,
        &setup,
        &result,
        Some(validation),
    )?;
    let artifact_path = write_artifact("thermal-validate", &artifact)?;
    println!("\nWrote validation artifact: {}", artifact_path.display());

    Ok(())
}

pub fn fit_diagnostics(config_path: &Path) -> ThermalResult<()> {
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

    let fit_cfg = &cfg.fit_diagnostics;
    let range_start = fit_cfg
        .start
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| setup.night1_start - chrono::Duration::hours(24));
    let range_end = fit_cfg
        .end
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| Utc::now().fixed_offset());

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = std::env::var(&cfg.influx.token_env)
        .map_err(|_| ThermalError::MissingEnv(cfg.influx.token_env.clone()))?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &range_start,
        &range_end,
    )?;
    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;
    let status_rows = influx::query_status_codes(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;

    if status_rows.is_empty() {
        return Err(ThermalError::NoStatusData);
    }

    let room_series = build_room_series(&room_rows, &rooms)?;
    let cooldown_periods = detect_cooldown_periods(
        &status_rows,
        &fit_cfg.heating_off_codes,
        fit_cfg.min_period_hours,
    );
    if cooldown_periods.is_empty() {
        return Err(ThermalError::NoCooldownPeriods);
    }

    println!("Config: {}", config_path.display());
    println!(
        "Diagnostics range: {} -> {}",
        range_start.to_rfc3339(),
        range_end.to_rfc3339()
    );
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );

    println!("Found {} cooldown periods:", cooldown_periods.len());
    for (start, end) in &cooldown_periods {
        let hours = (*end - *start).num_seconds() as f64 / 3600.0;
        println!(
            "  {} -> {} ({:.1}h)",
            start.format("%H:%M"),
            end.format("%H:%M"),
            hours
        );
    }

    println!(
        "\n{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>5} {:>16}",
        "Room", "Start", "End", "Meas", "Pred", "Ratio", "Body", "Period"
    );
    println!(
        "{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>5}",
        "", "°C", "°C", "°C/hr", "°C/hr", "P/M", "W"
    );
    println!("{}", "─".repeat(92));

    let doorways_base = build_doorways()?;
    let doorways_closed = doors_all_closed_except_chimney(&doorways_base);
    let doorways = match fit_cfg.door_state.as_str() {
        "closed_except_chimney" | "all_closed_except_chimney" | "closed" => &doorways_closed,
        _ => &doorways_base,
    };

    let mut records = Vec::new();
    for (period_start, period_end) in &cooldown_periods {
        let outside_in_period: Vec<f64> = outside_rows
            .iter()
            .filter(|(t, _)| *t >= *period_start && *t <= *period_end)
            .map(|(_, v)| *v)
            .collect();
        let avg_outside = if outside_in_period.is_empty() {
            8.0
        } else {
            outside_in_period.iter().sum::<f64>() / outside_in_period.len() as f64
        };

        let mut avg_temps = HashMap::new();
        for (room_name, series) in &room_series {
            let vals: Vec<f64> = series
                .iter()
                .filter(|(t, _)| *t >= *period_start && *t <= *period_end)
                .map(|(_, v)| *v)
                .collect();
            if !vals.is_empty() {
                avg_temps.insert(
                    room_name.clone(),
                    vals.iter().sum::<f64>() / vals.len() as f64,
                );
            }
        }

        for (room_name, series) in &room_series {
            let temps_in_period: Vec<(DateTime<FixedOffset>, f64)> = series
                .iter()
                .cloned()
                .filter(|(t, _)| *t >= *period_start && *t <= *period_end)
                .collect();
            if temps_in_period.len() < 2 {
                continue;
            }
            let (first, last) = match (temps_in_period.first(), temps_in_period.last()) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let hours = (last.0 - first.0).num_seconds() as f64 / 3600.0;
            if hours < fit_cfg.min_record_hours {
                continue;
            }

            let meas_rate = (first.1 - last.1) / hours;
            let avg_t = avg_temps
                .get(room_name)
                .copied()
                .unwrap_or((first.1 + last.1) / 2.0);
            let Some(room) = rooms.get(room_name) else {
                continue;
            };
            let c = estimate_thermal_mass(room, &setup.connections);
            let bal = room_energy_balance(
                room,
                avg_t,
                avg_outside,
                &avg_temps,
                &setup.connections,
                doorways,
                result.doorway_cd,
                1.0,
            );
            let pred_rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };
            let body_w = room.overnight_occupants as f64 * BODY_HEAT_SLEEPING_W;
            let ratio = if meas_rate.abs() > fit_cfg.ratio_min_meas {
                Some(pred_rate / meas_rate)
            } else {
                None
            };
            let true_cooling = meas_rate >= fit_cfg.min_meas_cooling;
            let marker = if true_cooling { "" } else { "*" };
            let period_str = format!(
                "{}→{}{}",
                period_start.format("%H:%M"),
                period_end.format("%H:%M"),
                marker
            );
            let ratio_str = ratio.map_or("   nan".to_string(), |v| format!("{:>6.2}", v));

            println!(
                "{:<14} {:>7.2} {:>7.2} {:>7.3} {:>7.3} {} {:>5.0} {:>16}",
                room_name, first.1, last.1, meas_rate, pred_rate, ratio_str, body_w, period_str
            );

            records.push(FitRecord {
                room: room_name.clone(),
                period_start: period_start.to_rfc3339(),
                period_end: period_end.to_rfc3339(),
                start_temp_c: first.1,
                end_temp_c: last.1,
                meas_rate_c_per_hr: meas_rate,
                pred_rate_c_per_hr: pred_rate,
                ratio_pred_over_meas: ratio,
                body_w,
                true_cooling,
            });
        }
    }

    println!("\n* marks weak/non-cooling measured periods (meas_rate < 0.03 °C/hr)\n");

    let summary_all = summarize_fit_records(&records);
    let good: Vec<FitRecord> = records.iter().filter(|r| r.true_cooling).cloned().collect();
    let summary_good = summarize_fit_records(&good);

    println!("Summary (all records):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  Median ratio={}",
        summary_all.n,
        summary_all.rmse,
        summary_all.mae,
        summary_all
            .med_ratio
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "nan".to_string())
    );

    println!("Summary (true cooling only):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  Median ratio={}",
        summary_good.n,
        summary_good.rmse,
        summary_good.mae,
        summary_good
            .med_ratio
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "nan".to_string())
    );

    let per_room = summarize_fit_by_room(&good);
    println!("\nPer-room summary (true cooling only):");
    println!(
        "{:<14} {:>4} {:>8} {:>8} {:>8}",
        "Room", "N", "RMSE", "MAE", "MedRat"
    );
    println!("{}", "─".repeat(46));
    for row in &per_room {
        println!(
            "{:<14} {:>4} {:>8.3} {:>8.3} {:>8}",
            row.room,
            row.n,
            row.rmse,
            row.mae,
            row.med_ratio
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "nan".to_string())
        );
    }

    let periods_json: Vec<FitPeriod> = cooldown_periods
        .iter()
        .map(|(s, e)| FitPeriod {
            start: s.to_rfc3339(),
            end: e.to_rfc3339(),
            hours: (*e - *s).num_seconds() as f64 / 3600.0,
        })
        .collect();

    let artifact = FitDiagnosticsArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-fit-diagnostics".to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(&cfg_txt),
        git: git_meta(),
        range_start: range_start.to_rfc3339(),
        range_end: range_end.to_rfc3339(),
        door_state: fit_cfg.door_state.clone(),
        cooldown_periods: periods_json,
        records: records.clone(),
        summary_all,
        summary_true_cooling: summary_good,
        per_room_true_cooling: per_room,
        calibrated_params: ArtifactCalibrationParams {
            leather_ach: result.leather_ach,
            landing_ach: result.landing_ach,
            conservatory_ach: result.conservatory_ach,
            office_ach: result.office_ach,
            doorway_cd: result.doorway_cd,
        },
    };

    let artifact_path = write_fit_artifact("thermal-fit-diagnostics", &artifact)?;
    println!(
        "\nWrote fit diagnostics artifact: {}",
        artifact_path.display()
    );

    Ok(())
}

/// Classify HP state from BuildingCircuitFlow (L/h).
/// Arotherm Plus 5kW (from eBUS data analysis 2026-03-28):
///   > 900 L/h (~15 L/min) = DHW (diverter open to cylinder, ~1240 L/h = 20.7 L/min)
///   780-900 L/h (~13-15 L/min) = Heating (fixed pump ~860 L/h = 14.3 L/min)
///   < 100 L/h = Off
fn classify_hp_state_from_flow(flow_lph: f64) -> HpState {
    if flow_lph > 900.0 {
        HpState::Dhw
    } else if flow_lph >= 780.0 {
        HpState::Heating
    } else {
        HpState::Off
    }
}

/// Segment flow rate series into contiguous periods of same HP state.
fn segment_by_flow(
    flow_rows: &[(DateTime<FixedOffset>, f64)],
    min_period_hours: f64,
) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>, HpState)> {
    if flow_rows.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut seg_start = flow_rows[0].0;
    let mut seg_state = classify_hp_state_from_flow(flow_rows[0].1);

    for &(t, flow) in &flow_rows[1..] {
        let state = classify_hp_state_from_flow(flow);
        if state != seg_state {
            let hours = (t - seg_start).num_seconds() as f64 / 3600.0;
            if hours >= min_period_hours {
                segments.push((seg_start, t, seg_state));
            }
            seg_start = t;
            seg_state = state;
        }
    }

    // Close last segment
    if let Some(&(t, _)) = flow_rows.last() {
        let hours = (t - seg_start).num_seconds() as f64 / 3600.0;
        if hours >= min_period_hours {
            segments.push((seg_start, t, seg_state));
        }
    }

    segments
}

pub fn operational_validate(config_path: &Path) -> ThermalResult<()> {
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

    let fit_cfg = &cfg.fit_diagnostics;
    let range_start = fit_cfg
        .start
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| setup.night1_start - chrono::Duration::hours(24));
    let range_end = fit_cfg
        .end
        .as_deref()
        .map(influx::parse_dt)
        .transpose()?
        .unwrap_or_else(|| Utc::now().fixed_offset());

    let sensor_topics: Vec<&str> = rooms.values().map(|r| r.sensor_topic).collect();
    let token = std::env::var(&cfg.influx.token_env)
        .map_err(|_| ThermalError::MissingEnv(cfg.influx.token_env.clone()))?;

    let room_rows = influx::query_room_temps(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &sensor_topics,
        &range_start,
        &range_end,
    )?;
    let outside_rows = influx::query_outside_temp(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;
    let bcf_rows = influx::query_building_circuit_flow(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;
    let mwt_rows = influx::query_mwt(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;
    let pv_rows = influx::query_pv_power(
        &cfg.influx.url,
        &cfg.influx.org,
        &cfg.influx.bucket,
        &token,
        &range_start,
        &range_end,
    )?;

    // Fetch solar irradiance from Open-Meteo (DNI + DHI → surface irradiance)
    let solar_irradiance = fetch_surface_irradiance(51.60, -0.11, range_start, range_end);

    if bcf_rows.is_empty() {
        return Err(ThermalError::NoStatusData);
    }

    let room_series = build_room_series(&room_rows, &rooms)?;
    let connections = &setup.connections;
    let doorways = build_doorways()?;

    let thermal_masses: HashMap<String, f64> = rooms
        .iter()
        .map(|(name, room)| (name.clone(), estimate_thermal_mass(room, connections)))
        .collect();

    // Segment into periods by BuildingCircuitFlow (not status codes — unreliable for DHW)
    // DHW cycles are typically 30 min to 2 hours; heating periods longer.
    // Use 5 min minimum to avoid noise from brief transitions.
    let segments = segment_by_flow(&bcf_rows, 5.0 / 60.0);

    println!("Config: {}", config_path.display());
    println!(
        "Operational range: {} -> {}",
        range_start.to_rfc3339(),
        range_end.to_rfc3339()
    );
    println!(
        "Calibrated params: leather_ach={:.2}, landing_ach={:.2}, conservatory_ach={:.2}, office_ach={:.2}, doorway_cd={:.2}",
        result.leather_ach, result.landing_ach, result.conservatory_ach, result.office_ach, result.doorway_cd
    );
    println!(
        "Found {} segments ({} heating, {} DHW, {} off)",
        segments.len(),
        segments.iter().filter(|s| s.2 == HpState::Heating).count(),
        segments.iter().filter(|s| s.2 == HpState::Dhw).count(),
        segments.iter().filter(|s| s.2 == HpState::Off).count(),
    );

    println!(
        "\n{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8} {:>16}",
        "Room", "Start", "End", "Meas", "Pred", "Rad", "Loss", "State", "Period"
    );
    println!(
        "{:<14} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8}",
        "", "°C", "°C", "°C/hr", "°C/hr", "W", "W", ""
    );
    println!("{}", "─".repeat(100));

    let mut records = Vec::new();

    for &(seg_start, seg_end, hp_state) in &segments {
        // Average MWT during this segment
        let mwt_in_seg: Vec<f64> = mwt_rows
            .iter()
            .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
            .map(|(_, v)| *v)
            .collect();
        let avg_mwt = if mwt_in_seg.is_empty() {
            0.0
        } else {
            mwt_in_seg.iter().sum::<f64>() / mwt_in_seg.len() as f64
        };

        // For off/DHW periods, radiators contribute nothing
        let effective_mwt = match hp_state {
            HpState::Heating => avg_mwt,
            _ => 0.0,
        };

        let outside_in_seg: Vec<f64> = outside_rows
            .iter()
            .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
            .map(|(_, v)| *v)
            .collect();
        let avg_outside = if outside_in_seg.is_empty() {
            8.0
        } else {
            outside_in_seg.iter().sum::<f64>() / outside_in_seg.len() as f64
        };

        // Compute average room temps in this segment
        let mut avg_temps = HashMap::new();
        for (room_name, series) in &room_series {
            let vals: Vec<f64> = series
                .iter()
                .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
                .map(|(_, v)| *v)
                .collect();
            if !vals.is_empty() {
                avg_temps.insert(
                    room_name.clone(),
                    vals.iter().sum::<f64>() / vals.len() as f64,
                );
            }
        }

        let sleeping = {
            let hour = seg_start.hour();
            hour >= 22 || hour < 7
        };

        // Solar irradiance: PV for SW (direct measurement), Open-Meteo for NE/horizontal
        let pv_in_seg: Vec<f64> = pv_rows
            .iter()
            .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
            .map(|(_, v)| *v)
            .collect();
        let pv_sw_vert = if pv_in_seg.is_empty() {
            0.0
        } else {
            pv_to_sw_vertical_irradiance(pv_in_seg.iter().sum::<f64>() / pv_in_seg.len() as f64)
        };

        let (_meteo_sw, ne_vert, ne_horiz, _se_vert) =
            avg_irradiance_in_window(&solar_irradiance, seg_start, seg_end);

        // Use PV for SW (more accurate than Open-Meteo for this orientation)
        let sw_vert = if pv_sw_vert > 0.0 {
            pv_sw_vert
        } else {
            _meteo_sw // Fallback to Open-Meteo if no PV data
        };

        let period_str = format!(
            "{}→{}",
            seg_start.format("%m-%d %H:%M"),
            seg_end.format("%H:%M")
        );

        for (room_name, series) in &room_series {
            let temps_in_seg: Vec<(DateTime<FixedOffset>, f64)> = series
                .iter()
                .cloned()
                .filter(|(t, _)| *t >= seg_start && *t <= seg_end)
                .collect();
            if temps_in_seg.len() < 2 {
                continue;
            }
            let (first, last) = match (temps_in_seg.first(), temps_in_seg.last()) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let hours = (last.0 - first.0).num_seconds() as f64 / 3600.0;
            if hours < 0.25 {
                continue;
            }

            let meas_rate = (first.1 - last.1) / hours;
            let Some(room) = rooms.get(room_name) else {
                continue;
            };

            let c = thermal_masses.get(room_name).copied().unwrap_or(0.0);

            let bal = full_room_energy_balance(
                room,
                avg_temps.get(room_name).copied().unwrap_or(first.1),
                avg_outside,
                &avg_temps,
                connections,
                &doorways,
                result.doorway_cd,
                1.0,
                effective_mwt,
                sleeping,
                sw_vert,
                ne_vert,
                ne_horiz,
            );
            let pred_rate = if c > 0.0 { -bal * 3.6 / c } else { 0.0 };

            // Compute radiator contribution separately for reporting
            let rad_w: f64 = if effective_mwt > 0.0 {
                let rt = avg_temps.get(room_name).copied().unwrap_or(first.1);
                room.radiators
                    .iter()
                    .filter(|r| r.active)
                    .map(|r| radiator_output(r.t50, effective_mwt, rt))
                    .sum()
            } else {
                0.0
            };
            let loss_w = bal - rad_w; // net loss (negative = losing heat)

            println!(
                "{:<14} {:>7.2} {:>7.2} {:>7.3} {:>7.3} {:>6.0} {:>6.0} {:>8} {:>16}",
                room_name,
                first.1,
                last.1,
                meas_rate,
                pred_rate,
                rad_w,
                loss_w,
                hp_state,
                period_str
            );

            records.push(OperationalRecord {
                room: room_name.clone(),
                period_start: seg_start.to_rfc3339(),
                period_end: seg_end.to_rfc3339(),
                hp_state: hp_state.to_string(),
                mwt_avg_c: effective_mwt,
                outside_avg_c: avg_outside,
                start_temp_c: first.1,
                end_temp_c: last.1,
                meas_rate_c_per_hr: meas_rate,
                pred_rate_c_per_hr: pred_rate,
                radiator_w: rad_w,
                loss_w,
            });
        }
    }

    // Exclude rooms from scoring (still reported in per-record output above)
    let exclude_rooms: HashSet<String> = cfg.objective.exclude_rooms.iter().cloned().collect();
    let scored_records: Vec<&OperationalRecord> = records
        .iter()
        .filter(|r| !exclude_rooms.contains(&r.room))
        .collect();
    let scored_owned: Vec<OperationalRecord> =
        scored_records.iter().map(|r| (*r).clone()).collect();

    if !exclude_rooms.is_empty() {
        println!("\nExcluded from scoring: {:?}", cfg.objective.exclude_rooms);
    }

    // Summaries (scored rooms only)
    let summary_all = operational_summary(&scored_owned);
    println!("\nSummary (scored rooms, all segments):");
    println!(
        "  N={}  RMSE={:.3} °C/hr  MAE={:.3} °C/hr  bias={:+.3} °C/hr",
        summary_all.n, summary_all.rmse, summary_all.mae, summary_all.bias,
    );

    let mut summary_by_state = Vec::new();
    for state_name in ["heating", "off", "dhw"] {
        let subset: Vec<OperationalRecord> = scored_owned
            .iter()
            .filter(|r| r.hp_state == state_name)
            .cloned()
            .collect();
        let s = operational_summary(&subset);
        if s.n > 0 {
            println!(
                "  {}: N={}  RMSE={:.3}  MAE={:.3}  bias={:+.3}",
                state_name, s.n, s.rmse, s.mae, s.bias,
            );
        }
        summary_by_state.push((state_name.to_string(), s));
    }

    // Per-room summary (all rooms, but mark excluded)
    let per_room = operational_summary_by_room(&records);
    println!(
        "\n{:<14} {:>4} {:>8} {:>8} {:>8} {}",
        "Room", "N", "RMSE", "MAE", "Bias", ""
    );
    println!("{}", "─".repeat(52));
    for row in &per_room {
        let marker = if exclude_rooms.contains(&row.room) {
            " (excluded)"
        } else {
            ""
        };
        println!(
            "{:<14} {:>4} {:>8.3} {:>8.3} {:>+8.3}{}",
            row.room, row.n, row.rmse, row.mae, row.bias, marker
        );
    }

    // Whole-house weighted metrics (scored rooms only)
    let wh_entries: Vec<RoomResidual> = {
        let mut by_room: HashMap<String, (f64, f64, f64)> = HashMap::new();
        for r in &scored_owned {
            let c = thermal_masses.get(&r.room).copied().unwrap_or(0.0);
            let entry = by_room.entry(r.room.clone()).or_insert((0.0, 0.0, 0.0));
            entry.0 += r.meas_rate_c_per_hr;
            entry.1 += r.pred_rate_c_per_hr;
            entry.2 = c;
        }
        by_room
            .into_iter()
            .map(|(room, (m, p, c))| RoomResidual {
                room,
                measured: m,
                predicted: p,
                residual: p - m,
                abs_residual: (p - m).abs(),
                thermal_mass_kj_per_k: c,
            })
            .collect()
    };
    let wh = whole_house_metrics(&wh_entries, 5);
    println!(
        "\nWhole-house (scored rooms, thermal-mass weighted): meas={:.0}W, pred={:.0}W, err={:+.0}W, pred/meas={:.2}",
        wh.measured_w, wh.predicted_w, wh.error_w, wh.pred_over_meas
    );
    println!("Top error contributors:");
    for c in &wh.top_contributors {
        println!(
            "  {:<14} meas={:>6.0}W  pred={:>6.0}W  err={:>+6.0}W",
            c.room, c.measured_w, c.predicted_w, c.error_w
        );
    }

    // Write artifact
    let artifact = OperationalArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-operational".to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(&cfg_txt),
        git: git_meta(),
        range_start: range_start.to_rfc3339(),
        range_end: range_end.to_rfc3339(),
        calibrated_params: ArtifactCalibrationParams {
            leather_ach: result.leather_ach,
            landing_ach: result.landing_ach,
            conservatory_ach: result.conservatory_ach,
            office_ach: result.office_ach,
            doorway_cd: result.doorway_cd,
        },
        summary_all,
        summary_by_state,
        per_room,
        whole_house: wh,
        records,
    };

    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("thermal-operational-{}.json", ts));
    let json = serde_json::to_string_pretty(&artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    println!("\nWrote operational artifact: {}", path.display());

    Ok(())
}

fn operational_summary(records: &[OperationalRecord]) -> OperationalSummary {
    if records.is_empty() {
        return OperationalSummary {
            n: 0,
            rmse: f64::NAN,
            mae: f64::NAN,
            bias: f64::NAN,
        };
    }
    let errs: Vec<f64> = records
        .iter()
        .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
        .collect();
    let n = errs.len();
    let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / n as f64).sqrt();
    let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / n as f64;
    let bias = errs.iter().sum::<f64>() / n as f64;
    OperationalSummary { n, rmse, mae, bias }
}

fn operational_summary_by_room(records: &[OperationalRecord]) -> Vec<PerRoomOperationalSummary> {
    let mut by_room: BTreeMap<String, Vec<&OperationalRecord>> = BTreeMap::new();
    for r in records {
        by_room.entry(r.room.clone()).or_default().push(r);
    }
    by_room
        .into_iter()
        .map(|(room, rows)| {
            let errs: Vec<f64> = rows
                .iter()
                .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
                .collect();
            let n = errs.len();
            let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / n as f64).sqrt();
            let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / n as f64;
            let bias = errs.iter().sum::<f64>() / n as f64;
            PerRoomOperationalSummary {
                room,
                n,
                rmse,
                mae,
                bias,
            }
        })
        .collect()
}

fn detect_cooldown_periods(
    status_rows: &[(DateTime<FixedOffset>, i32)],
    heating_off_codes: &[i32],
    min_period_hours: f64,
) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    let off: HashSet<i32> = heating_off_codes.iter().copied().collect();
    let mut periods = Vec::new();
    let mut in_cooldown = false;
    let mut start: Option<DateTime<FixedOffset>> = None;

    for (t, code) in status_rows {
        if off.contains(code) {
            if !in_cooldown {
                start = Some(*t);
                in_cooldown = true;
            }
        } else if in_cooldown {
            if let Some(s) = start {
                let h = (*t - s).num_seconds() as f64 / 3600.0;
                if h > min_period_hours {
                    periods.push((s, *t));
                }
            }
            in_cooldown = false;
            start = None;
        }
    }

    if in_cooldown {
        if let (Some(s), Some((end, _))) = (start, status_rows.last()) {
            let h = (*end - s).num_seconds() as f64 / 3600.0;
            if h > min_period_hours {
                periods.push((s, *end));
            }
        }
    }

    periods
}

fn summarize_fit_records(records: &[FitRecord]) -> FitSummary {
    if records.is_empty() {
        return FitSummary {
            n: 0,
            rmse: f64::NAN,
            mae: f64::NAN,
            med_ratio: None,
        };
    }

    let errs: Vec<f64> = records
        .iter()
        .map(|r| r.pred_rate_c_per_hr - r.meas_rate_c_per_hr)
        .collect();
    let rmse = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len() as f64).sqrt();
    let mae = errs.iter().map(|e| e.abs()).sum::<f64>() / errs.len() as f64;

    let mut ratios: Vec<f64> = records
        .iter()
        .filter_map(|r| r.ratio_pred_over_meas)
        .collect();
    ratios.sort_by(|a, b| a.total_cmp(b));
    let med_ratio = median(&ratios);

    FitSummary {
        n: records.len(),
        rmse,
        mae,
        med_ratio,
    }
}

fn summarize_fit_by_room(records: &[FitRecord]) -> Vec<PerRoomFitSummary> {
    let mut by_room: BTreeMap<String, Vec<FitRecord>> = BTreeMap::new();
    for r in records {
        by_room.entry(r.room.clone()).or_default().push(r.clone());
    }

    by_room
        .into_iter()
        .map(|(room, rows)| {
            let st = summarize_fit_records(&rows);
            PerRoomFitSummary {
                room,
                n: st.n,
                rmse: st.rmse,
                mae: st.mae,
                med_ratio: st.med_ratio,
            }
        })
        .collect()
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let n = values.len();
    if n % 2 == 1 {
        Some(values[n / 2])
    } else {
        Some((values[n / 2 - 1] + values[n / 2]) / 2.0)
    }
}

struct CalibrationSetup {
    rooms: BTreeMap<String, RoomDef>,
    connections: Vec<InternalConnection>,
    doors_n1: Vec<Doorway>,
    doors_n2: Vec<Doorway>,
    night1_start: DateTime<FixedOffset>,
    night1_end: DateTime<FixedOffset>,
    night2_start: DateTime<FixedOffset>,
    night2_end: DateTime<FixedOffset>,
    wind_mult_n1: f64,
    wind_avg_n1: f64,
    wind_mult_n2: f64,
    wind_avg_n2: f64,
    meas1: HashMap<String, f64>,
    avg1: HashMap<String, f64>,
    outside1: f64,
    meas2: HashMap<String, f64>,
    avg2: HashMap<String, f64>,
    outside2: f64,
}

fn load_thermal_config(config_path: &Path) -> ThermalResult<(String, ThermalConfig)> {
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

fn prepare_calibration(cfg: &ThermalConfig) -> ThermalResult<CalibrationSetup> {
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
fn run_grid_search(
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

fn parse_validation_windows(raw: &[ValidationWindowCfg]) -> ThermalResult<Vec<ParsedWindow>> {
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

fn residuals_for_rooms(
    measured: &HashMap<String, f64>,
    predicted: &HashMap<String, f64>,
    exclude: Option<&HashSet<String>>,
    thermal_masses: &HashMap<String, f64>,
) -> Vec<RoomResidual> {
    let mut out = Vec::new();
    let mut keys: Vec<_> = measured.keys().cloned().collect();
    keys.sort();

    for room in keys {
        if exclude.is_some_and(|x| x.contains(&room)) {
            continue;
        }
        let measured_v = measured.get(&room).copied().unwrap_or(0.0);
        let predicted_v = predicted.get(&room).copied().unwrap_or(f64::NAN);
        if predicted_v.is_nan() {
            continue;
        }
        let residual = predicted_v - measured_v;
        let c_kj = thermal_masses.get(&room).copied().unwrap_or(0.0);
        out.push(RoomResidual {
            room,
            measured: measured_v,
            predicted: predicted_v,
            residual,
            abs_residual: residual.abs(),
            thermal_mass_kj_per_k: c_kj,
        });
    }

    out
}

fn whole_house_metrics(residuals: &[RoomResidual], top_n: usize) -> WholeHouseMetrics {
    let mut entries: Vec<RoomHeatError> = residuals
        .iter()
        .map(|r| {
            // rate (°C/hr) * C (kJ/K) / 3.6 = W
            let meas_w = r.measured * r.thermal_mass_kj_per_k / 3.6;
            let pred_w = r.predicted * r.thermal_mass_kj_per_k / 3.6;
            RoomHeatError {
                room: r.room.clone(),
                measured_w: meas_w,
                predicted_w: pred_w,
                error_w: pred_w - meas_w,
            }
        })
        .collect();

    let measured_w: f64 = entries.iter().map(|e| e.measured_w).sum();
    let predicted_w: f64 = entries.iter().map(|e| e.predicted_w).sum();
    let error_w = predicted_w - measured_w;
    let pred_over_meas = if measured_w.abs() > 1e-9 {
        predicted_w / measured_w
    } else {
        f64::NAN
    };

    entries.sort_by(|a, b| b.error_w.abs().total_cmp(&a.error_w.abs()));
    let top_contributors: Vec<RoomHeatError> = entries.into_iter().take(top_n).collect();

    WholeHouseMetrics {
        measured_w,
        predicted_w,
        error_w,
        pred_over_meas,
        top_contributors,
    }
}

fn compute_metrics(residuals: &[RoomResidual]) -> Metrics {
    let values: Vec<f64> = residuals.iter().map(|r| r.residual).collect();
    compute_metrics_from_values(&values)
}

fn compute_metrics_from_values(values: &[f64]) -> Metrics {
    if values.is_empty() {
        return Metrics {
            rooms_count: 0,
            rmse: 999.0,
            mae: 999.0,
            bias: 0.0,
            max_abs_error: 999.0,
            within_0_5c: 0.0,
            within_1_0c: 0.0,
        };
    }

    let n = values.len() as f64;
    let sq = values.iter().map(|v| v * v).sum::<f64>();
    let abs_sum = values.iter().map(|v| v.abs()).sum::<f64>();
    let bias = values.iter().sum::<f64>() / n;
    let max_abs = values.iter().map(|v| v.abs()).fold(0.0, f64::max);
    let within_05 = values.iter().filter(|v| v.abs() <= 0.5).count() as f64 / n;
    let within_10 = values.iter().filter(|v| v.abs() <= 1.0).count() as f64 / n;

    Metrics {
        rooms_count: values.len(),
        rmse: (sq / n).sqrt(),
        mae: abs_sum / n,
        bias,
        max_abs_error: max_abs,
        within_0_5c: within_05,
        within_1_0c: within_10,
    }
}

fn config_sha256(cfg_txt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cfg_txt.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

fn git_meta() -> GitMeta {
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    GitMeta { sha, dirty }
}

fn build_artifact(
    command: &str,
    config_path: &Path,
    cfg_txt: &str,
    cfg: &ThermalConfig,
    setup: &CalibrationSetup,
    result: &CalibrationResult,
    validation: Option<ValidationSummary>,
) -> ThermalResult<CalibrationArtifact> {
    // Compute thermal masses for artifact residuals
    let thermal_masses: HashMap<String, f64> = setup
        .rooms
        .iter()
        .map(|(name, room)| {
            (
                name.clone(),
                estimate_thermal_mass(room, &setup.connections),
            )
        })
        .collect();

    let calibration = ArtifactCalibration {
        leather_ach: result.leather_ach,
        landing_ach: result.landing_ach,
        conservatory_ach: result.conservatory_ach,
        office_ach: result.office_ach,
        doorway_cd: result.doorway_cd,
        rmse_night1: result.r1,
        rmse_night2: result.r2,
        base_score: result.base_score,
        final_score: result.final_score,
        night1: residuals_for_rooms(&setup.meas1, &result.pred1, None, &thermal_masses),
        night2: residuals_for_rooms(&setup.meas2, &result.pred2, None, &thermal_masses),
    };

    Ok(CalibrationArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: command.to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(cfg_txt),
        git: git_meta(),
        calibration_windows: vec![
            ArtifactWindow {
                name: "night1".to_string(),
                start: cfg.test_nights.night1_start.clone(),
                end: cfg.test_nights.night1_end.clone(),
            },
            ArtifactWindow {
                name: "night2".to_string(),
                start: cfg.test_nights.night2_start.clone(),
                end: cfg.test_nights.night2_end.clone(),
            },
        ],
        calibration,
        validation,
    })
}

fn write_artifact(prefix: &str, artifact: &CalibrationArtifact) -> ThermalResult<PathBuf> {
    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{}.json", prefix, ts));
    let json = serde_json::to_string_pretty(artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

fn write_fit_artifact(prefix: &str, artifact: &FitDiagnosticsArtifact) -> ThermalResult<PathBuf> {
    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{}.json", prefix, ts));
    let json = serde_json::to_string_pretty(artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotFileEntry {
    source_rel_path: String,
    snapshot_rel_path: String,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ThermalSnapshotManifest {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    signoff_reason: String,
    git: GitMeta,
    config_path: String,
    config_sha256: String,
    files: Vec<SnapshotFileEntry>,
}

pub fn snapshot_export(
    config_path: &Path,
    signoff_reason: &str,
    approved_by_human: bool,
) -> ThermalResult<PathBuf> {
    if !approved_by_human {
        return Err(ThermalError::HumanApprovalRequired);
    }
    if signoff_reason.trim().is_empty() {
        return Err(ThermalError::EmptySignoffReason);
    }

    let cfg_txt = fs::read_to_string(config_path).map_err(|source| ThermalError::ConfigRead {
        path: config_path.display().to_string(),
        source,
    })?;

    let snapshot_root = Path::new("artifacts").join("thermal").join("snapshots");
    fs::create_dir_all(&snapshot_root).map_err(|source| ThermalError::ArtifactWrite {
        path: snapshot_root.display().to_string(),
        source,
    })?;

    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let out_dir = snapshot_root.join(format!("thermal-snapshot-{}", ts));
    let files_dir = out_dir.join("files");
    fs::create_dir_all(&files_dir).map_err(|source| ThermalError::ArtifactWrite {
        path: files_dir.display().to_string(),
        source,
    })?;

    let required_paths = [
        "artifacts/thermal/baselines/thermal-calibrate-baseline.json",
        "artifacts/thermal/baselines/thermal-validate-baseline.json",
        "artifacts/thermal/baselines/thermal-fit-diagnostics-baseline.json",
        "artifacts/thermal/regression-thresholds.toml",
    ];

    let mut entries = Vec::new();
    for src_rel in required_paths {
        let src = Path::new(src_rel);
        let file_name = src
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or_else(|| ThermalError::InvalidSnapshotPath(src_rel.to_string()))?;
        let dst_rel = format!("files/{file_name}");
        let dst = out_dir.join(&dst_rel);

        fs::copy(src, &dst).map_err(|source| ThermalError::SnapshotCopy {
            from: src.display().to_string(),
            to: dst.display().to_string(),
            source,
        })?;

        let sha = sha256_file(&dst)?;
        entries.push(SnapshotFileEntry {
            source_rel_path: src_rel.to_string(),
            snapshot_rel_path: dst_rel,
            sha256: sha,
        });
    }

    let cfg_copy_name = config_path
        .file_name()
        .and_then(|x| x.to_str())
        .ok_or_else(|| ThermalError::InvalidSnapshotPath(config_path.display().to_string()))?;
    let cfg_copy_rel = format!("files/{cfg_copy_name}");
    let cfg_copy_path = out_dir.join(&cfg_copy_rel);
    fs::copy(config_path, &cfg_copy_path).map_err(|source| ThermalError::SnapshotCopy {
        from: config_path.display().to_string(),
        to: cfg_copy_path.display().to_string(),
        source,
    })?;
    entries.push(SnapshotFileEntry {
        source_rel_path: config_path.display().to_string(),
        snapshot_rel_path: cfg_copy_rel,
        sha256: sha256_file(&cfg_copy_path)?,
    });

    let manifest = ThermalSnapshotManifest {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-snapshot-export".to_string(),
        signoff_reason: signoff_reason.trim().to_string(),
        git: git_meta(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(&cfg_txt),
        files: entries,
    };

    let manifest_path = out_dir.join("manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&manifest_path, manifest_json).map_err(|source| ThermalError::ArtifactWrite {
        path: manifest_path.display().to_string(),
        source,
    })?;

    Ok(manifest_path)
}

pub fn snapshot_import(
    manifest_path: &Path,
    signoff_reason: &str,
    approved_by_human: bool,
) -> ThermalResult<()> {
    if !approved_by_human {
        return Err(ThermalError::HumanApprovalRequired);
    }
    if signoff_reason.trim().is_empty() {
        return Err(ThermalError::EmptySignoffReason);
    }

    let manifest_txt =
        fs::read_to_string(manifest_path).map_err(|source| ThermalError::SnapshotManifestRead {
            path: manifest_path.display().to_string(),
            source,
        })?;
    let manifest: ThermalSnapshotManifest =
        serde_json::from_str(&manifest_txt).map_err(|source| {
            ThermalError::SnapshotManifestParse {
                path: manifest_path.display().to_string(),
                source,
            }
        })?;

    let root = manifest_path
        .parent()
        .ok_or_else(|| ThermalError::InvalidSnapshotPath(manifest_path.display().to_string()))?;

    for entry in manifest.files {
        let src_rel = sanitize_relative_path(&entry.snapshot_rel_path)?;
        let dst_rel = sanitize_relative_path(&entry.source_rel_path)?;

        let src = root.join(src_rel);
        let dst = Path::new(".").join(dst_rel);

        let src_sha = sha256_file(&src)?;
        if src_sha != entry.sha256 {
            return Err(ThermalError::InvalidSnapshotPath(format!(
                "sha256 mismatch for {}",
                src.display()
            )));
        }

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|source| ThermalError::ArtifactWrite {
                path: parent.display().to_string(),
                source,
            })?;
        }

        fs::copy(&src, &dst).map_err(|source| ThermalError::SnapshotCopy {
            from: src.display().to_string(),
            to: dst.display().to_string(),
            source,
        })?;
    }

    Ok(())
}

fn sha256_file(path: &Path) -> ThermalResult<String> {
    let bytes = fs::read(path).map_err(|source| ThermalError::ConfigRead {
        path: path.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn sanitize_relative_path(input: &str) -> ThermalResult<PathBuf> {
    let p = Path::new(input);
    if p.is_absolute() {
        return Err(ThermalError::InvalidSnapshotPath(input.to_string()));
    }

    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return Err(ThermalError::InvalidSnapshotPath(input.to_string())),
        }
    }

    if out.as_os_str().is_empty() {
        return Err(ThermalError::InvalidSnapshotPath(input.to_string()));
    }

    Ok(out)
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

fn virtual_room_temp(name: &str, all_temps: &HashMap<String, f64>) -> Option<f64> {
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

fn room_energy_balance(
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

    let q_rad = 0.0; // cooldown calibration assumes mwt=0
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

/// Full energy balance including radiator heat input from actual MWT.
/// Returns net heat flow into room in Watts (positive = warming).
#[allow(clippy::too_many_arguments)]
fn full_room_energy_balance(
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
        100.0 // BODY_HEAT_ACTIVE_W
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

fn external_loss(elements: &[ExternalElement], room_temp: f64, outside_temp: f64) -> f64 {
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

fn ventilation_loss(
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

fn wall_conduction(ua: f64, temp_a: f64, temp_b: f64) -> f64 {
    ua * (temp_a - temp_b)
}

fn doorway_exchange(door: &Doorway, temp_a: f64, temp_b: f64, doorway_cd: f64) -> f64 {
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

    // "chimney" doorways are now modelled explicitly as buoyancy links
    // (hall↔landing and landing↔top-landing proxy), not disabled.
    let flow =
        (doorway_cd / 3.0) * width * (DOORWAY_G * door.height.powi(3) * dt.abs() / t_mean).sqrt();

    flow * AIR_DENSITY * AIR_CP * dt
}

#[allow(dead_code)]
/// Solar gain through glazing in Watts.
///
/// All irradiance inputs are for **vertical** surfaces on their respective orientations.
/// Tilt corrections applied per-element:
///   - vertical: 1.0× (reference)
///   - sloping (~45°): 1.4× (more exposure than vertical at UK latitudes in March)
///   - horizontal: uses ne_horiz directly (already computed for horizontal plane)
///
/// sw_vert: W/m² on vertical SW surface (from PV, corrected to vertical reference).
/// ne_vert: W/m² on vertical NE surface (from Open-Meteo solar geometry).
/// ne_horiz: W/m² on horizontal surface (from Open-Meteo, for conservatory roof etc).
fn solar_gain_full(solar: &[SolarGlazingDef], sw_vert: f64, ne_vert: f64, ne_horiz: f64) -> f64 {
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
                // SE placeholder: average of SW and NE until SE sensor is installed
                ("SE", "vertical") => (sw_vert + ne_vert) / 2.0,
                ("SE", _) => (sw_vert + ne_vert) / 2.0,
                _ => ne_vert,
            };
            irr * sg.area * sg.g_value * sg.shading
        })
        .sum()
}

/// Convert PV power (W, negative = generating) to SW **vertical** irradiance (W/m²).
///
/// The PV panels sit on elvina's sloping roof (~45°, SW-facing).
/// Calibration factor 0.087 W/m² per W of PV was fitted against elvina's temp response
/// including her sloping velux — so it gives irradiance on the **sloping PV plane**.
///
/// Sloping surfaces receive ~1.4× more than vertical at these latitudes/season.
/// We convert to vertical as the reference, since `solar_gain_full` selects the
/// correct surface irradiance for each glazing element's actual tilt.
const PV_TO_SLOPING_IRRADIANCE: f64 = 0.087;
const SLOPING_TO_VERTICAL_RATIO: f64 = 1.4;

fn pv_to_sw_vertical_irradiance(pv_watts: f64) -> f64 {
    // PV reports negative when generating
    let gen = (-pv_watts).max(0.0);
    gen * PV_TO_SLOPING_IRRADIANCE / SLOPING_TO_VERTICAL_RATIO
}

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
            solar: r
                .solar
                .into_iter()
                .map(|s| SolarGlazingDef {
                    area: s.area,
                    orientation: leak(s.orientation),
                    tilt: leak(s.tilt),
                    g_value: s.g_value,
                    shading: s.shading,
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
