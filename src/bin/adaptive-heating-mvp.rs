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
    http_bind: String,
    state_file: PathBuf,
    jsonl_log_file: PathBuf,
    /// Path to thermal_geometry.json (for live solver)
    #[serde(default = "default_geometry_path")]
    geometry_path: PathBuf,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DhwConfig {
    cosy_windows: Vec<TimeWindow>,
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

/// Forward: curve + outside → flow temp
fn flow_for_curve(curve: f64, setpoint: f64, outside: f64, exponent: f64) -> f64 {
    let delta = (setpoint - outside).max(0.01);
    setpoint + curve * delta.powf(exponent)
}

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

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn clamp_curve(v: f64) -> f64 {
    v.clamp(CURVE_FLOOR, CURVE_CEILING)
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
    forecast_cache: Arc<Mutex<Option<ForecastCache>>>,
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

fn default_config() -> Config {
    Config {
        ebusd_host: "127.0.0.1".to_string(),
        ebusd_port: 8888,
        influx_url: "http://127.0.0.1:8086".to_string(),
        influx_org: "home".to_string(),
        influx_bucket: "energy".to_string(),
        influx_token_env: "INFLUX_TOKEN".to_string(),
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
        },
        dhw: DhwConfig {
            cosy_windows: vec![
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
            ],
            charge_trigger_c: 40.0,
            target_c: 45.0,
        },
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
/// Production (systemd): loaded from EnvironmentFile `/etc/adaptive-heating-mvp.env`
/// which sets INFLUX_TOKEN. File is root:root 0600.
///
/// Development: falls back to `ak get influxdb` (GPG-encrypted keystore on dev machine).
/// This fallback will fail on pi5data if ak is not installed — that's intentional.
fn influx_token(env_name: &str) -> Result<String> {
    if let Some(cached) = INFLUX_TOKEN_CACHE.get() {
        return Ok(cached.clone());
    }
    let token = resolve_influx_token(env_name)?;
    let _ = INFLUX_TOKEN_CACHE.set(token.clone());
    Ok(token)
}

fn resolve_influx_token(env_name: &str) -> Result<String> {
    // Primary: environment variable (set by systemd EnvironmentFile)
    if let Ok(v) = std::env::var(env_name) {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    // Fallback: ak keystore (development only)
    warn!(
        "{} not set in environment — falling back to 'ak get influxdb' (dev mode)",
        env_name
    );
    let output = Command::new("ak")
        .arg("get")
        .arg("influxdb")
        .output()
        .context(format!(
            "{} not set and 'ak get influxdb' failed. \
             Production: set {} in /etc/adaptive-heating-mvp.env",
            env_name, env_name
        ))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{} not set and 'ak get influxdb' returned error. \
             Production: set {} in /etc/adaptive-heating-mvp.env",
            env_name,
            env_name
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
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

fn query_latest_room_temp(client: &Client, config: &Config, topic: &str) -> Result<Option<f64>> {
    let token = influx_token(&config.influx_token_env)?;
    let field = if topic == "emon/emonth2_23/temperature" {
        "value"
    } else {
        "temperature"
    };
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: -2h) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"{}\") |> last() |> keep(columns: [\"_value\"])",
        config.influx_bucket, topic, field
    );
    query_single_value(client, config, &token, &flux)
}

/// Query latest DHW T1 (cylinder top) from InfluxDB Multical data.
/// Uses _field="value" (emon measurement format, not zigbee).
fn query_latest_dhw_t1(client: &Client, config: &Config) -> Result<Option<f64>> {
    let token = influx_token(&config.influx_token_env)?;
    let flux = format!(
        "from(bucket: \"{}\") |> range(start: -2h) |> filter(fn: (r) => r.topic == \"{}\" and r._field == \"value\") |> last() |> keep(columns: [\"_value\"])",
        config.influx_bucket, config.topics.dhw_t1
    );
    query_single_value(client, config, &token, &flux)
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
    let token = influx_token(&config.influx_token_env)?;
    let mode = format!("{:?}", entry.mode).to_lowercase();
    let action = entry.action.replace(' ', "_");
    let fields: Vec<String> = [
        influx_field("leather_temp_c", entry.leather_temp_c),
        influx_field("aldora_temp_c", entry.aldora_temp_c),
        influx_field("outside_temp_c", entry.outside_temp_c),
        influx_field("hwc_storage_temp_c", entry.hwc_storage_temp_c),
        influx_field("dhw_t1_c", entry.dhw_t1_c),
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

fn in_any_window(config: &Config, now: NaiveTime) -> bool {
    config
        .dhw
        .cosy_windows
        .iter()
        .any(|w| within_window(now, w))
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

/// Is now within the pre-heat window before waking hours?
fn is_preheat_time(model: &ModelConfig, now: NaiveTime) -> bool {
    if let Some(waking) = parse_time(&model.waking_start) {
        let preheat_mins = (model.preheat_hours * 60.0) as u32;
        // Compute preheat start time (may wrap around midnight)
        let waking_mins = waking.hour() * 60 + waking.minute();
        let preheat_start_mins = if waking_mins >= preheat_mins {
            waking_mins - preheat_mins
        } else {
            1440 + waking_mins - preheat_mins
        };
        let preheat_start =
            NaiveTime::from_hms_opt((preheat_start_mins / 60) % 24, preheat_start_mins % 60, 0)
                .unwrap_or(waking);

        if preheat_start <= waking {
            now >= preheat_start && now < waking
        } else {
            // Wraps midnight
            now >= preheat_start || now < waking
        }
    } else {
        false
    }
}

fn classify_tariff_period(now: NaiveTime) -> String {
    let t = |hh: u32, mm: u32| NaiveTime::from_hms_opt(hh, mm, 0).unwrap();
    if now >= t(4, 0) && now < t(7, 0) {
        "cosy_morning".to_string()
    } else if now >= t(13, 0) && now < t(16, 0) {
        "cosy_afternoon".to_string()
    } else if now >= t(22, 0) {
        "cosy_evening".to_string()
    } else if now >= t(16, 0) && now < t(19, 0) {
        "peak".to_string()
    } else {
        "standard".to_string()
    }
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
    let mut windows: Vec<TimeWindow> = config
        .dhw
        .cosy_windows
        .iter()
        .filter(|window| {
            let end = parse_time(&window.end);
            end != Some(waking)
        })
        .cloned()
        .collect();
    windows.sort_by_key(|window| parse_time(&window.start));
    windows
}

fn dhw_timer_payload(config: &Config, morning_enabled: bool) -> String {
    let mut windows = config.dhw.cosy_windows.clone();
    windows.sort_by_key(|window| parse_time(&window.start));
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

    state.last_dhw_timer_weekday = Some(weekday_name.clone());
    state.last_dhw_timer_morning_enabled = Some(morning_enabled);

    Ok(Some(format!(
        "{}={} -> {} (predicted T1 at {} {:.1}°C => morning window {})",
        register,
        payload,
        result,
        waking.format("%H:%M"),
        predicted_t1,
        if morning_enabled { "enabled" } else { "skipped" }
    )))
}

// ---------------------------------------------------------------------------
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
fn calculate_required_curve(
    config: &Config,
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
        model.target_leather_c,
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
    let required_curve = required_flow.map(|flow| {
        let curve = curve_for_flow(
            flow,
            model.setpoint_c,
            effective_outside,
            model.heat_curve_exponent,
        );
        round2(clamp_curve(curve))
    });

    let reason = format!(
        "{} outside={:.1}°C solar={:.0}W/m² → MWT={} flow={} curve={}",
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

// ---------------------------------------------------------------------------
// Phase 2: Overnight planner
// ---------------------------------------------------------------------------

/// House effective thermal time constant for leather room (hours).
/// Calibrated from Night 1/Night 2 (24-26 Mar 2026).
/// Leather cooling time constant (hours).
/// Empirical: τ≈50h from both calibration nights (n=18) and DHW segments (n=35).
/// Best single overnight (3.9h, Night 2): τ=65.8h.
/// Was 15.0h — wrong by 3.3×, caused planner to never coast.
const LEATHER_TAU_H: f64 = 50.0;

/// Cylinder-top T1 standby decay used for morning DHW skip logic.
/// Measured overnight on clean crossover nights.
const DHW_T1_DECAY_C_PER_H: f64 = 0.25;

/// HP max thermal output (W) — 5kW Arotherm Plus.
const HP_MAX_OUTPUT_W: f64 = 5000.0;

/// Whole-house HTC (W/K).
const HOUSE_HTC_W_K: f64 = 261.0;

/// Simulate leather temperature cooling from `start_temp` over `hours`,
/// given a constant outside temperature. No heating applied.
/// Uses exponential decay toward the no-heating equilibrium.
fn simulate_cooling(start_temp: f64, outside_temp: f64, hours: f64) -> f64 {
    // Equilibrium with no heating: leather tracks toward outside + internal gains
    // Use the solver for accuracy, but as a fast approximation:
    // leather_eq ≈ outside + 2.5°C (internal gains at ~650W, HTC 261)
    // For accuracy we should use the solver, but that's expensive in a loop.
    // Use the fast approximation; the solver is used for MWT calculation.
    let internal_gain_offset = 2.5; // ~650W internal gains / 261 W/K
    let equilibrium = outside_temp + internal_gain_offset;
    equilibrium + (start_temp - equilibrium) * (-hours / LEATHER_TAU_H).exp()
}

/// Estimate hours needed to reheat leather from `start_temp` to `target_temp`
/// at a given outside temperature. Returns None if HP is in deficit.
fn estimate_reheat_hours(start_temp: f64, target_temp: f64, outside_temp: f64) -> Option<f64> {
    if start_temp >= target_temp {
        return Some(0.0);
    }

    // HP surplus at the midpoint temperature (average during reheat)
    let mid_temp = (start_temp + target_temp) / 2.0;
    let heat_loss_at_mid = HOUSE_HTC_W_K * (mid_temp - outside_temp).max(0.0);
    let surplus_w = HP_MAX_OUTPUT_W - heat_loss_at_mid;

    if surplus_w <= 50.0 {
        return None; // HP can't effectively reheat
    }

    // Empirical reheat rate: leather rises at rate proportional to HP surplus.
    // Calibrated from two data points:
    //   1-2 Apr (outside 10°C): leather 19.9→20.5°C in 2h, surplus ~2260W → 0.3°C/h
    //   Night 1 recovery (25 Mar, outside 5-7°C): leather 17.5→20°C in 6h, surplus ~1200W → 0.42°C/h
    // Rate ≈ surplus/7500 °C/h (7500 W per °C/h, accounts for thermal lag + modulation)
    let rate_c_per_h = surplus_w / 7500.0;
    let delta_t = target_temp - start_temp;
    let hours = delta_t / rate_c_per_h;
    Some(hours)
}

/// Overnight plan result.
#[derive(Debug, Clone)]
struct OvernightPlan {
    /// When to start preheat (hours from now, 0 = start immediately)
    preheat_start_hours_from_now: f64,
    /// Curve to use during preheat (from model)
    preheat_curve: f64,
    /// Target flow during preheat
    preheat_target_flow: f64,
    /// Whether to maintain heating overnight (cold nights)
    maintain_heating: bool,
    /// Overnight curve if maintaining heating
    overnight_heating_curve: f64,
    /// Projected leather temp at preheat start
    projected_leather_at_start: f64,
    /// Explanation
    reason: String,
}

/// Plan the overnight heating strategy.
///
/// Inputs:
///   - current_leather: current leather room temp
///   - forecast: hourly forecast from now until 07:00
///   - waking_time: NaiveTime for target (07:00)
///   - config: model config for setpoint, exponent, etc.
///
/// Returns an OvernightPlan with preheat start time and curves.
fn plan_overnight(
    current_leather: f64,
    forecast: &[ForecastHour],
    now: NaiveTime,
    waking_time: NaiveTime,
    config: &ModelConfig,
) -> OvernightPlan {
    let target = config.target_leather_c; // 20.5°C

    // Hours until waking
    let now_mins = now.hour() as f64 * 60.0 + now.minute() as f64;
    let wake_mins = waking_time.hour() as f64 * 60.0 + waking_time.minute() as f64;
    let hours_until_wake = if wake_mins > now_mins {
        (wake_mins - now_mins) / 60.0
    } else {
        (1440.0 - now_mins + wake_mins) / 60.0
    };

    // Get average overnight outside temp from forecast
    let avg_outside = if forecast.is_empty() {
        5.0 // conservative fallback
    } else {
        forecast.iter().map(|f| f.temperature_c).sum::<f64>() / forecast.len() as f64
    };

    // Minimum overnight outside temp (worst case for reheat)
    let min_outside = forecast
        .iter()
        .map(|f| f.temperature_c)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(avg_outside);

    // --- Cold night: HP can't recover, must maintain heating ---
    if min_outside < 2.0 {
        // Solve for the MWT needed to maintain ~19.5°C at this outside temp
        let maintain_target = target - 1.0; // 19.5°C — don't fight for full 20.5
        let mwt = heatpump_analysis::thermal::bisect_mwt_for_room(
            "leather",
            maintain_target,
            min_outside,
            0.0,
            0.0,
        )
        .ok()
        .flatten()
        .unwrap_or(30.0);

        let delta_t = config.default_delta_t_c;
        let flow = mwt + delta_t / 2.0;
        let curve = curve_for_flow(
            flow,
            config.setpoint_c,
            min_outside,
            config.heat_curve_exponent,
        );
        let curve = round2(clamp_curve(curve));

        return OvernightPlan {
            preheat_start_hours_from_now: 0.0,
            preheat_curve: curve,
            preheat_target_flow: flow,
            maintain_heating: true,
            overnight_heating_curve: curve,
            projected_leather_at_start: current_leather,
            reason: format!(
                "cold night (min {:.1}°C < 2°C): maintain heating at curve {:.2} (MWT {:.1}°C) for {:.1}°C",
                min_outside, curve, mwt, maintain_target
            ),
        };
    }

    // --- Mild/cool night: find latest safe preheat start ---
    // Scan backward from waking time in 30-min steps.
    // At each candidate, simulate cooling to that point, then check reheat time.
    let step_h = 0.5; // 30-min resolution
    let mut best_start = 0.0; // default: start now
    let mut best_projected = current_leather;

    // Use the minimum forecast outside temp for reheat estimate (conservative)
    let reheat_outside = min_outside;

    let max_steps = (hours_until_wake / step_h) as usize;
    for i in (0..=max_steps).rev() {
        let coast_hours = i as f64 * step_h;
        let projected = simulate_cooling(current_leather, avg_outside, coast_hours);
        let remaining_hours = hours_until_wake - coast_hours;

        // Can we reheat from projected temp to target in the remaining time?
        if let Some(reheat_h) = estimate_reheat_hours(projected, target, reheat_outside) {
            // Add 30-min safety margin
            if reheat_h + 0.5 <= remaining_hours {
                best_start = coast_hours;
                best_projected = projected;
                break; // Found the latest safe start — maximum coast time
            }
        }
        // If reheat not possible (deficit) or too slow, try earlier start
    }

    // Calculate the preheat curve for the start conditions
    let preheat_outside = min_outside; // conservative
    let mwt = heatpump_analysis::thermal::bisect_mwt_for_room(
        "leather",
        target,
        preheat_outside,
        0.0,
        0.0,
    )
    .ok()
    .flatten()
    .unwrap_or(30.0);
    let delta_t = config.default_delta_t_c;
    let flow = mwt + delta_t / 2.0;
    let curve = curve_for_flow(
        flow,
        config.setpoint_c,
        preheat_outside,
        config.heat_curve_exponent,
    );
    let curve = round2(clamp_curve(curve));

    let preheat_time_str = {
        let start_mins = now_mins + best_start * 60.0;
        let h = ((start_mins / 60.0) as u32) % 24;
        let m = (start_mins % 60.0) as u32;
        format!("{:02}:{:02}", h, m)
    };

    OvernightPlan {
        preheat_start_hours_from_now: best_start,
        preheat_curve: curve,
        preheat_target_flow: flow,
        maintain_heating: false,
        overnight_heating_curve: CURVE_FLOOR, // 0.10 = no heating
        projected_leather_at_start: best_projected,
        reason: format!(
            "coast {:.1}h to {:.1}°C, preheat at {} (MWT {:.1}°C curve {:.2}), \
             avg outside {:.1}°C min {:.1}°C, {:.1}h to reheat",
            best_start,
            best_projected,
            preheat_time_str,
            mwt,
            curve,
            avg_outside,
            min_outside,
            estimate_reheat_hours(best_projected, target, reheat_outside).unwrap_or(99.0),
        ),
    }
}

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
    let dhw_t1 = query_latest_dhw_t1(client, config).ok().flatten();

    let tariff_period = classify_tariff_period(now_time);

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

        let cosy_now = in_any_window(config, now_time);
        if cosy_now && !is_dhw {
            if let Some(storage) = hwc_storage_temp {
                if storage < config.dhw.charge_trigger_c {
                    let res = ebusd_write(config, "700", "HwcSFMode", "load")?;
                    action = "dhw_boost".to_string();
                    reason = format!(
                        "cosy window and HwcStorageTemp {:.1}C below trigger {:.1}C",
                        storage, config.dhw.charge_trigger_c
                    );
                    writes.push(format!("HwcSFMode=load -> {}", res));
                }
            }
        }
    }

    // --- Heating control (outer loop: set target_flow_c + initial curve guess) ---
    if action == "hold"
        && state.mode != Mode::Disabled
        && state.mode != Mode::MonitorOnly
        && !is_dhw
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
                // Get forecast for current hour
                let forecast = get_forecast_for_hour(client, config, forecast_cache, current_hour);

                let waking = is_waking_hours(&config.model, now_time);
                let preheating = is_preheat_time(&config.model, now_time);

                if waking || preheating {
                    // Restore heating if we were coasting with Z1OpMode=off
                    if state.heating_off {
                        let res = ebusd_write(config, "700", "Z1OpMode", "night")?;
                        writes.push(format!("Z1OpMode=night (restore from coast) -> {}", res));
                        state.heating_off = false;
                    }
                    // --- Daytime / preheat: model-predictive control ---
                    let calc =
                        calculate_required_curve(config, outside, live_dt, forecast.as_ref());

                    model_forecast_outside = calc.forecast_outside_c;
                    model_forecast_solar = calc.forecast_solar_w_m2;
                    model_required_mwt = calc.required_mwt;
                    model_required_flow = calc.required_flow;
                    model_required_curve = calc.required_curve;

                    if let Some(target_flow) = calc.required_flow {
                        // Store target_flow for inner loop
                        state.target_flow_c = Some(target_flow);

                        if let Some(target_curve) = calc.required_curve {
                            let change = (target_curve - current_curve).abs();
                            if change > config.model.curve_deadband {
                                // Write the initial curve guess
                                let res = ebusd_write(
                                    config,
                                    "700",
                                    "Hc1HeatCurve",
                                    &format!("{:.2}", target_curve),
                                )?;
                                writes.push(format!("Hc1HeatCurve={:.2} -> {}", target_curve, res));
                                curve_after = Some(target_curve);
                                action = if preheating {
                                    "preheat_model".to_string()
                                } else {
                                    "daytime_model".to_string()
                                };
                                reason =
                                    format!("target_flow={:.1}°C: {}", target_flow, calc.reason);

                                if target_curve > CURVE_WARN_THRESHOLD {
                                    warn!(
                                        "curve {:.2} exceeds warning threshold {:.2}",
                                        target_curve, CURVE_WARN_THRESHOLD
                                    );
                                }
                            } else {
                                action = "hold".to_string();
                                reason = format!("target_flow={:.1}°C, model curve {:.2} within deadband of current {:.2}: {}",
                                    target_flow, target_curve, current_curve, calc.reason);
                            }
                        }
                    } else {
                        action = "hold".to_string();
                        reason = format!("model returned no target: {}", calc.reason);
                    }
                } else {
                    // --- Overnight: Phase 2 planner ---
                    let waking = parse_time(&config.model.waking_start)
                        .unwrap_or(NaiveTime::from_hms_opt(7, 0, 0).unwrap());

                    // Get overnight forecast hours
                    let overnight_forecast: Vec<ForecastHour> = (0..24)
                        .filter_map(|h| {
                            let fh = get_forecast_for_hour(
                                client,
                                config,
                                forecast_cache,
                                (current_hour + h) % 24,
                            );
                            // Only include hours until waking
                            if h as f64 <= 12.0 {
                                fh
                            } else {
                                None
                            }
                        })
                        .collect();

                    let leather = leather_temp.unwrap_or(20.0);
                    let plan = plan_overnight(
                        leather,
                        &overnight_forecast,
                        now_time,
                        waking,
                        &config.model,
                    );

                    // Restore heating if we were coasting with Z1OpMode=off
                    if state.heating_off && (plan.maintain_heating || plan.preheat_start_hours_from_now <= 0.25) {
                        let res = ebusd_write(config, "700", "Z1OpMode", "night")?;
                        writes.push(format!("Z1OpMode=night (restore from coast) -> {}", res));
                        state.heating_off = false;
                    }

                    if plan.maintain_heating {
                        // Cold night: maintain heating, inner loop tracks
                        state.target_flow_c = Some(plan.preheat_target_flow);
                        let target_curve = plan.overnight_heating_curve;
                        if (target_curve - current_curve).abs() > config.model.curve_deadband {
                            let res = ebusd_write(
                                config,
                                "700",
                                "Hc1HeatCurve",
                                &format!("{:.2}", target_curve),
                            )?;
                            writes.push(format!("Hc1HeatCurve={:.2} -> {}", target_curve, res));
                            curve_after = Some(target_curve);
                        }
                        action = "overnight_maintain".to_string();
                        reason = plan.reason;
                    } else if plan.preheat_start_hours_from_now <= 0.25 {
                        // Time to preheat: set target_flow, inner loop tracks
                        state.target_flow_c = Some(plan.preheat_target_flow);
                        let target_curve = plan.preheat_curve;
                        if (target_curve - current_curve).abs() > config.model.curve_deadband {
                            let res = ebusd_write(
                                config,
                                "700",
                                "Hc1HeatCurve",
                                &format!("{:.2}", target_curve),
                            )?;
                            writes.push(format!("Hc1HeatCurve={:.2} -> {}", target_curve, res));
                            curve_after = Some(target_curve);
                        }
                        action = "overnight_preheat".to_string();
                        reason = plan.reason;
                    } else {
                        // Coasting: turn heating OFF, inner loop idle
                        state.target_flow_c = None;
                        if !state.heating_off {
                            // Turn off heating circuit
                            let res = ebusd_write(config, "700", "Z1OpMode", "off")?;
                            writes.push(format!("Z1OpMode=off -> {}", res));
                            state.heating_off = true;
                        }
                        action = "overnight_coast".to_string();
                        reason = format!(
                            "coast {:.1}h then preheat: {}",
                            plan.preheat_start_hours_from_now, plan.reason
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
    info!("startup: setting Z1OpMode=night");
    match ebusd_write(&config, "700", "Z1OpMode", "night") {
        Ok(res) => info!("startup: Z1OpMode=night -> {}", res),
        Err(e) => error!("startup: failed to set Z1OpMode=night: {}", e),
    }

    // Lower MinFlowTempDesired to match SP=19 — removes the 20°C floor
    // that prevented genuine coast (curve 0.10 still produced 20°C+ flow)
    match ebusd_write(&config, "700", "Hc1MinFlowTempDesired", "19") {
        Ok(res) => info!("startup: Hc1MinFlowTempDesired=19 -> {}", res),
        Err(e) => error!("startup: failed to set Hc1MinFlowTempDesired=19: {}", e),
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
    let mut runtime = state.runtime.lock().unwrap();
    runtime.mode = mode;
    runtime.away_until = away_until;
    runtime.updated_at = Utc::now();
    runtime.last_reason = reason.to_string();
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
    let restore = restore_baseline(&state.config);
    let mode = set_mode(&state, Mode::Disabled, None, "HTTP kill / baseline restore").await;
    Json(serde_json::json!({
        "ok": restore.is_ok() && mode.is_ok(),
        "restored": restore.unwrap_or_default(),
        "mode": "disabled"
    }))
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
    let config = load_config(&cli.config)?;

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

    ensure_parent(&config.state_file)?;
    ensure_parent(&config.jsonl_log_file)?;

    let runtime = Arc::new(Mutex::new(load_runtime_state(&config.state_file)?));
    let forecast_cache: Arc<Mutex<Option<ForecastCache>>> = Arc::new(Mutex::new(None));

    let service_state = ServiceState {
        config: config.clone(),
        runtime: runtime.clone(),
        forecast_cache: forecast_cache.clone(),
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
