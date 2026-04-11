use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use axum::extract::State;
use axum::routing::{get, post};
use axum::Json;
use chrono::{DateTime, Datelike, Local, NaiveTime, Timelike, Utc, Weekday};
use clap::{Parser, Subcommand, ValueEnum};
use heatpump_analysis::octopus_tariff::CachedTariffWindows;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "adaptive-heating-mvp")]
#[command(about = "Adaptive heating V2 — model-predictive control via VRC 700")]
struct Cli {
    #[arg(long, default_value = "model/adaptive-heating-mvp.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the V2 service
    Run,
    /// Restore the known-good baseline immediately
    RestoreBaseline,
    /// Print current runtime snapshot (structured by default; use --human for operator view)
    Status {
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
    },
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    ebusd_host: String,
    ebusd_port: u16,
    influx_url: String,
    influx_org: String,
    influx_bucket: String,
    influx_token_env: String,
    #[serde(default = "default_influx_token_credential")]
    influx_token_credential: String,
    http_bind: String,
    state_file: PathBuf,
    jsonl_log_file: PathBuf,
    /// Path to thermal_geometry.json (for live solver)
    #[serde(default = "default_geometry_path")]
    geometry_path: PathBuf,
    /// Where to cache the Octopus tariff window structure (JSON).
    /// Refreshed automatically when older than 12 hours.
    #[serde(default = "default_tariff_cache_path")]
    tariff_cache_path: PathBuf,
    control_every_seconds: u64,
    sample_every_seconds: u64,
    startup_grace_seconds: u64,
    baseline: Baseline,
    topics: Topics,
    dhw: DhwConfig,
    #[serde(default)]
    model: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Baseline {
    hc1_heat_curve: f64,
    z1_day_temp: f64,
    z1_night_temp: f64,
    hwc_temp_desired: f64,
    z1_op_mode: String,
    hwc_op_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Topics {
    leather_temp: String,
    aldora_temp: String,
    /// Multical T1: cylinder top / hot water outlet temperature
    #[serde(default = "default_dhw_t1_topic")]
    dhw_t1: String,
    /// Powerwall state of charge from energy-hub Tesla collector.
    #[serde(default = "default_tesla_soc_topic")]
    tesla_soc_pct: String,
    /// Powerwall instantaneous battery power (+ve = discharging into home).
    #[serde(default = "default_tesla_battery_power_topic")]
    tesla_battery_w: String,
    /// Whole-home demand at the Powerwall boundary.
    #[serde(default = "default_tesla_home_power_topic")]
    tesla_home_w: String,
    /// Explicit discretionary battery headroom signal from energy-hub.
    #[serde(default = "default_tesla_headroom_topic")]
    tesla_headroom_to_next_cosy_kwh: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DhwConfig {
    cosy_windows: Vec<TimeWindow>,
    /// Peak-rate windows (highest rate tier, e.g. 16:00–19:00 on Cosy).
    /// Populated at runtime from Octopus API; empty when not yet fetched.
    #[serde(default)]
    peak_windows: Vec<TimeWindow>,
    charge_trigger_c: f64,
    target_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimeWindow {
    start: String,
    end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfig {
    /// Heat curve formula exponent (default 1.25)
    #[serde(default = "default_exponent")]
    heat_curve_exponent: f64,
    /// Target leather temperature during waking hours
    #[serde(default = "default_target_leather")]
    target_leather_c: f64,
    /// VRC 700 setpoint — must match Z1NightTemp since we run in night mode
    #[serde(default = "default_setpoint")]
    setpoint_c: f64,
    /// Typical system ΔT (flow - return), used for MWT→flow conversion
    #[serde(default = "default_delta_t")]
    default_delta_t_c: f64,
    /// Minimum curve change before writing to eBUS (outer loop)
    #[serde(default = "default_curve_deadband")]
    curve_deadband: f64,
    /// Waking hours start (HH:MM)
    #[serde(default = "default_waking_start")]
    waking_start: String,
    /// Waking hours end (HH:MM)
    #[serde(default = "default_waking_end")]
    waking_end: String,
    /// Overnight curve (minimum, let house cool)
    #[serde(default = "default_overnight_curve")]
    overnight_curve: f64,
    /// Hours before waking to start heating (Phase 1 fixed, Phase 2 will calculate)
    #[serde(default = "default_preheat_hours")]
    preheat_hours: f64,
    /// Inner loop: proportional gain (curve units per °C error)
    #[serde(default = "default_inner_loop_gain")]
    inner_loop_gain: f64,
    /// Inner loop: deadband — no adjustment if |error| < this (°C)
    #[serde(default = "default_inner_loop_deadband")]
    inner_loop_deadband_c: f64,
    /// Inner loop: max curve step per tick
    #[serde(default = "default_inner_loop_max_step")]
    inner_loop_max_step: f64,
    /// Open-Meteo forecast URL
    #[serde(default = "default_forecast_url")]
    forecast_url: String,
    /// Forecast cache lifetime in seconds
    #[serde(default = "default_forecast_cache_secs")]
    forecast_cache_secs: u64,
}

fn default_exponent() -> f64 {
    1.25
}
fn default_target_leather() -> f64 {
    20.5
}
fn default_setpoint() -> f64 {
    19.0
} // Phase 1a: Z1NightTemp since Z1OpMode=night
fn default_delta_t() -> f64 {
    4.0
}
fn default_curve_deadband() -> f64 {
    0.05
}
fn default_waking_start() -> String {
    "07:00".to_string()
}
fn default_waking_end() -> String {
    "23:00".to_string()
}
fn default_overnight_curve() -> f64 {
    0.10
}
fn default_preheat_hours() -> f64 {
    3.0
}
fn default_inner_loop_gain() -> f64 {
    0.10
}
fn default_inner_loop_deadband() -> f64 {
    0.5
}
fn default_inner_loop_max_step() -> f64 {
    0.20
}
fn default_geometry_path() -> PathBuf {
    PathBuf::from("data/canonical/thermal_geometry.json")
}
fn default_dhw_t1_topic() -> String {
    "emon/multical/dhw_t1".to_string()
}
fn default_tesla_soc_topic() -> String {
    "emon/tesla/soc_pct".to_string()
}
fn default_tesla_battery_power_topic() -> String {
    "emon/tesla/battery_W".to_string()
}
fn default_tesla_home_power_topic() -> String {
    "emon/tesla/home_W".to_string()
}
fn default_tesla_headroom_topic() -> String {
    "emon/tesla/discretionary_headroom_to_next_cosy_kWh".to_string()
}
fn default_tariff_cache_path() -> PathBuf {
    dirs_or_fallback().join("tariff-windows.json")
}
fn dirs_or_fallback() -> PathBuf {
    // Prefer XDG state dir, fall back to ~/.local/state/<app>
    std::env::var("STATE_DIRECTORY")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local/state/adaptive-heating-mvp"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/adaptive-heating-mvp"))
}
fn default_dhw_cosy_windows() -> Vec<TimeWindow> {
    // Fallback used only when the Octopus API is unreachable at startup.
    // Normally overridden at runtime from CachedTariffWindows.
    vec![
        TimeWindow {
            start: "04:00".to_string(),
            end: "07:00".to_string(),
        },
        TimeWindow {
            start: "13:00".to_string(),
            end: "16:00".to_string(),
        },
        TimeWindow {
            start: "22:00".to_string(),
            end: "23:59".to_string(),
        },
    ]
}
fn default_forecast_url() -> String {
    "https://api.open-meteo.com/v1/forecast?latitude=51.611&longitude=-0.108&hourly=temperature_2m,relative_humidity_2m,direct_radiation&forecast_hours=24&timezone=Europe/London".to_string()
}
fn default_forecast_cache_secs() -> u64 {
    3600
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            heat_curve_exponent: default_exponent(),
            target_leather_c: default_target_leather(),
            setpoint_c: default_setpoint(),
            default_delta_t_c: default_delta_t(),
            curve_deadband: default_curve_deadband(),
            waking_start: default_waking_start(),
            waking_end: default_waking_end(),
            overnight_curve: default_overnight_curve(),
            preheat_hours: default_preheat_hours(),
            inner_loop_gain: default_inner_loop_gain(),
            inner_loop_deadband_c: default_inner_loop_deadband(),
            inner_loop_max_step: default_inner_loop_max_step(),
            forecast_url: default_forecast_url(),
            forecast_cache_secs: default_forecast_cache_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// Weather forecast (Open-Meteo)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ForecastHour {
    time: String,
    hour: u32,
    temperature_c: f64,
    humidity_pct: f64,
    direct_radiation_w_m2: f64,
}

#[derive(Debug, Clone)]
struct ForecastCache {
    hours: Vec<ForecastHour>,
    fetched_at: Instant,
}

fn fetch_forecast(client: &Client, url: &str) -> Result<Vec<ForecastHour>> {
    let resp = client.get(url).timeout(Duration::from_secs(10)).send()?;
    let body: serde_json::Value = resp.json()?;
    let times = body["hourly"]["time"]
        .as_array()
        .context("no hourly.time")?;
    let temps = body["hourly"]["temperature_2m"]
        .as_array()
        .context("no temperature_2m")?;
    let rhs = body["hourly"]["relative_humidity_2m"]
        .as_array()
        .context("no humidity")?;
    let rads = body["hourly"]["direct_radiation"]
        .as_array()
        .context("no radiation")?;

    let mut hours = Vec::new();
    for i in 0..times.len() {
        let time_str = times[i].as_str().unwrap_or_default().to_string();
        let hour: u32 = time_str
            .get(11..13)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        hours.push(ForecastHour {
            time: time_str,
            hour,
            temperature_c: temps[i].as_f64().unwrap_or(10.0),
            humidity_pct: rhs[i].as_f64().unwrap_or(75.0),
            direct_radiation_w_m2: rads[i].as_f64().unwrap_or(0.0),
        });
    }
    Ok(hours)
}

/// Convert Open-Meteo direct_radiation (horizontal, W/m²) to approximate
/// SW vertical irradiance. Factor ~0.7 for UK latitude in heating season.
fn horizontal_to_sw_vertical(direct_rad: f64) -> f64 {
    (direct_rad * 0.7).max(0.0)
}

// ---------------------------------------------------------------------------
// Heat curve formula
// ---------------------------------------------------------------------------

/// Inverse: target_flow + outside → required curve
fn curve_for_flow(target_flow: f64, setpoint: f64, outside: f64, exponent: f64) -> f64 {
    let delta = (setpoint - outside).max(0.01);
    let curve = (target_flow - setpoint) / delta.powf(exponent);
    curve.max(CURVE_FLOOR)
}

/// VRC 700 effective minimum heat curve value
const CURVE_FLOOR: f64 = 0.10;
/// VRC 700 maximum practical curve
const CURVE_CEILING: f64 = 4.00;
/// Warn if curve exceeds this
const CURVE_WARN_THRESHOLD: f64 = 1.50;
/// Above the VRC setpoint the inverse heat-curve formula becomes ill-conditioned.
/// In that region the outer loop seeds a known-safe baseline curve and lets the
/// inner loop/black-box readback handle any real residual demand.
const WARM_END_FORMULA_DISABLE_MARGIN_C: f64 = 0.0;

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn clamp_curve(v: f64) -> f64 {
    v.clamp(CURVE_FLOOR, CURVE_CEILING)
}

fn should_defer_outer_curve_reset(
    current_curve: f64,
    target_curve: f64,
    target_flow: f64,
    flow_desired: Option<f64>,
    deadband_c: f64,
) -> bool {
    let fd = match flow_desired {
        Some(v) if v >= 1.0 => v,
        _ => return false,
    };

    target_curve < current_curve && fd < (target_flow - deadband_c)
}

// ---------------------------------------------------------------------------
// Runtime state & online error correction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Occupied,
    ShortAbsence,
    AwayUntil,
    Disabled,
    MonitorOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeState {
    mode: Mode,
    away_until: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    last_reason: String,
    /// Phase 1a: target flow temp set by outer loop, consumed by inner loop
    #[serde(default)]
    target_flow_c: Option<f64>,
    /// Last outside temp used for model calculation
    #[serde(default)]
    last_calc_outside_c: Option<f64>,
    /// True when Z1OpMode=off for coast (needs restore to night before heating)
    #[serde(default)]
    heating_off: bool,
    /// Last weekday for which the controller rewrote the morning DHW timer.
    #[serde(default)]
    last_dhw_timer_weekday: Option<String>,
    /// Whether the last rewritten morning DHW timer kept the 04:00–07:00 window.
    #[serde(default)]
    last_dhw_timer_morning_enabled: Option<bool>,
    /// Last active DHW scheduler slot already launched via HwcSFMode=load.
    #[serde(default)]
    last_dhw_scheduler_slot: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            mode: Mode::Occupied,
            away_until: None,
            updated_at: Utc::now(),
            last_reason: "default startup".to_string(),
            target_flow_c: None,
            last_calc_outside_c: None,
            heating_off: false,
            last_dhw_timer_weekday: None,
            last_dhw_timer_morning_enabled: None,
            last_dhw_scheduler_slot: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct StatusResponse {
    mode: Mode,
    away_until: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    last_reason: String,
    target_flow_c: Option<f64>,
}

#[derive(Debug, Serialize)]
struct StatusSnapshot {
    runtime: RuntimeState,
    service: StatusService,
    heating: StatusHeating,
    dhw: StatusDhw,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StatusService {
    state_file: String,
    jsonl_log_file: String,
    runtime_age_minutes: i64,
}

#[derive(Debug, Serialize)]
struct StatusHeating {
    current_curve: Option<f64>,
    target_flow_c: Option<f64>,
    actual_flow_desired_c: Option<f64>,
    actual_flow_c: Option<f64>,
    return_c: Option<f64>,
    outside_c: Option<f64>,
    leather_c: Option<f64>,
    aldora_c: Option<f64>,
    run_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusDhw {
    t1_c: Option<f64>,
    hwc_storage_c: Option<f64>,
    battery_soc_pct: Option<f64>,
    battery_power_w: Option<f64>,
    battery_home_w: Option<f64>,
    battery_headroom_to_next_cosy_kwh: Option<f64>,
    target_c: f64,
    trigger_c: f64,
    likely_active: bool,
}

#[derive(Debug, Deserialize)]
struct AwayRequest {
    return_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ServiceState {
    config: Config,
    runtime: Arc<Mutex<RuntimeState>>,
}

// ---------------------------------------------------------------------------
// Decision log
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct DecisionLog {
    ts: DateTime<Utc>,
    mode: Mode,
    tariff_period: String,
    leather_temp_c: Option<f64>,
    aldora_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    hwc_storage_temp_c: Option<f64>,
    /// Multical T1: actual hot water outlet temperature at cylinder top
    dhw_t1_c: Option<f64>,
    hwc_mode: Option<String>,
    battery_soc_pct: Option<f64>,
    battery_power_w: Option<f64>,
    battery_home_w: Option<f64>,
    battery_headroom_to_next_cosy_kwh: Option<f64>,
    battery_adequate_to_next_cosy: Option<bool>,
    run_status: Option<String>,
    compressor_util: Option<f64>,
    elec_consumption_w: Option<f64>,
    yield_power_kw: Option<f64>,
    flow_desired_c: Option<f64>,
    flow_actual_c: Option<f64>,
    return_actual_c: Option<f64>,
    curve_before: Option<f64>,
    curve_after: Option<f64>,
    // V2 model fields (outer loop)
    target_flow_c: Option<f64>,
    forecast_outside_c: Option<f64>,
    forecast_solar_w_m2: Option<f64>,
    model_required_mwt: Option<f64>,
    model_required_flow: Option<f64>,
    model_required_curve: Option<f64>,
    action: String,
    reason: String,
    write_results: Vec<String>,
}

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

fn default_influx_token_credential() -> String {
    "influx_token".to_string()
}

fn default_config() -> Config {
    Config {
        ebusd_host: "127.0.0.1".to_string(),
        ebusd_port: 8888,
        influx_url: "http://127.0.0.1:8086".to_string(),
        influx_org: "home".to_string(),
        influx_bucket: "energy".to_string(),
        influx_token_env: "INFLUX_TOKEN".to_string(),
        influx_token_credential: default_influx_token_credential(),
        http_bind: "0.0.0.0:3031".to_string(),
        state_file: PathBuf::from("/home/jack/.local/state/adaptive-heating-mvp/state.toml"),
        jsonl_log_file: PathBuf::from("/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl"),
        geometry_path: default_geometry_path(),
        control_every_seconds: 900,
        sample_every_seconds: 60,
        startup_grace_seconds: 120,
        baseline: Baseline {
            hc1_heat_curve: 0.55,
            z1_day_temp: 21.0,
            z1_night_temp: 19.0,
            hwc_temp_desired: 45.0,
            z1_op_mode: "auto".to_string(),
            hwc_op_mode: "auto".to_string(),
        },
        topics: Topics {
            leather_temp: "emon/emonth2_23/temperature".to_string(),
            aldora_temp: "zigbee2mqtt/aldora_temp_humid".to_string(),
            dhw_t1: default_dhw_t1_topic(),
            tesla_soc_pct: default_tesla_soc_topic(),
            tesla_battery_w: default_tesla_battery_power_topic(),
            tesla_home_w: default_tesla_home_power_topic(),
            tesla_headroom_to_next_cosy_kwh: default_tesla_headroom_topic(),
        },
        dhw: DhwConfig {
            cosy_windows: default_dhw_cosy_windows(),
            peak_windows: Vec::new(),
            charge_trigger_c: 40.0,
            target_c: 45.0,
        },
        tariff_cache_path: default_tariff_cache_path(),
        model: ModelConfig::default(),
    }
}

fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let cfg = default_config();
        fs::write(path, toml::to_string_pretty(&cfg)?)?;
        return Ok(cfg);
    }
    let raw = fs::read_to_string(path)?;
    Ok(toml::from_str(&raw)?)
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn load_runtime_state(path: &Path) -> Result<RuntimeState> {
    if !path.exists() {
        return Ok(RuntimeState::default());
    }
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

fn save_runtime_state(path: &Path, state: &RuntimeState) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, toml::to_string_pretty(state)?)?;
    Ok(())
}

/// Cached InfluxDB token (resolved once, reused for all queries).
static INFLUX_TOKEN_CACHE: OnceLock<String> = OnceLock::new();

/// Get InfluxDB token (cached after first call).
///
/// Production (systemd): prefer a credential loaded via `LoadCredential=` and exposed at
/// `$CREDENTIALS_DIRECTORY/<credential-name>`. Legacy env-var injection remains supported as a
/// fallback for manual runs.
///
/// Development: falls back to `ak get influxdb` (GPG-encrypted keystore on dev machine).
/// This fallback will fail on pi5data if ak is not installed — that's intentional.
fn influx_token(config: &Config) -> Result<String> {
    if let Some(cached) = INFLUX_TOKEN_CACHE.get() {
        return Ok(cached.clone());
    }
    let token = resolve_influx_token(config)?;
    let _ = INFLUX_TOKEN_CACHE.set(token.clone());
    Ok(token)
}

fn resolve_influx_token(config: &Config) -> Result<String> {
    let env_name = &config.influx_token_env;

    if let Ok(v) = std::env::var(env_name) {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }

    let env_file_name = format!("{env_name}_FILE");
    if let Ok(path) = std::env::var(&env_file_name) {
        if let Ok(token) = read_token_file(Path::new(&path)) {
            return Ok(token);
        }
    }

    if let Ok(dir) = std::env::var("CREDENTIALS_DIRECTORY") {
        let path = Path::new(&dir).join(&config.influx_token_credential);
        if let Ok(token) = read_token_file(&path) {
            return Ok(token);
        }
    }

    warn!(
        "{} not available via env, *_FILE, or systemd credential — falling back to 'ak get influxdb' (dev mode)",
        env_name
    );
    let output = Command::new("ak")
        .arg("get")
        .arg("influxdb")
        .output()
        .context(format!(
            "{} not available via env, *_FILE, or systemd credential and 'ak get influxdb' failed. \
             Production: configure LoadCredential={}:/path/to/token or set {}_FILE",
            env_name, config.influx_token_credential, env_name
        ))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{} not available via env, *_FILE, or systemd credential and 'ak get influxdb' returned error. \
             Production: configure LoadCredential={}:/path/to/token or set {}_FILE",
            env_name,
            config.influx_token_credential,
            env_name
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn read_token_file(path: &Path) -> Result<String> {
    let token = fs::read_to_string(path)?.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("token file {} is empty", path.display()));
    }
    Ok(token)
}

// ---------------------------------------------------------------------------
// eBUS communication
// ---------------------------------------------------------------------------

fn ebusd_command(config: &Config, cmd: &str) -> Result<String> {
    let addr = format!("{}:{}", config.ebusd_host, config.ebusd_port);
    let socket = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("could not resolve ebusd address"))?;
    let mut stream = TcpStream::connect_timeout(&socket, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                if !bytes.is_empty() {
                    break;
                }
                return Err(err.into());
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

fn ebusd_read(config: &Config, circuit: &str, reg: &str) -> Result<String> {
    ebusd_command(config, &format!("read -f -c {circuit} {reg}"))
}

fn ebusd_write(config: &Config, circuit: &str, reg: &str, value: &str) -> Result<String> {
    ebusd_command(config, &format!("write -c {circuit} {reg} {value}"))
}

fn parse_f64(s: Result<String>) -> Option<f64> {
    s.ok()?.trim().parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// InfluxDB queries
// ---------------------------------------------------------------------------

fn query_latest_topic_value(
    client: &Client,
    config: &Config,
    topic: &str,
    field: &str,
    lookback: &str,
) -> Result<Option<f64>> {
    let token = influx_token(config)?;
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: {}) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\") |> last() |> keep(columns: [\"_value\"])",
        config.influx_bucket, lookback, topic, field
    );
    query_single_value(client, config, &token, &flux)
}

fn query_latest_room_temp(client: &Client, config: &Config, topic: &str) -> Result<Option<f64>> {
    let field = if topic == "emon/emonth2_23/temperature" {
        "value"
    } else {
        "temperature"
    };
    query_latest_topic_value(client, config, topic, field, "-2h")
}

/// Query latest DHW T1 (cylinder top) from InfluxDB Multical data.
/// Uses _field="value" (emon measurement format, not zigbee).
fn query_latest_dhw_t1(client: &Client, config: &Config) -> Result<Option<f64>> {
    query_latest_topic_value(client, config, &config.topics.dhw_t1, "value", "-2h")
}

fn query_single_value(
    client: &Client,
    config: &Config,
    token: &str,
    flux: &str,
) -> Result<Option<f64>> {
    let resp = client
        .post(format!(
            "{}/api/v2/query?org={}",
            config.influx_url, config.influx_org
        ))
        .header("Authorization", format!("Token {}", token))
        .header("Content-Type", "application/vnd.flux")
        .header("Accept", "application/csv")
        .body(flux.to_string())
        .send()?;
    let body = resp.text()?;
    let mut headers: Vec<String> = Vec::new();
    let mut val = None;
    for line in body.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if headers.is_empty() {
            headers = fields.iter().map(|s| s.to_string()).collect();
            continue;
        }
        if let Some(i) = headers.iter().position(|h| h == "_value") {
            val = fields.get(i).and_then(|s| s.parse::<f64>().ok());
        }
    }
    Ok(val)
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn write_jsonl(path: &Path, entry: &DecisionLog) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, entry)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn write_influx_decision(client: &Client, config: &Config, entry: &DecisionLog) -> Result<()> {
    let token = influx_token(config)?;
    let mode = format!("{:?}", entry.mode).to_lowercase();
    let action = entry.action.replace(' ', "_");
    let fields: Vec<String> = [
        influx_field("leather_temp_c", entry.leather_temp_c),
        influx_field("aldora_temp_c", entry.aldora_temp_c),
        influx_field("outside_temp_c", entry.outside_temp_c),
        influx_field("hwc_storage_temp_c", entry.hwc_storage_temp_c),
        influx_field("dhw_t1_c", entry.dhw_t1_c),
        influx_field("battery_soc_pct", entry.battery_soc_pct),
        influx_field("battery_power_w", entry.battery_power_w),
        influx_field("battery_home_w", entry.battery_home_w),
        influx_field(
            "battery_headroom_to_next_cosy_kwh",
            entry.battery_headroom_to_next_cosy_kwh,
        ),
        entry
            .battery_adequate_to_next_cosy
            .map(|v| format!("battery_adequate_to_next_cosy={}", if v { 1 } else { 0 })),
        influx_field("compressor_util", entry.compressor_util),
        influx_field("elec_consumption_w", entry.elec_consumption_w),
        influx_field("yield_power_kw", entry.yield_power_kw),
        influx_field("flow_desired_c", entry.flow_desired_c),
        influx_field("flow_actual_c", entry.flow_actual_c),
        influx_field("return_actual_c", entry.return_actual_c),
        influx_field("curve_before", entry.curve_before),
        influx_field("curve_after", entry.curve_after),
        influx_field("target_flow_c", entry.target_flow_c),
        influx_field("forecast_outside_c", entry.forecast_outside_c),
        influx_field("forecast_solar_w_m2", entry.forecast_solar_w_m2),
        influx_field("model_required_mwt", entry.model_required_mwt),
        influx_field("model_required_flow", entry.model_required_flow),
        influx_field("model_required_curve", entry.model_required_curve),
    ]
    .into_iter()
    .flatten()
    .collect();
    if fields.is_empty() {
        return Ok(());
    }
    let line = format!(
        "adaptive_heating_mvp,mode={},action={},tariff={} {} {}",
        mode,
        action,
        entry.tariff_period.replace(' ', "_"),
        fields.join(","),
        entry.ts.timestamp()
    );
    client
        .post(format!(
            "{}/api/v2/write?org={}&bucket={}&precision=s",
            config.influx_url, config.influx_org, config.influx_bucket
        ))
        .header("Authorization", format!("Token {}", token))
        .body(line)
        .send()?;
    Ok(())
}

fn influx_field(name: &str, v: Option<f64>) -> Option<String> {
    v.map(|x| format!("{name}={x}"))
}

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

fn within_window(now: NaiveTime, window: &TimeWindow) -> bool {
    match (parse_time(&window.start), parse_time(&window.end)) {
        (Some(s), Some(e)) => now >= s && now <= e,
        _ => false,
    }
}

fn sorted_cosy_windows(config: &Config) -> Vec<TimeWindow> {
    let mut windows = config.dhw.cosy_windows.clone();
    windows.sort_by_key(|window| parse_time(&window.start));
    windows
}

fn is_waking_hours(model: &ModelConfig, now: NaiveTime) -> bool {
    match (
        parse_time(&model.waking_start),
        parse_time(&model.waking_end),
    ) {
        (Some(s), Some(e)) => now >= s && now < e,
        _ => true, // default to always waking if parse fails
    }
}

/// Classify current tariff period for logging/observability.
/// All window times come from the API-derived config (Cosy + peak).
fn classify_tariff_period(config: &Config, now: NaiveTime) -> String {
    let cosy = sorted_cosy_windows(config);
    for (idx, window) in cosy.iter().enumerate() {
        if within_window(now, window) {
            return match idx {
                0 => "cosy_morning",
                1 => "cosy_afternoon",
                2 => "cosy_evening",
                _ => "cosy",
            }
            .to_string();
        }
    }
    // Check peak windows (highest-rate tier, e.g. 16:00–19:00 on Cosy tariff)
    let mut peak: Vec<&TimeWindow> = config.dhw.peak_windows.iter().collect();
    peak.sort_by_key(|w| parse_time(&w.start));
    for window in &peak {
        if within_window(now, window) {
            return "peak".to_string();
        }
    }
    "standard".to_string()
}

fn hours_until_time(now: NaiveTime, target: NaiveTime) -> f64 {
    let now_minutes = now.hour() as f64 * 60.0 + now.minute() as f64;
    let target_minutes = target.hour() as f64 * 60.0 + target.minute() as f64;
    if target_minutes >= now_minutes {
        (target_minutes - now_minutes) / 60.0
    } else {
        (1440.0 - now_minutes + target_minutes) / 60.0
    }
}

fn predict_t1_at_time(current_t1_c: f64, now: NaiveTime, target: NaiveTime) -> f64 {
    current_t1_c - hours_until_time(now, target) * DHW_T1_DECAY_C_PER_H
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn target_dhw_timer_weekday(now: DateTime<Local>, waking: NaiveTime) -> Weekday {
    if now.time() < waking {
        now.weekday()
    } else {
        now.weekday().succ()
    }
}

fn morning_dhw_windows_enabled(config: &Config) -> Vec<TimeWindow> {
    let waking = parse_time(&config.model.waking_start)
        .unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap());
    sorted_cosy_windows(config)
        .into_iter()
        .filter(|window| parse_time(&window.end) != Some(waking))
        .collect()
}

fn dhw_timer_payload(config: &Config, morning_enabled: bool) -> String {
    let mut windows = sorted_cosy_windows(config);
    if !morning_enabled {
        windows = morning_dhw_windows_enabled(config);
    }

    let mut parts = Vec::with_capacity(6);
    for window in windows.into_iter().take(3) {
        parts.push(window.start);
        parts.push(window.end);
    }
    while parts.len() < 6 {
        parts.push("-:-".to_string());
    }
    parts.join(";")
}

fn sync_morning_dhw_timer(
    config: &Config,
    state: &mut RuntimeState,
    now: DateTime<Local>,
    dhw_t1: Option<f64>,
) -> Result<Option<String>> {
    let waking = parse_time(&config.model.waking_start)
        .unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap());
    let Some(t1) = dhw_t1 else {
        return Ok(None);
    };

    let predicted_t1 = predict_t1_at_time(t1, now.time(), waking);
    let morning_enabled = predicted_t1 < config.dhw.charge_trigger_c;
    let weekday = target_dhw_timer_weekday(now, waking);
    let weekday_name = weekday_name(weekday).to_string();

    if state.last_dhw_timer_weekday.as_deref() == Some(weekday_name.as_str())
        && state.last_dhw_timer_morning_enabled == Some(morning_enabled)
    {
        return Ok(None);
    }

    let payload = dhw_timer_payload(config, morning_enabled);
    let register = format!("HwcTimer_{weekday_name}");
    let result = ebusd_write(config, "700", &register, &payload)?;

    // Only update dedup state if the write didn't return an error.
    // On failure, we must retry on the next tick.
    let write_ok = !result.to_uppercase().contains("ERR:");
    if write_ok {
        state.last_dhw_timer_weekday = Some(weekday_name.clone());
        state.last_dhw_timer_morning_enabled = Some(morning_enabled);
    } else {
        warn!(
            "DHW timer write failed ({}), will retry next tick: {}",
            result, register
        );
        // Clear dedup state so next tick retries
        state.last_dhw_timer_weekday = None;
        state.last_dhw_timer_morning_enabled = None;
    }

    Ok(Some(format!(
        "{}={} -> {} (predicted T1 at {} {:.1}°C => morning window {})",
        register,
        payload,
        result,
        waking.format("%H:%M"),
        predicted_t1,
        if morning_enabled {
            "enabled"
        } else {
            "skipped"
        }
    )))
}

// ---------------------------------------------------------------------------
// Reinitialize eBUS for active control (same as startup sequence)
// ---------------------------------------------------------------------------

fn reinitialize_ebus(config: &Config) -> Result<Vec<String>> {
    let mut results = Vec::new();
    results.push(format!(
        "Z1OpMode=night -> {}",
        ebusd_write(config, "700", "Z1OpMode", "night")?
    ));
    results.push(format!(
        "Hc1MinFlowTempDesired=19 -> {}",
        ebusd_write(config, "700", "Hc1MinFlowTempDesired", "19")?
    ));
    Ok(results)
}

// Baseline restore (Phase 1a: only Z1OpMode + Hc1HeatCurve)
// ---------------------------------------------------------------------------

fn restore_baseline(config: &Config) -> Result<Vec<String>> {
    let mut results = Vec::new();
    results.push(format!(
        "Hc1HeatCurve={} -> {}",
        config.baseline.hc1_heat_curve,
        ebusd_write(
            config,
            "700",
            "Hc1HeatCurve",
            &config.baseline.hc1_heat_curve.to_string()
        )?
    ));
    results.push(format!(
        "Z1OpMode={} -> {}",
        config.baseline.z1_op_mode,
        ebusd_write(config, "700", "Z1OpMode", &config.baseline.z1_op_mode)?
    ));
    // Restore MinFlowTempDesired to VRC 700 default
    results.push(format!(
        "Hc1MinFlowTempDesired=20 -> {}",
        ebusd_write(config, "700", "Hc1MinFlowTempDesired", "20")?
    ));
    results.push(format!(
        "HwcSFMode=auto -> {}",
        ebusd_write(config, "700", "HwcSFMode", "auto")?
    ));
    let payload = dhw_timer_payload(config, true);
    for weekday in [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ] {
        let register = format!("HwcTimer_{}", weekday_name(weekday));
        results.push(format!(
            "{}={} -> {}",
            register,
            payload,
            ebusd_write(config, "700", &register, &payload)?
        ));
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Forecast management
// ---------------------------------------------------------------------------

fn get_forecast_for_hour(
    client: &Client,
    config: &Config,
    cache: &Arc<Mutex<Option<ForecastCache>>>,
    target_hour: u32,
) -> Option<ForecastHour> {
    let mut guard = cache.lock().unwrap();

    // Check if cache is still valid
    let needs_refresh = match &*guard {
        None => true,
        Some(fc) => fc.fetched_at.elapsed().as_secs() > config.model.forecast_cache_secs,
    };

    if needs_refresh {
        match fetch_forecast(client, &config.model.forecast_url) {
            Ok(hours) => {
                info!("forecast refreshed: {} hours", hours.len());
                *guard = Some(ForecastCache {
                    hours,
                    fetched_at: Instant::now(),
                });
            }
            Err(e) => {
                warn!("forecast fetch failed: {e}");
                // Keep stale cache if available
            }
        }
    }

    guard
        .as_ref()
        .and_then(|fc| fc.hours.iter().find(|h| h.hour == target_hour).cloned())
}

// ---------------------------------------------------------------------------
// V2 model-predictive control
// ---------------------------------------------------------------------------

struct ModelCalculation {
    forecast_outside_c: Option<f64>,
    forecast_solar_w_m2: Option<f64>,
    required_mwt: Option<f64>,
    required_flow: Option<f64>,
    required_curve: Option<f64>,
    reason: String,
}

/// Core V2 calculation: forecast conditions → required flow temp and initial curve.
///
/// End-to-end: target_leather → required MWT (live solver) → required flow (MWT + ΔT/2)
///             → required curve (heat curve formula, initial guess only)
fn calculate_required_curve_for_target(
    config: &Config,
    target_leather_c: f64,
    outside_temp: f64,
    live_delta_t: Option<f64>,
    forecast: Option<&ForecastHour>,
) -> ModelCalculation {
    let model = &config.model;

    // Use forecast if available, else fall back to live outside temp
    let (effective_outside, effective_solar, source) = match forecast {
        Some(fh) => (
            fh.temperature_c,
            horizontal_to_sw_vertical(fh.direct_radiation_w_m2),
            "forecast",
        ),
        None => (outside_temp, 0.0, "live"),
    };

    // Step 1: Solve for required MWT using thermal physics model
    let required_mwt = match heatpump_analysis::thermal::bisect_mwt_for_room(
        "leather",
        target_leather_c,
        effective_outside,
        effective_solar, // irr_sw
        0.0,             // irr_ne (not available from forecast)
    ) {
        Ok(mwt) => mwt,
        Err(e) => {
            warn!("thermal solver failed: {}, falling back to no-MWT", e);
            None
        }
    };

    // Step 2: MWT → flow temp
    let delta_t = live_delta_t.unwrap_or(model.default_delta_t_c);
    let required_flow = required_mwt.map(|mwt| mwt + delta_t / 2.0);

    // Step 3: flow → curve (initial guess via formula; inner loop will converge)
    let mut warm_end_curve_fallback = false;
    let required_curve = required_flow.map(|flow| {
        let outside_gap_c = model.setpoint_c - effective_outside;
        let curve = if outside_gap_c <= WARM_END_FORMULA_DISABLE_MARGIN_C {
            warm_end_curve_fallback = true;
            config.baseline.hc1_heat_curve
        } else {
            curve_for_flow(
                flow,
                model.setpoint_c,
                effective_outside,
                model.heat_curve_exponent,
            )
        };
        round2(clamp_curve(curve))
    });

    let reason = format!(
        "target={:.1}°C {} outside={:.1}°C solar={:.0}W/m² → MWT={} flow={} curve={}{}",
        target_leather_c,
        source,
        effective_outside,
        effective_solar,
        required_mwt
            .map(|v| format!("{:.1}", v))
            .unwrap_or("N/A".into()),
        required_flow
            .map(|v| format!("{:.1}", v))
            .unwrap_or("N/A".into()),
        required_curve
            .map(|v| format!("{:.2}", v))
            .unwrap_or("N/A".into()),
        if warm_end_curve_fallback {
            format!(
                " (warm-end fallback: outside {:.1}°C ≥ setpoint {:.1}°C, using baseline seed)",
                effective_outside, model.setpoint_c
            )
        } else {
            String::new()
        },
    );

    ModelCalculation {
        forecast_outside_c: Some(effective_outside),
        forecast_solar_w_m2: Some(effective_solar),
        required_mwt,
        required_flow,
        required_curve,
        reason,
    }
}

fn calculate_required_curve(
    config: &Config,
    outside_temp: f64,
    live_delta_t: Option<f64>,
    forecast: Option<&ForecastHour>,
) -> ModelCalculation {
    calculate_required_curve_for_target(
        config,
        config.model.target_leather_c,
        outside_temp,
        live_delta_t,
        forecast,
    )
}

/// Overnight target is the bottom of the comfort band.
/// Coast is free; once leather reaches this floor the thermal solver
/// holds it there at minimum flow → minimum electrical input.
const OVERNIGHT_COMFORT_FLOOR_OFFSET_C: f64 = 0.5;
/// Deadband for coast→heat transition.  Sensor resolution is 0.1°C;
/// 0.15°C avoids hunting on sensor noise without wasting coast time.
const OVERNIGHT_COAST_MARGIN_C: f64 = 0.15;
const DHW_NORMAL_ELEC_KWH: f64 = 2.4;
const DHW_ECO_ELEC_KWH: f64 = 1.9;

#[derive(Debug, Clone)]
struct BatteryHeadroom {
    discretionary_headroom_kwh: f64,
    dhw_event_kwh: f64,
    adequate_to_next_cosy: bool,
}

#[derive(Debug, Clone)]
struct DhwScheduleDecision {
    slot_key: String,
    launch_now: bool,
    battery_adequate_to_next_cosy: Option<bool>,
    reason: String,
}

/// Overnight target: comfort-band floor (flat, not a ramp).
///
/// Physics rationale (Pontryagin / lumped-capacitance):
///   Total electrical cost = ∫ Q_hp / COP(T_flow) dt.
///   Heat-pump COP degrades with flow temperature.  Higher Q_hp
///   requires higher flow → worse COP.  A linear ramp back-loads the
///   hardest temperature rise into the final hours, demanding high
///   flow just when the room's exponential approach (τ ≈ 50 h) is
///   slowest — and the code can never catch up.
///
///   Minimum-electrical strategy:
///     1. Coast for free while leather > comfort floor  (Q = 0)
///     2. Hold the comfort floor at equilibrium flow    (lowest flow,
///        best COP, indefinitely sustainable)
///     3. At waking_start, step to midband target —
///        the daytime solver handles the 0.5°C lift.
///
///   Simulation (τ = 50 h, UA = 190 W/K, outside 9.5°C):
///     Linear ramp → 20.5:    2.82 kWh electrical
///     Coast → hold 20.0:     1.86 kWh electrical  (−34%)
fn overnight_target_leather(model: &ModelConfig, now: NaiveTime) -> f64 {
    let waking_start = parse_time(&model.waking_start)
        .unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap());
    let waking_end =
        parse_time(&model.waking_end).unwrap_or_else(|| NaiveTime::from_hms_opt(23, 0, 0).unwrap());

    if now >= waking_start && now < waking_end {
        return model.target_leather_c;
    }

    // Flat overnight target = bottom of comfort band.
    // Coast mechanism handles T > this (free).  Thermal solver
    // handles T <= this (minimum flow = minimum electrical input).
    (model.target_leather_c - OVERNIGHT_COMFORT_FLOOR_OFFSET_C).max(model.setpoint_c)
}

fn should_coast_overnight(
    model: &ModelConfig,
    now: NaiveTime,
    current_leather_c: f64,
    outside_c: f64,
    target_leather_c: f64,
) -> bool {
    if is_waking_hours(model, now) {
        return false;
    }
    if outside_c < 2.0 {
        return false;
    }
    if hours_until_time(
        now,
        parse_time(&model.waking_start)
            .unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap()),
    ) <= 0.5
    {
        return false;
    }
    current_leather_c >= target_leather_c + OVERNIGHT_COAST_MARGIN_C
}

fn estimate_dhw_event_kwh(hwc_mode: Option<&str>) -> f64 {
    if hwc_mode
        .map(|mode| mode.to_lowercase().contains("eco"))
        .unwrap_or(false)
    {
        DHW_ECO_ELEC_KWH
    } else {
        DHW_NORMAL_ELEC_KWH
    }
}

fn assess_battery_headroom(
    discretionary_headroom_kwh: Option<f64>,
    hwc_mode: Option<&str>,
) -> Option<BatteryHeadroom> {
    let discretionary_headroom_kwh = discretionary_headroom_kwh?;
    let dhw_event_kwh = estimate_dhw_event_kwh(hwc_mode);
    let adequate_to_next_cosy = discretionary_headroom_kwh >= dhw_event_kwh;

    Some(BatteryHeadroom {
        discretionary_headroom_kwh,
        dhw_event_kwh,
        adequate_to_next_cosy,
    })
}

/// Map current time to a DHW scheduling slot.
/// Cosy windows: 04-07, 13-16, 22-00 (Octopus Cosy tariff, UK local time).
/// Non-Cosy overnight: 00-04. Verify: ~/github/energy-hub/scripts/octopus-tariff-windows.sh
fn current_dhw_slot(config: &Config, now: NaiveTime) -> Option<&'static str> {
    let windows = sorted_cosy_windows(config);
    let morning = windows.first()?;
    let afternoon = windows.get(1)?;
    let evening = windows.get(2)?;
    let morning_start = parse_time(&morning.start)?;

    if within_window(now, evening) {
        Some("evening_bank")
    } else if now < morning_start {
        Some("overnight_battery")
    } else if within_window(now, morning) {
        Some("cosy_morning")
    } else if within_window(now, afternoon) {
        Some("afternoon_fallback")
    } else {
        None
    }
}

fn plan_dhw_schedule(
    config: &Config,
    now: DateTime<Local>,
    outside_c: Option<f64>,
    dhw_t1: Option<f64>,
    hwc_storage_c: Option<f64>,
    hwc_mode: Option<&str>,
    battery: Option<&BatteryHeadroom>,
) -> Option<DhwScheduleDecision> {
    let slot = current_dhw_slot(config, now.time())?;
    let waking = parse_time(&config.model.waking_start)
        .unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap());
    let predicted_t1 = dhw_t1.map(|t1| predict_t1_at_time(t1, now.time(), waking));
    let trigger = config.dhw.charge_trigger_c;
    let t1_needs_charge = predicted_t1.map(|t1| t1 < trigger);
    let storage_needs_charge = hwc_storage_c.map(|t| t < trigger - 2.0);
    let needs_charge = t1_needs_charge.or(storage_needs_charge).unwrap_or(false);
    if !needs_charge {
        return None;
    }

    let outside = outside_c.unwrap_or(6.0);
    let deficit = predicted_t1
        .map(|t1| (trigger - t1).max(0.0))
        .unwrap_or(0.0);
    let eco_mode = hwc_mode
        .map(|mode| mode.to_lowercase().contains("eco"))
        .unwrap_or(false);
    let battery_ok = battery.map(|b| b.adequate_to_next_cosy);
    let launch_now = match slot {
        "evening_bank" => true,
        "overnight_battery" => {
            outside < 2.0 || deficit >= 1.5 || eco_mode || battery_ok == Some(true)
        }
        "cosy_morning" => true,
        "afternoon_fallback" => true,
        _ => false,
    };

    let battery_summary = match battery {
        Some(b) => format!(
            "headroom_signal={:.2}kWh dhw={:.2}kWh adequate={}",
            b.discretionary_headroom_kwh, b.dhw_event_kwh, b.adequate_to_next_cosy,
        ),
        None => "headroom_signal=unavailable".to_string(),
    };

    let reason = format!(
        "slot={} predicted_T1@{}={} trigger={:.1}°C outside={:.1}°C hwc_mode={} {}{}",
        slot,
        waking.format("%H:%M"),
        predicted_t1
            .map(|v| format!("{:.1}°C", v))
            .unwrap_or_else(|| "unknown".to_string()),
        trigger,
        outside,
        hwc_mode.unwrap_or("unknown"),
        battery_summary,
        if launch_now {
            " → launch now"
        } else {
            " → wait for next Cosy slot"
        }
    );

    Some(DhwScheduleDecision {
        slot_key: format!("{}:{}", now.format("%Y-%m-%d"), slot),
        launch_now,
        battery_adequate_to_next_cosy: battery_ok,
        reason,
    })
}

// ---------------------------------------------------------------------------
/// Cylinder-top T1 standby decay used for morning DHW skip logic.
/// Measured from 47 standby segments (no draws, no charging, ≥2h each,
/// 10-min resolution with Multical flow filtering) over 18 days:
///   mean 0.212, median 0.218, P75 0.234, P90 0.242 °C/h.
/// Use P75 for a slightly pessimistic estimate (better to charge
/// unnecessarily than run cold).
const DHW_T1_DECAY_C_PER_H: f64 = 0.23;

// ---------------------------------------------------------------------------
// Outer loop: model-predictive control (every control_every_seconds = 900s)
// ---------------------------------------------------------------------------

fn run_outer_cycle(
    config: &Config,
    runtime: &Arc<Mutex<RuntimeState>>,
    client: &Client,
    forecast_cache: &Arc<Mutex<Option<ForecastCache>>>,
) -> Result<()> {
    let mut state = runtime.lock().unwrap().clone();
    let now_local = Local::now();
    let now_time = now_local.time();
    let current_hour = now_local.hour();

    // Read all eBUS inputs (full sensor sweep)
    let status = ebusd_read(config, "hmu", "RunDataStatuscode").ok();
    let outside_temp = parse_f64(ebusd_read(config, "700", "DisplayedOutsideTemp"));
    let flow_desired = parse_f64(ebusd_read(config, "700", "Hc1ActualFlowTempDesired"));
    let flow_actual = parse_f64(ebusd_read(config, "hmu", "RunDataFlowTemp"));
    let return_actual = parse_f64(ebusd_read(config, "hmu", "RunDataReturnTemp"));
    let compressor_util = parse_f64(ebusd_read(config, "hmu", "CurrentCompressorUtil"));
    let elec_consumption = parse_f64(ebusd_read(config, "hmu", "RunDataElectricPowerConsumption"));
    let yield_power = parse_f64(ebusd_read(config, "hmu", "CurrentYieldPower"));
    let curve_before = parse_f64(ebusd_read(config, "700", "Hc1HeatCurve"));
    let leather_temp = query_latest_room_temp(client, config, &config.topics.leather_temp)
        .ok()
        .flatten();
    let aldora_temp = query_latest_room_temp(client, config, &config.topics.aldora_temp)
        .ok()
        .flatten();
    let hwc_storage_temp = parse_f64(ebusd_read(config, "700", "HwcStorageTemp"));
    let hwc_mode = ebusd_read(config, "hmu", "HwcMode").ok();
    let dhw_t1 = query_latest_dhw_t1(client, config).ok().flatten();
    let battery_soc_pct = query_latest_topic_value(
        client,
        config,
        &config.topics.tesla_soc_pct,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_power_w = query_latest_topic_value(
        client,
        config,
        &config.topics.tesla_battery_w,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_home_w =
        query_latest_topic_value(client, config, &config.topics.tesla_home_w, "value", "-30m")
            .ok()
            .flatten();
    let battery_headroom_to_next_cosy_kwh = query_latest_topic_value(
        client,
        config,
        &config.topics.tesla_headroom_to_next_cosy_kwh,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_adequacy =
        assess_battery_headroom(battery_headroom_to_next_cosy_kwh, hwc_mode.as_deref());

    let tariff_period = classify_tariff_period(&config, now_time);

    let mut action = "hold".to_string();
    let mut reason = "no rule fired".to_string();
    let mut writes = Vec::new();
    let mut curve_after = curve_before;

    // Model calculation fields for logging
    let mut model_forecast_outside = None;
    let mut model_forecast_solar = None;
    let mut model_required_mwt = None;
    let mut model_required_flow = None;
    let mut model_required_curve = None;

    let is_defrost = status
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains("defrost");
    let is_dhw = status
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains("warm_water");
    let missing_core = leather_temp.is_none() || outside_temp.is_none() || curve_before.is_none();

    // --- DHW service ---
    if state.mode != Mode::Disabled && state.mode != Mode::MonitorOnly {
        if let Some(timer_write) = sync_morning_dhw_timer(config, &mut state, now_local, dhw_t1)? {
            writes.push(timer_write);
        }

        if !is_dhw {
            if let Some(dhw_plan) = plan_dhw_schedule(
                config,
                now_local,
                outside_temp,
                dhw_t1,
                hwc_storage_temp,
                hwc_mode.as_deref(),
                battery_adequacy.as_ref(),
            ) {
                if dhw_plan.launch_now
                    && state.last_dhw_scheduler_slot.as_deref() != Some(dhw_plan.slot_key.as_str())
                {
                    let res = ebusd_write(config, "700", "HwcSFMode", "load")?;
                    action = "dhw_schedule_launch".to_string();
                    reason = dhw_plan.reason.clone();
                    writes.push(format!("HwcSFMode=load -> {}", res));
                    state.last_dhw_scheduler_slot = Some(dhw_plan.slot_key);
                } else if action == "hold" {
                    reason = dhw_plan.reason;
                }
            }
        }
    }

    // --- Heating control (outer loop: set target_flow_c + initial curve guess) ---
    if action == "hold"
        && state.mode != Mode::Disabled
        && state.mode != Mode::MonitorOnly
        && !is_defrost
        && !missing_core
    {
        let outside = outside_temp.unwrap();
        let current_curve = curve_before.unwrap_or(config.baseline.hc1_heat_curve);

        // Phase 1b: ΔT stabilisation — only use live ΔT when compressor is actively
        // heating. When compressor cycles off, flow≈return and live ΔT collapses,
        // causing target_flow to oscillate. Use default_delta_t_c instead.
        let compressor_heating = status
            .as_deref()
            .map(|s| {
                s.to_lowercase().contains("heating") && s.to_lowercase().contains("compressor")
            })
            .unwrap_or(false);
        let live_dt = if compressor_heating {
            match (flow_actual, return_actual) {
                (Some(f), Some(r)) if f > r && (f - r) > 1.0 => Some(f - r),
                _ => None,
            }
        } else {
            None // will fall back to default_delta_t_c in calculate_required_curve
        };

        match state.mode {
            Mode::Occupied => {
                let forecast = get_forecast_for_hour(client, config, forecast_cache, current_hour);
                let leather = leather_temp.unwrap_or(config.model.target_leather_c);
                let waking = is_waking_hours(&config.model, now_time);
                let target_leather = if waking {
                    config.model.target_leather_c
                } else {
                    overnight_target_leather(&config.model, now_time)
                };

                // Always run the model so forecast/target fields are populated
                // in the log even during DHW — prevents "blind" ticks.
                let calc = calculate_required_curve_for_target(
                    config,
                    target_leather,
                    outside,
                    live_dt,
                    forecast.as_ref(),
                );
                model_forecast_outside = calc.forecast_outside_c;
                model_forecast_solar = calc.forecast_solar_w_m2;
                model_required_mwt = calc.required_mwt;
                model_required_flow = calc.required_flow;
                model_required_curve = calc.required_curve;

                if is_dhw {
                    // DHW active — don't write heating registers, but keep
                    // target_flow populated so the inner loop can resume
                    // immediately when DHW finishes.
                    if let Some(target_flow) = calc.required_flow {
                        state.target_flow_c = Some(target_flow);
                    }
                    action = "dhw_active".to_string();
                    reason = format!(
                        "DHW active, model ready: trajectory target {:.1}°C target_flow={}: {}",
                        target_leather,
                        calc.required_flow
                            .map(|v| format!("{:.1}°C", v))
                            .unwrap_or("N/A".into()),
                        calc.reason
                    );
                } else if should_coast_overnight(
                    &config.model,
                    now_time,
                    leather,
                    outside,
                    target_leather,
                ) {
                    state.target_flow_c = None;
                    if !state.heating_off {
                        let res = ebusd_write(config, "700", "Z1OpMode", "off")?;
                        writes.push(format!("Z1OpMode=off -> {}", res));
                        state.heating_off = true;
                    }
                    action = "overnight_coast".to_string();
                    reason = format!(
                        "trajectory target {:.1}°C, leather {:.1}°C, outside {:.1}°C => coast",
                        target_leather, leather, outside
                    );
                } else {
                    if state.heating_off {
                        let res = ebusd_write(config, "700", "Z1OpMode", "night")?;
                        writes.push(format!("Z1OpMode=night (restore from coast) -> {}", res));
                        state.heating_off = false;
                    }

                    if let Some(target_flow) = calc.required_flow {
                        state.target_flow_c = Some(target_flow);

                        if let Some(target_curve) = calc.required_curve {
                            let change = (target_curve - current_curve).abs();
                            let defer_reset = should_defer_outer_curve_reset(
                                current_curve,
                                target_curve,
                                target_flow,
                                flow_desired,
                                config.model.inner_loop_deadband_c,
                            );
                            if change > config.model.curve_deadband && !defer_reset {
                                let res = ebusd_write(
                                    config,
                                    "700",
                                    "Hc1HeatCurve",
                                    &format!("{:.2}", target_curve),
                                )?;
                                writes.push(format!("Hc1HeatCurve={:.2} -> {}", target_curve, res));
                                curve_after = Some(target_curve);
                                action = if waking {
                                    "daytime_model".to_string()
                                } else {
                                    "overnight_model".to_string()
                                };
                                reason = format!(
                                    "trajectory target {:.1}°C target_flow={:.1}°C: {}",
                                    target_leather, target_flow, calc.reason
                                );

                                if target_curve > CURVE_WARN_THRESHOLD {
                                    warn!(
                                        "curve {:.2} exceeds warning threshold {:.2}",
                                        target_curve, CURVE_WARN_THRESHOLD
                                    );
                                }
                            } else {
                                action = "hold".to_string();
                                reason = if defer_reset {
                                    format!(
                                        "trajectory target {:.1}°C target_flow={:.1}°C, deferring curve reset from {:.2} to {:.2} while VRC still wants {:.1}°C (< target): {}",
                                        target_leather,
                                        target_flow,
                                        current_curve,
                                        target_curve,
                                        flow_desired.unwrap_or_default(),
                                        calc.reason
                                    )
                                } else {
                                    format!(
                                        "trajectory target {:.1}°C target_flow={:.1}°C, model curve {:.2} within deadband of current {:.2}: {}",
                                        target_leather,
                                        target_flow,
                                        target_curve,
                                        current_curve,
                                        calc.reason
                                    )
                                };
                            }
                        }
                    } else {
                        action = "hold".to_string();
                        reason = format!(
                            "model returned no target for trajectory {:.1}°C: {}",
                            target_leather, calc.reason
                        );
                    }
                }
            }
            Mode::ShortAbsence => {
                state.target_flow_c = None;
                let desired_curve = round2((current_curve - 0.10).max(CURVE_FLOOR));
                action = "short_absence_setback".to_string();
                reason = "short absence cost bias".to_string();
                if (desired_curve - current_curve).abs() > f64::EPSILON {
                    let res = ebusd_write(
                        config,
                        "700",
                        "Hc1HeatCurve",
                        &format!("{:.2}", desired_curve),
                    )?;
                    writes.push(format!("Hc1HeatCurve={:.2} -> {}", desired_curve, res));
                    curve_after = Some(desired_curve);
                }
            }
            Mode::AwayUntil => {
                let hours_to_return = state
                    .away_until
                    .map(|t| (t - Utc::now()).num_minutes() as f64 / 60.0)
                    .unwrap_or(999.0);
                let (desired_curve, desc) = if hours_to_return > 20.0 {
                    state.target_flow_c = None;
                    (0.30, "deep away setback")
                } else if hours_to_return > 6.0 {
                    state.target_flow_c = None;
                    (0.45, "away warm-up stage 1")
                } else {
                    // Use model for the final approach
                    let forecast =
                        get_forecast_for_hour(client, config, forecast_cache, current_hour);
                    let calc =
                        calculate_required_curve(config, outside, live_dt, forecast.as_ref());
                    let curve = calc
                        .required_curve
                        .unwrap_or(config.baseline.hc1_heat_curve);
                    state.target_flow_c = calc.required_flow;
                    (curve, "away warm-up model")
                };
                action = "away_control".to_string();
                reason = format!("{} ({:.1}h to return)", desc, hours_to_return);
                if (desired_curve - current_curve).abs() > f64::EPSILON {
                    let res = ebusd_write(
                        config,
                        "700",
                        "Hc1HeatCurve",
                        &format!("{:.2}", desired_curve),
                    )?;
                    writes.push(format!("Hc1HeatCurve={:.2} -> {}", desired_curve, res));
                    curve_after = Some(desired_curve);
                }
            }
            Mode::Disabled | Mode::MonitorOnly => {}
        }
    }

    // Save updated state
    state.updated_at = Utc::now();
    {
        let mut guard = runtime.lock().unwrap();
        guard.target_flow_c = state.target_flow_c;
        guard.last_calc_outside_c = model_forecast_outside;
        guard.heating_off = state.heating_off;
        guard.last_dhw_timer_weekday = state.last_dhw_timer_weekday.clone();
        guard.last_dhw_timer_morning_enabled = state.last_dhw_timer_morning_enabled;
        guard.last_dhw_scheduler_slot = state.last_dhw_scheduler_slot.clone();
        save_runtime_state(&config.state_file, &guard)?;
    }

    let entry = DecisionLog {
        ts: Utc::now(),
        mode: state.mode,
        tariff_period,
        leather_temp_c: leather_temp,
        aldora_temp_c: aldora_temp,
        outside_temp_c: outside_temp,
        hwc_storage_temp_c: hwc_storage_temp,
        dhw_t1_c: dhw_t1,
        hwc_mode,
        battery_soc_pct,
        battery_power_w,
        battery_home_w,
        battery_headroom_to_next_cosy_kwh,
        battery_adequate_to_next_cosy: battery_adequacy.as_ref().map(|b| b.adequate_to_next_cosy),
        run_status: status,
        compressor_util,
        elec_consumption_w: elec_consumption,
        yield_power_kw: yield_power,
        flow_desired_c: flow_desired,
        flow_actual_c: flow_actual,
        return_actual_c: return_actual,
        curve_before,
        curve_after,
        target_flow_c: state.target_flow_c,
        forecast_outside_c: model_forecast_outside,
        forecast_solar_w_m2: model_forecast_solar,
        model_required_mwt,
        model_required_flow,
        model_required_curve,
        action,
        reason,
        write_results: writes,
    };

    write_jsonl(&config.jsonl_log_file, &entry)?;
    if let Err(err) = write_influx_decision(client, config, &entry) {
        warn!("failed to write Influx decision log: {err}");
    }
    info!("outer: {}", serde_json::to_string(&entry)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Inner loop: closed-loop curve adjustment (every sample_every_seconds = 60s)
// ---------------------------------------------------------------------------

fn run_inner_cycle(config: &Config, runtime: &Arc<Mutex<RuntimeState>>) -> Result<()> {
    let state = runtime.lock().unwrap().clone();

    // Only run if we have a target flow from the outer loop
    let target_flow = match state.target_flow_c {
        Some(tf) => tf,
        None => return Ok(()), // overnight or no target yet
    };

    if state.mode == Mode::Disabled || state.mode == Mode::MonitorOnly {
        return Ok(());
    }

    // Light eBUS reads — only what the inner loop needs
    let status = ebusd_read(config, "hmu", "RunDataStatuscode").ok();
    let flow_desired = parse_f64(ebusd_read(config, "700", "Hc1ActualFlowTempDesired"));
    let curve_before = parse_f64(ebusd_read(config, "700", "Hc1HeatCurve"));

    let is_defrost = status
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains("defrost");
    let is_dhw = status
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains("warm_water");

    // Don't adjust during DHW or defrost
    if is_dhw || is_defrost {
        return Ok(());
    }

    let (fd, cb) = match (flow_desired, curve_before) {
        (Some(fd), Some(cb)) => (fd, cb),
        _ => return Ok(()), // missing readings, skip
    };

    // When the HP is in standby, Hc1ActualFlowTempDesired reads 0.0.
    // Without this guard the inner loop sees error = target - 0 ≈ 29°C
    // and ramps the curve to 3+ before the next outer tick resets it.
    if fd < 1.0 {
        return Ok(());
    }

    // Proportional feedback: error = target - actual
    let error = target_flow - fd;
    let model = &config.model;

    // Phase 1b: floor guard — near the curve floor, reduce gain and widen deadband
    // to prevent hunting where each 0.01 curve ≈ 0.20°C flow change
    let (effective_gain, effective_deadband) = if cb < 0.25 {
        (
            model.inner_loop_gain * 0.5,
            model.inner_loop_deadband_c * 2.0,
        )
    } else {
        (model.inner_loop_gain, model.inner_loop_deadband_c)
    };

    if error.abs() <= effective_deadband {
        return Ok(()); // within deadband, no adjustment needed
    }

    // Compute adjustment: gain × error, clamped to max step
    let raw_adjustment = effective_gain * error;
    let adjustment = raw_adjustment.clamp(-model.inner_loop_max_step, model.inner_loop_max_step);
    let new_curve = round2(clamp_curve(cb + adjustment));

    // Don't write if curve hasn't actually changed (rounding)
    if (new_curve - cb).abs() < 0.005 {
        return Ok(());
    }

    let res = ebusd_write(config, "700", "Hc1HeatCurve", &format!("{:.2}", new_curve))?;

    if new_curve > CURVE_WARN_THRESHOLD {
        warn!(
            "inner loop: curve {:.2} exceeds warning threshold {:.2}",
            new_curve, CURVE_WARN_THRESHOLD
        );
    }

    info!(
        "inner: target_flow={:.1} flow_desired={:.1} error={:.1} curve {:.2}->{:.2} ({})",
        target_flow, fd, error, cb, new_curve, res
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Control loop (two-loop architecture)
// ---------------------------------------------------------------------------

fn control_loop(
    config: Config,
    runtime: Arc<Mutex<RuntimeState>>,
    forecast_cache: Arc<Mutex<Option<ForecastCache>>>,
) {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("failed to build HTTP client");
    let start = Instant::now();

    // Phase 1a startup: force Z1OpMode=night to disable Optimum Start and day/night transitions.
    // Night mode uses Z1NightTemp=19°C. Gives zero overnight rad output (cleanest "off").
    // Curve runs 0.40-1.24 across -5 to 15°C operating range; inner loop converges regardless.
    // Skip if mode is Disabled — baseline should remain intact.
    {
        let state = runtime.lock().unwrap();
        if state.mode == Mode::Disabled || state.mode == Mode::MonitorOnly {
            info!(
                "startup: mode is {:?}, skipping eBUS initialisation",
                state.mode
            );
        } else {
            drop(state);
            match reinitialize_ebus(&config) {
                Ok(results) => {
                    for r in &results {
                        info!("startup: {}", r);
                    }
                }
                Err(e) => error!("startup: reinitialize_ebus failed: {}", e),
            }
        }
    }

    // Clear DHW timer dedup state so sync_morning_dhw_timer re-evaluates
    // on the first outer tick. Without this, a previous (possibly failed)
    // timer write may suppress the retry indefinitely.
    {
        let mut state = runtime.lock().unwrap();
        state.last_dhw_timer_weekday = None;
        state.last_dhw_timer_morning_enabled = None;
    }

    let mut last_outer = Instant::now() - Duration::from_secs(config.control_every_seconds);

    loop {
        std::thread::sleep(Duration::from_secs(config.sample_every_seconds));

        if start.elapsed().as_secs() < config.startup_grace_seconds {
            continue;
        }

        // Outer loop (every control_every_seconds = 900s)
        if last_outer.elapsed().as_secs() >= config.control_every_seconds {
            last_outer = Instant::now();
            let outer_start = Instant::now();
            if let Err(err) = run_outer_cycle(&config, &runtime, &client, &forecast_cache) {
                error!("outer cycle failed: {err:#}");
            }
            let outer_elapsed = outer_start.elapsed();
            if outer_elapsed.as_secs() > 120 {
                warn!(
                    "outer cycle took {:.0}s (>120s) — possible I/O hang",
                    outer_elapsed.as_secs_f64()
                );
            }
        }

        // Inner loop (every tick = 60s)
        if let Err(err) = run_inner_cycle(&config, &runtime) {
            error!("inner cycle failed: {err:#}");
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP API
// ---------------------------------------------------------------------------

fn build_status_snapshot(config: &Config, runtime: RuntimeState) -> StatusSnapshot {
    let target_flow_c = runtime.target_flow_c;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("status client");

    let current_curve = parse_f64(ebusd_read(config, "700", "Hc1HeatCurve"));
    let actual_flow_desired_c = parse_f64(ebusd_read(config, "700", "Hc1ActualFlowTempDesired"));
    let actual_flow_c = parse_f64(ebusd_read(config, "hmu", "RunDataFlowTemp"));
    let return_c = parse_f64(ebusd_read(config, "hmu", "RunDataReturnTemp"));
    let outside_c = parse_f64(ebusd_read(config, "700", "DisplayedOutsideTemp"));
    let hwc_storage_c = parse_f64(ebusd_read(config, "700", "HwcStorageTemp"));
    let run_status = ebusd_read(config, "hmu", "RunDataStatuscode").ok();

    let leather_c = query_latest_room_temp(&client, config, &config.topics.leather_temp)
        .ok()
        .flatten();
    let aldora_c = query_latest_room_temp(&client, config, &config.topics.aldora_temp)
        .ok()
        .flatten();
    let t1_c = query_latest_dhw_t1(&client, config).ok().flatten();
    let battery_soc_pct = query_latest_topic_value(
        &client,
        config,
        &config.topics.tesla_soc_pct,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_power_w = query_latest_topic_value(
        &client,
        config,
        &config.topics.tesla_battery_w,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_home_w = query_latest_topic_value(
        &client,
        config,
        &config.topics.tesla_home_w,
        "value",
        "-30m",
    )
    .ok()
    .flatten();
    let battery_headroom_to_next_cosy_kwh = query_latest_topic_value(
        &client,
        config,
        &config.topics.tesla_headroom_to_next_cosy_kwh,
        "value",
        "-30m",
    )
    .ok()
    .flatten();

    let likely_active = run_status
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains("warm_water");

    let runtime_age_minutes = (Utc::now() - runtime.updated_at).num_minutes();
    let mut warnings = Vec::new();
    if runtime_age_minutes > 60 {
        warnings.push(format!(
            "runtime state is {} minutes old",
            runtime_age_minutes
        ));
    }
    if leather_c.is_none() {
        warnings.push("leather temperature unavailable".to_string());
    }
    if t1_c.is_none() {
        warnings.push("DHW T1 unavailable".to_string());
    }
    if battery_headroom_to_next_cosy_kwh.is_none() {
        warnings.push("battery headroom signal unavailable".to_string());
    }
    if current_curve.is_none() {
        warnings.push("current heat curve unavailable from eBUS".to_string());
    }

    StatusSnapshot {
        runtime,
        service: StatusService {
            state_file: config.state_file.display().to_string(),
            jsonl_log_file: config.jsonl_log_file.display().to_string(),
            runtime_age_minutes,
        },
        heating: StatusHeating {
            current_curve,
            target_flow_c,
            actual_flow_desired_c,
            actual_flow_c,
            return_c,
            outside_c,
            leather_c,
            aldora_c,
            run_status,
        },
        dhw: StatusDhw {
            t1_c,
            hwc_storage_c,
            battery_soc_pct,
            battery_power_w,
            battery_home_w,
            battery_headroom_to_next_cosy_kwh,
            target_c: config.dhw.target_c,
            trigger_c: config.dhw.charge_trigger_c,
            likely_active,
        },
        warnings,
    }
}

async fn api_status(State(state): State<ServiceState>) -> Json<StatusResponse> {
    let runtime = state.runtime.lock().unwrap().clone();
    Json(StatusResponse {
        mode: runtime.mode,
        away_until: runtime.away_until,
        updated_at: runtime.updated_at,
        last_reason: runtime.last_reason,
        target_flow_c: runtime.target_flow_c,
    })
}

async fn set_mode(
    state: &ServiceState,
    mode: Mode,
    away_until: Option<DateTime<Utc>>,
    reason: &str,
) -> Result<()> {
    let previous_mode = {
        let runtime = state.runtime.lock().unwrap();
        runtime.mode
    };

    // Transitioning from Disabled → active mode: re-run startup eBUS writes
    let is_activating = (previous_mode == Mode::Disabled || previous_mode == Mode::MonitorOnly)
        && mode != Mode::Disabled
        && mode != Mode::MonitorOnly;

    if is_activating {
        match reinitialize_ebus(&state.config) {
            Ok(results) => {
                for r in &results {
                    tracing::info!("reinitialize: {}", r);
                }
            }
            Err(e) => tracing::error!("reinitialize_ebus failed: {}", e),
        }
    }

    let mut runtime = state.runtime.lock().unwrap();
    runtime.mode = mode;
    runtime.away_until = away_until;
    runtime.updated_at = Utc::now();
    runtime.last_reason = reason.to_string();
    // Clear DHW dedup state on mode change so timers re-evaluate
    if is_activating {
        runtime.last_dhw_timer_weekday = None;
        runtime.last_dhw_timer_morning_enabled = None;
    }
    save_runtime_state(&state.config.state_file, &runtime)?;
    Ok(())
}

async fn api_mode_occupied(State(state): State<ServiceState>) -> Json<serde_json::Value> {
    let result = set_mode(&state, Mode::Occupied, None, "HTTP occupied").await;
    Json(serde_json::json!({"ok": result.is_ok(), "mode": "occupied"}))
}

async fn api_mode_short_absence(State(state): State<ServiceState>) -> Json<serde_json::Value> {
    let result = set_mode(&state, Mode::ShortAbsence, None, "HTTP short_absence").await;
    Json(serde_json::json!({"ok": result.is_ok(), "mode": "short_absence"}))
}

async fn api_mode_disabled(State(state): State<ServiceState>) -> Json<serde_json::Value> {
    let result = set_mode(&state, Mode::Disabled, None, "HTTP disabled").await;
    Json(serde_json::json!({"ok": result.is_ok(), "mode": "disabled"}))
}

async fn api_mode_monitor_only(State(state): State<ServiceState>) -> Json<serde_json::Value> {
    let result = set_mode(&state, Mode::MonitorOnly, None, "HTTP monitor_only").await;
    Json(serde_json::json!({"ok": result.is_ok(), "mode": "monitor_only"}))
}

async fn api_mode_away(
    State(state): State<ServiceState>,
    Json(body): Json<AwayRequest>,
) -> Json<serde_json::Value> {
    let result = set_mode(
        &state,
        Mode::AwayUntil,
        Some(body.return_at),
        "HTTP away_until",
    )
    .await;
    Json(
        serde_json::json!({"ok": result.is_ok(), "mode": "away_until", "return_at": body.return_at}),
    )
}

async fn api_kill(State(state): State<ServiceState>) -> Json<serde_json::Value> {
    let current_mode = {
        let runtime = state.runtime.lock().unwrap();
        runtime.mode
    };

    if current_mode == Mode::Disabled {
        // Toggle ON: re-activate (set_mode handles reinitialize_ebus)
        let mode = set_mode(&state, Mode::Occupied, None, "HTTP kill toggle / resume").await;
        Json(serde_json::json!({
            "ok": mode.is_ok(),
            "action": "resumed",
            "mode": "occupied"
        }))
    } else {
        // Toggle OFF: restore baseline + disable
        let restore = restore_baseline(&state.config);
        let mode = set_mode(&state, Mode::Disabled, None, "HTTP kill / baseline restore").await;
        Json(serde_json::json!({
            "ok": restore.is_ok() && mode.is_ok(),
            "action": "killed",
            "restored": restore.unwrap_or_default(),
            "mode": "disabled"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    fn test_config() -> Config {
        default_config()
    }

    // @lat: [[tests#Adaptive heating controller#Overnight target stays at the comfort floor until waking]]
    #[test]
    fn overnight_target_is_flat_comfort_floor() {
        let config = test_config();
        let late_evening = NaiveTime::from_hms_opt(23, 0, 0).unwrap();
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        let prewake = NaiveTime::from_hms_opt(6, 0, 0).unwrap();
        let waking = NaiveTime::from_hms_opt(7, 0, 0).unwrap();

        let expected_floor = config.model.target_leather_c - OVERNIGHT_COMFORT_FLOOR_OFFSET_C;

        // Overnight: flat at comfort floor
        let evening = overnight_target_leather(&config.model, late_evening);
        let mid = overnight_target_leather(&config.model, midnight);
        let pre = overnight_target_leather(&config.model, prewake);
        assert!((evening - expected_floor).abs() < 0.01);
        assert!((mid - expected_floor).abs() < 0.01);
        assert!((pre - expected_floor).abs() < 0.01);

        // Waking: steps to midband target
        let wake = overnight_target_leather(&config.model, waking);
        assert!((wake - config.model.target_leather_c).abs() < 0.01);
    }

    // @lat: [[tests#Adaptive heating controller#Overnight coast requires mild weather and headroom above the floor]]
    #[test]
    fn coast_only_when_above_floor_and_not_cold() {
        let config = test_config();
        let now = NaiveTime::from_hms_opt(1, 0, 0).unwrap();
        let target = overnight_target_leather(&config.model, now);

        // Above target + margin, mild outside => coast
        assert!(should_coast_overnight(
            &config.model,
            now,
            target + OVERNIGHT_COAST_MARGIN_C + 0.2,
            6.0,
            target,
        ));
        // Above target + margin, freezing => no coast (cold protection)
        assert!(!should_coast_overnight(
            &config.model,
            now,
            target + OVERNIGHT_COAST_MARGIN_C + 0.2,
            1.0,
            target,
        ));
        // Below target + margin => no coast (need to heat)
        assert!(!should_coast_overnight(
            &config.model,
            now,
            target + OVERNIGHT_COAST_MARGIN_C - 0.05,
            6.0,
            target,
        ));
    }

    // @lat: [[tests#Adaptive heating controller#Warm-end curve fallback uses the baseline seed]]
    #[test]
    fn warm_end_curve_uses_baseline_seed_above_setpoint() {
        let config = test_config();
        let calc = calculate_required_curve_for_target(&config, 20.5, 24.0, None, None);

        assert_eq!(calc.required_curve, Some(config.baseline.hc1_heat_curve));
        assert!(calc.reason.contains("warm-end fallback"));
    }

    #[test]
    fn cold_weather_curve_still_uses_formula() {
        let config = test_config();
        let calc = calculate_required_curve_for_target(&config, 20.5, 5.0, None, None);

        let curve = calc.required_curve.expect("curve should be present");
        assert_ne!(curve, config.baseline.hc1_heat_curve);
        assert!(!calc.reason.contains("warm-end fallback"));
    }

    // @lat: [[tests#Adaptive heating controller#Outer loop defers downward resets until flow converges]]
    #[test]
    fn outer_loop_defers_downward_curve_reset_while_flow_still_lags() {
        assert!(should_defer_outer_curve_reset(
            1.73,
            1.32,
            28.4,
            Some(26.3),
            0.5
        ));
        assert!(should_defer_outer_curve_reset(
            2.04,
            1.24,
            27.8,
            Some(24.9),
            0.5
        ));
    }

    #[test]
    fn outer_loop_allows_downward_curve_reset_once_flow_has_converged() {
        assert!(!should_defer_outer_curve_reset(
            1.73,
            1.32,
            28.4,
            Some(27.9),
            0.5
        ));
        assert!(!should_defer_outer_curve_reset(
            1.73,
            1.32,
            28.4,
            Some(28.0),
            0.5
        ));
        assert!(!should_defer_outer_curve_reset(
            1.20,
            1.32,
            28.4,
            Some(26.0),
            0.5
        ));
    }

    #[test]
    fn replay_2026_04_09_morning_relearn_cycles_now_defer_resets() {
        let samples = [
            (1.52, 1.13, 28.3, 26.2),
            (1.52, 1.09, 28.3, 25.9),
            (1.62, 1.12, 28.6, 25.8),
            (1.73, 1.32, 28.4, 26.3),
            (2.04, 1.24, 27.8, 24.9),
        ];

        for (current_curve, target_curve, target_flow, flow_desired) in samples {
            assert!(should_defer_outer_curve_reset(
                current_curve,
                target_curve,
                target_flow,
                Some(flow_desired),
                0.5,
            ));
        }
    }

    #[test]
    fn battery_headroom_detects_when_signal_can_cover_dhw_event() {
        let adequacy = assess_battery_headroom(Some(3.2), Some("normal"))
            .expect("headroom signal should produce an adequacy assessment");

        assert!(adequacy.adequate_to_next_cosy);
        assert!(adequacy.discretionary_headroom_kwh > adequacy.dhw_event_kwh);
    }

    #[test]
    fn battery_headroom_detects_when_signal_is_too_low() {
        let adequacy = assess_battery_headroom(Some(1.0), Some("normal"))
            .expect("headroom signal should produce an adequacy assessment");

        assert!(!adequacy.adequate_to_next_cosy);
        assert!(adequacy.discretionary_headroom_kwh < adequacy.dhw_event_kwh);
    }

    // @lat: [[tests#Controller tariff and timer helpers#Tariff period classification follows sorted windows]]
    #[test]
    fn tariff_period_classification_sorts_windows_before_labelling() {
        let mut config = test_config();
        config.dhw.cosy_windows = vec![
            TimeWindow {
                start: "22:00".to_string(),
                end: "23:59".to_string(),
            },
            TimeWindow {
                start: "13:00".to_string(),
                end: "16:00".to_string(),
            },
            TimeWindow {
                start: "04:00".to_string(),
                end: "07:00".to_string(),
            },
        ];
        config.dhw.peak_windows = vec![TimeWindow {
            start: "16:00".to_string(),
            end: "19:00".to_string(),
        }];

        assert_eq!(
            classify_tariff_period(&config, NaiveTime::from_hms_opt(4, 30, 0).unwrap()),
            "cosy_morning"
        );
        assert_eq!(
            classify_tariff_period(&config, NaiveTime::from_hms_opt(14, 0, 0).unwrap()),
            "cosy_afternoon"
        );
        assert_eq!(
            classify_tariff_period(&config, NaiveTime::from_hms_opt(17, 0, 0).unwrap()),
            "peak"
        );
        assert_eq!(
            classify_tariff_period(&config, NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            "standard"
        );
    }

    // @lat: [[tests#Controller tariff and timer helpers#DHW slot mapping respects tariff boundaries]]
    #[test]
    fn current_dhw_slot_maps_expected_tariff_boundaries() {
        let config = test_config();

        assert_eq!(
            current_dhw_slot(&config, NaiveTime::from_hms_opt(0, 30, 0).unwrap()),
            Some("overnight_battery")
        );
        assert_eq!(
            current_dhw_slot(&config, NaiveTime::from_hms_opt(4, 0, 0).unwrap()),
            Some("cosy_morning")
        );
        assert_eq!(
            current_dhw_slot(&config, NaiveTime::from_hms_opt(13, 0, 0).unwrap()),
            Some("afternoon_fallback")
        );
        assert_eq!(
            current_dhw_slot(&config, NaiveTime::from_hms_opt(22, 30, 0).unwrap()),
            Some("evening_bank")
        );
        assert_eq!(
            current_dhw_slot(&config, NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
            None
        );
    }

    // @lat: [[tests#Controller tariff and timer helpers#Morning DHW timer skip uses dash-colon padding]]
    #[test]
    fn dhw_timer_payload_skips_morning_window_and_uses_padding_marker() {
        let config = test_config();

        assert_eq!(
            dhw_timer_payload(&config, true),
            "04:00;07:00;13:00;16:00;22:00;23:59"
        );
        assert_eq!(
            dhw_timer_payload(&config, false),
            "13:00;16:00;22:00;23:59;-:-;-:-"
        );
    }

    // @lat: [[tests#Controller tariff and timer helpers#DHW timer weekday rolls after waking]]
    #[test]
    fn target_dhw_timer_weekday_rolls_after_waking() {
        let waking = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let before = Local.with_ymd_and_hms(2026, 4, 6, 6, 30, 0).unwrap();
        let after = Local.with_ymd_and_hms(2026, 4, 6, 8, 0, 0).unwrap();

        assert_eq!(target_dhw_timer_weekday(before, waking), Weekday::Mon);
        assert_eq!(target_dhw_timer_weekday(after, waking), Weekday::Tue);
    }

    // @lat: [[tests#Controller tariff and timer helpers#T1 prediction wraps across midnight]]
    #[test]
    fn predict_t1_wraps_across_midnight() {
        let now = NaiveTime::from_hms_opt(23, 30, 0).unwrap();
        let target = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let predicted = predict_t1_at_time(45.0, now, target);

        assert!((hours_until_time(now, target) - 7.5).abs() < 1e-9);
        assert!((predicted - (45.0 - 7.5 * DHW_T1_DECAY_C_PER_H)).abs() < 1e-9);
    }

    // @lat: [[tests#Adaptive heating controller#Overnight coast guard near waking]]
    #[test]
    fn overnight_coast_guard_stops_at_half_hour_prewake_boundary() {
        let config = test_config();
        let target =
            overnight_target_leather(&config.model, NaiveTime::from_hms_opt(1, 0, 0).unwrap());

        assert!(should_coast_overnight(
            &config.model,
            NaiveTime::from_hms_opt(6, 29, 0).unwrap(),
            target + OVERNIGHT_COAST_MARGIN_C,
            6.0,
            target,
        ));
        assert!(!should_coast_overnight(
            &config.model,
            NaiveTime::from_hms_opt(6, 30, 0).unwrap(),
            target + OVERNIGHT_COAST_MARGIN_C,
            6.0,
            target,
        ));
    }

    // @lat: [[tests#Adaptive heating controller#Battery headroom threshold depends on DHW mode]]
    #[test]
    fn battery_headroom_threshold_depends_on_dhw_mode() {
        let eco = assess_battery_headroom(Some(DHW_ECO_ELEC_KWH), Some("eco"))
            .expect("eco headroom should produce an adequacy assessment");
        let normal = assess_battery_headroom(Some(DHW_ECO_ELEC_KWH), Some("normal"))
            .expect("normal headroom should produce an adequacy assessment");

        assert!(eco.adequate_to_next_cosy);
        assert_eq!(eco.dhw_event_kwh, DHW_ECO_ELEC_KWH);
        assert!(!normal.adequate_to_next_cosy);
        assert_eq!(normal.dhw_event_kwh, DHW_NORMAL_ELEC_KWH);
    }

    // @lat: [[tests#Adaptive heating controller#Cosy windows ignore battery gating for DHW]]
    #[test]
    fn dhw_scheduler_launches_during_cosy_even_with_low_or_missing_battery_signal() {
        let config = test_config();
        let cases = [
            (
                Local.with_ymd_and_hms(2026, 4, 5, 4, 30, 0).unwrap(),
                assess_battery_headroom(Some(0.1), Some("normal")),
                ":cosy_morning",
            ),
            (
                Local.with_ymd_and_hms(2026, 4, 5, 13, 30, 0).unwrap(),
                None,
                ":afternoon_fallback",
            ),
            (
                Local.with_ymd_and_hms(2026, 4, 5, 22, 30, 0).unwrap(),
                assess_battery_headroom(Some(0.0), Some("normal")),
                ":evening_bank",
            ),
        ];

        for (now, battery, expected_slot_suffix) in cases {
            let plan = plan_dhw_schedule(
                &config,
                now,
                Some(8.0),
                Some(39.0),
                Some(39.0),
                Some("normal"),
                battery.as_ref(),
            )
            .expect("Cosy slots should still produce a DHW plan when charge is needed");

            assert!(plan.launch_now, "expected immediate launch for {}", now);
            assert!(
                plan.slot_key.ends_with(expected_slot_suffix),
                "unexpected slot {}",
                plan.slot_key
            );
            assert!(plan.reason.contains("launch now"));
        }
    }

    proptest! {
        // @lat: [[tests#Adaptive heating controller#Battery headroom adequacy is monotonic]]
        #[test]
        fn battery_headroom_adequacy_is_monotonic(
            mode in prop_oneof![Just("eco"), Just("normal")],
            low in 0.0f64..6.0,
            delta in 0.0f64..6.0,
        ) {
            let high = low + delta;
            let low_assessment = assess_battery_headroom(Some(low), Some(mode))
                .expect("generated headroom should always produce an assessment");
            let high_assessment = assess_battery_headroom(Some(high), Some(mode))
                .expect("generated headroom should always produce an assessment");

            prop_assert!(
                !low_assessment.adequate_to_next_cosy || high_assessment.adequate_to_next_cosy,
                "adequacy regressed for mode={mode}, low={low}, high={high}"
            );
            prop_assert!(
                high_assessment.discretionary_headroom_kwh >= low_assessment.discretionary_headroom_kwh
            );
        }
    }

    // @lat: [[tests#Adaptive heating controller#Overnight battery DHW waits without adequate headroom]]
    #[test]
    fn dhw_scheduler_waits_until_morning_when_battery_cannot_bridge() {
        let config = test_config();
        let now = Local.with_ymd_and_hms(2026, 4, 5, 1, 0, 0).unwrap();
        let battery = assess_battery_headroom(Some(1.0), Some("normal"));
        let plan = plan_dhw_schedule(
            &config,
            now,
            Some(6.0),
            Some(40.6),
            Some(41.0),
            Some("normal"),
            battery.as_ref(),
        )
        .expect("plan should exist when charge is needed");

        assert!(!plan.launch_now);
        assert_eq!(plan.battery_adequate_to_next_cosy, Some(false));
        assert!(plan.reason.contains("wait for next Cosy slot"));
    }

    // @lat: [[tests#Adaptive heating controller#Eco-mode overnight DHW bypasses the battery gate]]
    #[test]
    fn dhw_scheduler_launches_overnight_in_eco_mode_even_without_battery_headroom() {
        let config = test_config();
        let now = Local.with_ymd_and_hms(2026, 4, 5, 1, 0, 0).unwrap();
        let battery = assess_battery_headroom(Some(1.0), Some("eco"));
        let plan = plan_dhw_schedule(
            &config,
            now,
            Some(6.0),
            Some(41.2),
            Some(41.0),
            Some("eco"),
            battery.as_ref(),
        )
        .expect("eco overnight slot should still plan a charge when T1 falls below trigger");

        assert!(plan.launch_now);
        assert_eq!(plan.slot_key, "2026-04-05:overnight_battery");
        assert_eq!(plan.battery_adequate_to_next_cosy, Some(false));
        assert!(plan.reason.contains("hwc_mode=eco"));
        assert!(plan.reason.contains("launch now"));
    }

    // @lat: [[tests#Adaptive heating controller#Overnight battery DHW launches when headroom is sufficient]]
    #[test]
    fn dhw_scheduler_launches_overnight_when_battery_can_bridge() {
        let config = test_config();
        let now = Local.with_ymd_and_hms(2026, 4, 5, 1, 0, 0).unwrap();
        let battery = assess_battery_headroom(Some(3.2), Some("normal"));
        let plan = plan_dhw_schedule(
            &config,
            now,
            Some(6.0),
            Some(40.6),
            Some(41.0),
            Some("normal"),
            battery.as_ref(),
        )
        .expect("plan should exist when charge is needed");

        assert!(plan.launch_now);
        assert_eq!(plan.battery_adequate_to_next_cosy, Some(true));
        assert_eq!(plan.slot_key, "2026-04-05:overnight_battery");
        assert!(plan.reason.contains("launch now"));
    }

    // ----- Phase 1 coverage: pure helpers with zero prior coverage -----

    // @lat: [[tests#Adaptive heating controller#Heat curve inverse returns floor for tiny delta]]
    #[test]
    fn curve_for_flow_clamps_to_floor() {
        // When target_flow < setpoint, raw curve is negative → clamp to floor
        let c = curve_for_flow(15.0, 19.0, 5.0, 1.25);
        assert_eq!(c, CURVE_FLOOR, "negative raw curve must clamp to floor");
        // Verify without clamp it would be negative
        let delta = (19.0_f64 - 5.0).max(0.01);
        let raw = (15.0 - 19.0) / delta.powf(1.25);
        assert!(raw < 0.0, "raw curve should be negative: {raw}");
    }

    // @lat: [[tests#Adaptive heating controller#Heat curve inverse is positive for moderate conditions]]
    #[test]
    fn curve_for_flow_moderate_conditions() {
        // 30°C flow, 19°C setpoint, 5°C outside, exponent 1.25
        let c = curve_for_flow(30.0, 19.0, 5.0, 1.25);
        assert!(c > 0.0 && c < 2.0, "curve {c} should be moderate for typical winter");
    }

    // @lat: [[tests#Adaptive heating controller#Round2 preserves two decimal places]]
    #[test]
    fn round2_preserves_two_decimals() {
        assert_eq!(round2(1.005), 1.0); // banker's rounding edge
        assert_eq!(round2(1.456), 1.46);
        assert_eq!(round2(0.0), 0.0);
    }

    // @lat: [[tests#Adaptive heating controller#Clamp curve stays within floor and ceiling]]
    #[test]
    fn clamp_curve_bounds() {
        assert_eq!(clamp_curve(0.05), CURVE_FLOOR);
        assert_eq!(clamp_curve(5.0), CURVE_CEILING);
        assert_eq!(clamp_curve(0.55), 0.55);
    }

    // @lat: [[tests#Adaptive heating controller#Hours until time wraps across midnight]]
    #[test]
    fn hours_until_time_wraps_midnight() {
        let t23 = NaiveTime::from_hms_opt(23, 0, 0).unwrap();
        let t01 = NaiveTime::from_hms_opt(1, 0, 0).unwrap();
        let t07 = NaiveTime::from_hms_opt(7, 0, 0).unwrap();

        // Same-day: 23→01 should be 2 hours (wraps midnight)
        let h = hours_until_time(t23, t01);
        assert!((h - 2.0).abs() < 0.01, "23:00→01:00 should be 2h, got {h}");

        // Same-day forward: 01→07 should be 6 hours
        let h = hours_until_time(t01, t07);
        assert!((h - 6.0).abs() < 0.01, "01:00→07:00 should be 6h, got {h}");

        // Identity: same time → 0 hours (not 24)
        let h = hours_until_time(t07, t07);
        assert!(h.abs() < 0.01, "same time should be 0h, got {h}");
    }

    // @lat: [[tests#Adaptive heating controller#Solar irradiance conversion is non-negative]]
    #[test]
    fn horizontal_to_sw_vertical_non_negative() {
        assert_eq!(horizontal_to_sw_vertical(0.0), 0.0);
        assert_eq!(horizontal_to_sw_vertical(-10.0), 0.0);
        assert!((horizontal_to_sw_vertical(100.0) - 70.0).abs() < 0.01);
    }

    // @lat: [[tests#Adaptive heating controller#Waking hours detection respects boundaries]]
    #[test]
    fn is_waking_hours_boundaries() {
        let model = test_config().model; // 07:00–23:00
        let t0659 = NaiveTime::from_hms_opt(6, 59, 0).unwrap();
        let t0700 = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let t2259 = NaiveTime::from_hms_opt(22, 59, 0).unwrap();
        let t2300 = NaiveTime::from_hms_opt(23, 0, 0).unwrap();

        assert!(!is_waking_hours(&model, t0659));
        assert!(is_waking_hours(&model, t0700));
        assert!(is_waking_hours(&model, t2259));
        assert!(!is_waking_hours(&model, t2300)); // end is exclusive
    }

    // @lat: [[tests#Adaptive heating controller#DHW energy estimate depends on mode]]
    #[test]
    fn estimate_dhw_event_kwh_depends_on_mode() {
        assert_eq!(estimate_dhw_event_kwh(Some("eco")), DHW_ECO_ELEC_KWH);
        assert_eq!(estimate_dhw_event_kwh(Some("Eco")), DHW_ECO_ELEC_KWH);
        assert_eq!(estimate_dhw_event_kwh(Some("normal")), DHW_NORMAL_ELEC_KWH);
        assert_eq!(estimate_dhw_event_kwh(None), DHW_NORMAL_ELEC_KWH);
    }

    // @lat: [[tests#Controller tariff and timer helpers#T1 prediction wraps across midnight]]
    // Strengthened: also verify the decay rate direction.
    #[test]
    fn predict_t1_decay_direction_and_wrap() {
        let now = NaiveTime::from_hms_opt(23, 0, 0).unwrap();
        let target = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let t1 = predict_t1_at_time(50.0, now, target);
        // 8 hours of decay
        let expected = 50.0 - 8.0 * DHW_T1_DECAY_C_PER_H;
        assert!((t1 - expected).abs() < 0.01, "decay should be {expected}, got {t1}");
        assert!(t1 < 50.0, "T1 should decrease over time");
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "adaptive_heating_mvp=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Run);
    let mut config = load_config(&cli.config)?;

    match command {
        Commands::RestoreBaseline => {
            for line in restore_baseline(&config)? {
                println!("{line}");
            }
            return Ok(());
        }
        Commands::Status { human } => {
            let state = load_runtime_state(&config.state_file)?;
            let config_clone = config.clone();
            let snapshot = std::thread::spawn(move || build_status_snapshot(&config_clone, state))
                .join()
                .map_err(|_| anyhow!("status snapshot thread panicked"))?;
            if human {
                println!("Adaptive heating status");
                println!("-----------------------");
                println!("mode: {:?}", snapshot.runtime.mode);
                println!("updated_at: {}", snapshot.runtime.updated_at);
                println!("last_reason: {}", snapshot.runtime.last_reason);
                if let Some(v) = snapshot.heating.current_curve {
                    println!("current_curve: {:.2}", v);
                }
                if let Some(v) = snapshot.heating.target_flow_c {
                    println!("target_flow_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.actual_flow_desired_c {
                    println!("actual_flow_desired_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.actual_flow_c {
                    println!("actual_flow_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.return_c {
                    println!("return_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.outside_c {
                    println!("outside_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.leather_c {
                    println!("leather_c: {:.1}", v);
                }
                if let Some(v) = snapshot.heating.aldora_c {
                    println!("aldora_c: {:.1}", v);
                }
                if let Some(ref v) = snapshot.heating.run_status {
                    println!("run_status: {v}");
                }
                if let Some(v) = snapshot.dhw.t1_c {
                    println!("dhw_t1_c: {:.1}", v);
                }
                if let Some(v) = snapshot.dhw.hwc_storage_c {
                    println!("hwc_storage_c: {:.1}", v);
                }
                if let Some(v) = snapshot.dhw.battery_soc_pct {
                    println!("battery_soc_pct: {:.1}", v);
                }
                if let Some(v) = snapshot.dhw.battery_power_w {
                    println!("battery_power_w: {:.0}", v);
                }
                if let Some(v) = snapshot.dhw.battery_home_w {
                    println!("battery_home_w: {:.0}", v);
                }
                if let Some(v) = snapshot.dhw.battery_headroom_to_next_cosy_kwh {
                    println!("battery_headroom_to_next_cosy_kwh: {:.2}", v);
                }
                println!(
                    "runtime_age_minutes: {}",
                    snapshot.service.runtime_age_minutes
                );
                if snapshot.warnings.is_empty() {
                    println!("warnings: none");
                } else {
                    println!("warnings:");
                    for warning in &snapshot.warnings {
                        println!("- {warning}");
                    }
                }
            } else {
                println!("{}", toml::to_string_pretty(&snapshot)?);
            }
            return Ok(());
        }
        Commands::Run => {}
    }

    // Validate thermal solver is working (geometry loads OK)
    match heatpump_analysis::thermal::bisect_mwt_for_room("leather", 20.5, 5.0, 0.0, 0.0) {
        Ok(Some(mwt)) => info!(
            "thermal solver OK: leather 20.5°C at 5°C outside → MWT {:.1}°C",
            mwt
        ),
        Ok(None) => warn!("thermal solver: leather 20.5°C not achievable at 5°C (unexpected)"),
        Err(e) => anyhow::bail!("thermal solver failed to load geometry: {}", e),
    }

    // Load tariff window structure from Octopus API (cached; refreshed when >12 h old).
    // Replaces the TOML fallback cosy_windows with account-derived times.
    match CachedTariffWindows::load_or_fetch(
        &config.tariff_cache_path,
        Duration::from_secs(12 * 3600),
    ) {
        Ok(windows) => {
            info!(
                "tariff windows: {} Cosy, {} peak from {} (fetched {})",
                windows.cosy_windows.len(),
                windows.peak_windows.len(),
                windows.tariff_code,
                windows.fetched_at.with_timezone(&Local).format("%H:%M"),
            );
            config.dhw.cosy_windows = windows
                .cosy_windows
                .into_iter()
                .map(|w| TimeWindow {
                    start: w.start,
                    end: w.end,
                })
                .collect();
            config.dhw.peak_windows = windows
                .peak_windows
                .into_iter()
                .map(|w| TimeWindow {
                    start: w.start,
                    end: w.end,
                })
                .collect();
        }
        Err(e) => {
            warn!("tariff window fetch failed, using TOML fallback windows: {e:#}");
        }
    }

    ensure_parent(&config.state_file)?;
    ensure_parent(&config.jsonl_log_file)?;

    let runtime = Arc::new(Mutex::new(load_runtime_state(&config.state_file)?));
    let forecast_cache: Arc<Mutex<Option<ForecastCache>>> = Arc::new(Mutex::new(None));

    let service_state = ServiceState {
        config: config.clone(),
        runtime: runtime.clone(),
    };

    let loop_config = config.clone();
    let loop_runtime = runtime.clone();
    let loop_forecast = forecast_cache.clone();
    std::thread::spawn(move || control_loop(loop_config, loop_runtime, loop_forecast));

    let app = axum::Router::new()
        .route("/status", get(api_status))
        .route("/mode/occupied", post(api_mode_occupied))
        .route("/mode/short-absence", post(api_mode_short_absence))
        .route("/mode/away", post(api_mode_away))
        .route("/mode/disabled", post(api_mode_disabled))
        .route("/mode/monitor-only", post(api_mode_monitor_only))
        .route("/kill", post(api_kill))
        .with_state(service_state);

    let listener = tokio::net::TcpListener::bind(&config.http_bind).await?;
    info!(
        "adaptive-heating-mvp V2 HTTP listening on {}",
        config.http_bind
    );

    let shutdown_config = config.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            warn!("ctrl-c received, restoring baseline");
            let _ = restore_baseline(&shutdown_config);
            std::process::exit(0);
        }
    });

    axum::serve(listener, app).await?;
    Ok(())
}
