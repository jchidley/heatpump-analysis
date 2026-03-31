use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use axum::extract::State;
use axum::routing::{get, post};
use axum::Json;
use chrono::{DateTime, Local, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "adaptive-heating-mvp")]
#[command(about = "Adaptive heating MVP for live control via VRC 700")]
struct Cli {
    #[arg(long, default_value = "model/adaptive-heating-mvp.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the MVP service
    Run,
    /// Restore the known-good baseline immediately
    RestoreBaseline,
    /// Print current persisted runtime state
    Status,
}

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
    control_every_seconds: u64,
    sample_every_seconds: u64,
    startup_grace_seconds: u64,
    baseline: Baseline,
    topics: Topics,
    dhw: DhwConfig,
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
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            mode: Mode::Occupied,
            away_until: None,
            updated_at: Utc::now(),
            last_reason: "default startup".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct StatusResponse {
    mode: Mode,
    away_until: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    last_reason: String,
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

#[derive(Debug, Serialize)]
struct DecisionLog {
    ts: DateTime<Utc>,
    mode: Mode,
    tariff_period: String,
    leather_temp_c: Option<f64>,
    aldora_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    hwc_storage_temp_c: Option<f64>,
    run_status: Option<String>,
    compressor_util: Option<f64>,
    elec_consumption_w: Option<f64>,
    yield_power_kw: Option<f64>,
    flow_desired_c: Option<f64>,
    flow_actual_c: Option<f64>,
    return_actual_c: Option<f64>,
    curve_before: Option<f64>,
    curve_after: Option<f64>,
    z1_day_before: Option<f64>,
    z1_day_after: Option<f64>,
    z1_night_before: Option<f64>,
    z1_night_after: Option<f64>,
    hwc_target_before: Option<f64>,
    hwc_target_after: Option<f64>,
    action: String,
    reason: String,
    write_results: Vec<String>,
}

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

fn influx_token(env_name: &str) -> Result<String> {
    if let Ok(v) = std::env::var(env_name) {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let output = Command::new("ak")
        .arg("get")
        .arg("influxdb")
        .output()
        .context("failed to run 'ak get influxdb'")?;
    if !output.status.success() {
        return Err(anyhow!("'ak get influxdb' failed"));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

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
    let reason = entry.reason.replace('"', "'");
    let fields: Vec<String> = [
        influx_field("leather_temp_c", entry.leather_temp_c),
        influx_field("aldora_temp_c", entry.aldora_temp_c),
        influx_field("outside_temp_c", entry.outside_temp_c),
        influx_field("hwc_storage_temp_c", entry.hwc_storage_temp_c),
        influx_field("compressor_util", entry.compressor_util),
        influx_field("elec_consumption_w", entry.elec_consumption_w),
        influx_field("yield_power_kw", entry.yield_power_kw),
        influx_field("flow_desired_c", entry.flow_desired_c),
        influx_field("flow_actual_c", entry.flow_actual_c),
        influx_field("return_actual_c", entry.return_actual_c),
        influx_field("curve_before", entry.curve_before),
        influx_field("curve_after", entry.curve_after),
        influx_field("z1_day_before", entry.z1_day_before),
        influx_field("z1_day_after", entry.z1_day_after),
        influx_field("z1_night_before", entry.z1_night_before),
        influx_field("z1_night_after", entry.z1_night_after),
        influx_field("hwc_target_before", entry.hwc_target_before),
        influx_field("hwc_target_after", entry.hwc_target_after),
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
    let _ = reason; // reserved for future use in line-protocol tag/field split
    Ok(())
}

fn influx_field(name: &str, v: Option<f64>) -> Option<String> {
    v.map(|x| format!("{name}={x}"))
}

fn within_window(now: chrono::NaiveTime, window: &TimeWindow) -> bool {
    let start = chrono::NaiveTime::parse_from_str(&window.start, "%H:%M").ok();
    let end = chrono::NaiveTime::parse_from_str(&window.end, "%H:%M").ok();
    match (start, end) {
        (Some(s), Some(e)) => now >= s && now <= e,
        _ => false,
    }
}

fn in_any_window(config: &Config, now: chrono::NaiveTime) -> bool {
    config
        .dhw
        .cosy_windows
        .iter()
        .any(|w| within_window(now, w))
}

fn classify_tariff_period(now: chrono::NaiveTime) -> String {
    use chrono::NaiveTime;
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
        "Z1DayTemp={} -> {}",
        config.baseline.z1_day_temp,
        ebusd_write(
            config,
            "700",
            "Z1DayTemp",
            &config.baseline.z1_day_temp.to_string()
        )?
    ));
    results.push(format!(
        "Z1NightTemp={} -> {}",
        config.baseline.z1_night_temp,
        ebusd_write(
            config,
            "700",
            "Z1NightTemp",
            &config.baseline.z1_night_temp.to_string()
        )?
    ));
    results.push(format!(
        "HwcTempDesired={} -> {}",
        config.baseline.hwc_temp_desired,
        ebusd_write(
            config,
            "700",
            "HwcTempDesired",
            &config.baseline.hwc_temp_desired.to_string()
        )?
    ));
    results.push(format!(
        "Z1OpMode={} -> {}",
        config.baseline.z1_op_mode,
        ebusd_write(config, "700", "Z1OpMode", &config.baseline.z1_op_mode)?
    ));
    results.push(format!(
        "HwcOpMode={} -> {}",
        config.baseline.hwc_op_mode,
        ebusd_write(config, "700", "HwcOpMode", &config.baseline.hwc_op_mode)?
    ));
    Ok(results)
}

/// VRC 700 effective minimum heat curve value (writes below this read back as 0.10)
const CURVE_FLOOR: f64 = 0.10;

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn run_control_cycle(
    config: &Config,
    runtime: &Arc<Mutex<RuntimeState>>,
    client: &Client,
) -> Result<()> {
    let state = runtime.lock().unwrap().clone();
    let now_local = Local::now();
    let status = ebusd_read(config, "hmu", "RunDataStatuscode").ok();
    let outside_temp = parse_f64(ebusd_read(config, "700", "DisplayedOutsideTemp"));
    let flow_desired = parse_f64(ebusd_read(config, "700", "Hc1ActualFlowTempDesired"));
    let flow_actual = parse_f64(ebusd_read(config, "hmu", "RunDataFlowTemp"));
    let return_actual = parse_f64(ebusd_read(config, "hmu", "RunDataReturnTemp"));
    let compressor_util = parse_f64(ebusd_read(config, "hmu", "CurrentCompressorUtil"));
    let elec_consumption = parse_f64(ebusd_read(config, "hmu", "RunDataElectricPowerConsumption"));
    let yield_power = parse_f64(ebusd_read(config, "hmu", "CurrentYieldPower"));
    let curve_before = parse_f64(ebusd_read(config, "700", "Hc1HeatCurve"));
    let z1_day_before = parse_f64(ebusd_read(config, "700", "Z1DayTemp"));
    let z1_night_before = parse_f64(ebusd_read(config, "700", "Z1NightTemp"));
    let hwc_target_before = parse_f64(ebusd_read(config, "700", "HwcTempDesired"));
    let leather_temp = query_latest_room_temp(client, config, &config.topics.leather_temp)
        .ok()
        .flatten();
    let aldora_temp = query_latest_room_temp(client, config, &config.topics.aldora_temp)
        .ok()
        .flatten();
    let hwc_storage_temp = parse_f64(ebusd_read(config, "700", "HwcStorageTemp"));

    // Determine tariff period
    let tariff_period = classify_tariff_period(now_local.time());

    let mut action = "hold".to_string();
    let mut reason = "no rule fired".to_string();
    let mut writes = Vec::new();
    let mut curve_after = curve_before;
    let mut z1_day_after = z1_day_before;
    let mut z1_night_after = z1_night_before;
    let hwc_target_after = hwc_target_before;

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

    // DHW service first: cosy window is preferred charge opportunity, but only if actually needed.
    if state.mode != Mode::Disabled && state.mode != Mode::MonitorOnly {
        let cosy_now = in_any_window(config, now_local.time());
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

    // Heating control only when not masked by transient conditions.
    if action == "hold"
        && state.mode != Mode::Disabled
        && state.mode != Mode::MonitorOnly
        && !is_dhw
        && !is_defrost
        && !missing_core
    {
        match state.mode {
            Mode::Occupied => {
                let leather = leather_temp.unwrap();
                let current_curve = curve_before.unwrap_or(config.baseline.hc1_heat_curve);
                let mut desired_curve = current_curve;
                let mut desired_day = z1_day_before.unwrap_or(config.baseline.z1_day_temp);
                let desired_night = config.baseline.z1_night_temp;

                if leather < 20.0 {
                    desired_day = 21.0;
                    desired_curve = round2(current_curve + 0.10);
                    action = "heating_recovery".to_string();
                    reason = format!("Leather {:.2}C below comfort band", leather);
                } else if leather > 21.0 {
                    desired_day = 20.0;
                    desired_curve = round2((current_curve - 0.10).max(CURVE_FLOOR));
                    action = "heating_coast".to_string();
                    reason = format!("Leather {:.2}C above comfort band", leather);
                }

                // If no writes would actually change anything, hold instead
                if action != "hold" {
                    let would_change_curve =
                        (desired_curve - current_curve).abs() > 0.001;
                    let would_change_day =
                        (desired_day - z1_day_before.unwrap_or(desired_day)).abs() > 0.001;
                    let would_change_night =
                        (desired_night - z1_night_before.unwrap_or(desired_night)).abs()
                            > 0.001;
                    if !would_change_curve && !would_change_day && !would_change_night {
                        action = "hold".to_string();
                        reason = format!(
                            "Leather {:.2}C outside comfort band but levers already at limit (curve {:.2}, day {:.0})",
                            leather, current_curve, desired_day
                        );
                    }
                }

                if action != "hold" {
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
                    if (desired_day - z1_day_before.unwrap_or(desired_day)).abs() > f64::EPSILON {
                        let res = ebusd_write(
                            config,
                            "700",
                            "Z1DayTemp",
                            &format!("{:.1}", desired_day),
                        )?;
                        writes.push(format!("Z1DayTemp={:.1} -> {}", desired_day, res));
                        z1_day_after = Some(desired_day);
                    }
                    if (desired_night - z1_night_before.unwrap_or(desired_night)).abs()
                        > f64::EPSILON
                    {
                        let res = ebusd_write(
                            config,
                            "700",
                            "Z1NightTemp",
                            &format!("{:.1}", desired_night),
                        )?;
                        writes.push(format!("Z1NightTemp={:.1} -> {}", desired_night, res));
                        z1_night_after = Some(desired_night);
                    }
                }
            }
            Mode::ShortAbsence => {
                let desired_day = 19.0;
                let desired_night = 19.0;
                let current_curve = curve_before.unwrap_or(config.baseline.hc1_heat_curve);
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
                if (desired_day - z1_day_before.unwrap_or(desired_day)).abs() > f64::EPSILON {
                    let res =
                        ebusd_write(config, "700", "Z1DayTemp", &format!("{:.1}", desired_day))?;
                    writes.push(format!("Z1DayTemp={:.1} -> {}", desired_day, res));
                    z1_day_after = Some(desired_day);
                }
                if (desired_night - z1_night_before.unwrap_or(desired_night)).abs() > f64::EPSILON {
                    let res = ebusd_write(
                        config,
                        "700",
                        "Z1NightTemp",
                        &format!("{:.1}", desired_night),
                    )?;
                    writes.push(format!("Z1NightTemp={:.1} -> {}", desired_night, res));
                    z1_night_after = Some(desired_night);
                }
            }
            Mode::AwayUntil => {
                let away_until = state.away_until;
                let hours_to_return = away_until
                    .map(|t| (t - Utc::now()).num_minutes() as f64 / 60.0)
                    .unwrap_or(999.0);
                let (desired_day, desired_curve, desc) = if hours_to_return > 20.0 {
                    (15.0, 0.30, "deep away setback")
                } else if hours_to_return > 6.0 {
                    (18.0, 0.45, "away warm-up stage 1")
                } else {
                    (21.0, 0.55, "away warm-up stage 2")
                };
                action = "away_control".to_string();
                reason = format!("{} ({:.1}h to return)", desc, hours_to_return);
                if (desired_curve - curve_before.unwrap_or(desired_curve)).abs() > f64::EPSILON {
                    let res = ebusd_write(
                        config,
                        "700",
                        "Hc1HeatCurve",
                        &format!("{:.2}", desired_curve),
                    )?;
                    writes.push(format!("Hc1HeatCurve={:.2} -> {}", desired_curve, res));
                    curve_after = Some(desired_curve);
                }
                if (desired_day - z1_day_before.unwrap_or(desired_day)).abs() > f64::EPSILON {
                    let res =
                        ebusd_write(config, "700", "Z1DayTemp", &format!("{:.1}", desired_day))?;
                    writes.push(format!("Z1DayTemp={:.1} -> {}", desired_day, res));
                    z1_day_after = Some(desired_day);
                }
            }
            Mode::Disabled | Mode::MonitorOnly => {}
        }
    }

    let entry = DecisionLog {
        ts: Utc::now(),
        mode: state.mode,
        tariff_period,
        leather_temp_c: leather_temp,
        aldora_temp_c: aldora_temp,
        outside_temp_c: outside_temp,
        hwc_storage_temp_c: hwc_storage_temp,
        run_status: status,
        compressor_util,
        elec_consumption_w: elec_consumption,
        yield_power_kw: yield_power,
        flow_desired_c: flow_desired,
        flow_actual_c: flow_actual,
        return_actual_c: return_actual,
        curve_before,
        curve_after,
        z1_day_before,
        z1_day_after,
        z1_night_before,
        z1_night_after,
        hwc_target_before,
        hwc_target_after,
        action,
        reason,
        write_results: writes,
    };

    write_jsonl(&config.jsonl_log_file, &entry)?;
    if let Err(err) = write_influx_decision(client, config, &entry) {
        warn!("failed to write Influx decision log: {err}");
    }
    info!("decision: {}", serde_json::to_string(&entry)?);
    Ok(())
}

fn control_loop(config: Config, runtime: Arc<Mutex<RuntimeState>>) {
    let client = Client::new();
    let start = Instant::now();
    let mut last_decision = Instant::now() - Duration::from_secs(config.control_every_seconds);
    loop {
        std::thread::sleep(Duration::from_secs(config.sample_every_seconds));
        if start.elapsed().as_secs() < config.startup_grace_seconds {
            continue;
        }
        if last_decision.elapsed().as_secs() < config.control_every_seconds {
            continue;
        }
        last_decision = Instant::now();
        if let Err(err) = run_control_cycle(&config, &runtime, &client) {
            error!("control cycle failed: {err:#}");
        }
    }
}

async fn api_status(State(state): State<ServiceState>) -> Json<StatusResponse> {
    let runtime = state.runtime.lock().unwrap().clone();
    Json(StatusResponse {
        mode: runtime.mode,
        away_until: runtime.away_until,
        updated_at: runtime.updated_at,
        last_reason: runtime.last_reason,
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
        Commands::Status => {
            let state = load_runtime_state(&config.state_file)?;
            println!("{}", toml::to_string_pretty(&state)?);
            return Ok(());
        }
        Commands::Run => {}
    }

    ensure_parent(&config.state_file)?;
    ensure_parent(&config.jsonl_log_file)?;

    let runtime = Arc::new(Mutex::new(load_runtime_state(&config.state_file)?));
    let service_state = ServiceState {
        config: config.clone(),
        runtime: runtime.clone(),
    };

    let loop_config = config.clone();
    let loop_runtime = runtime.clone();
    std::thread::spawn(move || control_loop(loop_config, loop_runtime));

    let app = axum::Router::new()
        .route("/status", get(api_status))
        .route("/mode/occupied", post(api_mode_occupied))
        .route("/mode/short-absence", post(api_mode_short_absence))
        .route("/mode/away", post(api_mode_away))
        .route("/mode/disabled", post(api_mode_disabled))
        .route("/mode/monitor-only", post(api_mode_monitor_only))
        .route("/kill", post(api_kill))
        .with_state(service_state.clone());

    let listener = tokio::net::TcpListener::bind(&config.http_bind).await?;
    info!(
        "adaptive-heating-mvp HTTP listening on {}",
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
