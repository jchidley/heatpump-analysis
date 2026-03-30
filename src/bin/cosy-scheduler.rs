//! Cosy period scheduler — runs at the start of each Cosy window.
//!
//! Reads current outside temperature from ebusd and makes decisions:
//!   - DHW mode: eco (mild) vs normal (cold)
//!   - Heating: off at midnight, on at morning Cosy
//!
//! Usage (from crontab on pi5data):
//!   0  0 * * * /usr/local/bin/cosy-scheduler midnight
//!   0  4 * * * /usr/local/bin/cosy-scheduler morning
//!   0 13 * * * /usr/local/bin/cosy-scheduler afternoon
//!   0 22 * * * /usr/local/bin/cosy-scheduler evening

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const EBUSD_HOST: &str = "localhost:8888";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Outside temp below this → normal DHW (faster, leaves more Cosy time for heating)
const COLD_THRESHOLD: f64 = 4.0;

fn ebus_command(cmd: &str) -> Result<String, String> {
    let stream = TcpStream::connect_timeout(&EBUSD_HOST.parse().unwrap(), CONNECT_TIMEOUT)
        .map_err(|e| format!("connect failed: {e}"))?;

    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| format!("set timeout: {e}"))?;

    let mut stream = stream;
    writeln!(stream, "{cmd}").map_err(|e| format!("write failed: {e}"))?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("read failed: {e}"))?;

    let response = response.trim().to_string();
    if response.starts_with("ERR:") {
        return Err(format!("ebusd error: {response}"));
    }
    Ok(response)
}

fn read_outside_temp() -> Result<f64, String> {
    let resp = ebus_command("read -c hmu OutsideTemp")?;
    resp.parse::<f64>()
        .map_err(|e| format!("parse outside temp '{resp}': {e}"))
}

fn read_hwc_mode() -> Result<String, String> {
    ebus_command("read -c hmu HwcMode")
}

fn set_hwc_mode(mode: &str) -> Result<(), String> {
    // mode: "eco" or "normal"
    let val = match mode {
        "eco" => "0",
        "normal" => "1",
        "balance" => "2",
        _ => return Err(format!("unknown mode: {mode}")),
    };
    let resp = ebus_command(&format!("write -c hmu HwcMode {val}"))?;
    if resp != "done" {
        return Err(format!("unexpected response: {resp}"));
    }
    Ok(())
}

fn set_heating(on: bool) -> Result<(), String> {
    let mode = if on { "auto" } else { "off" };
    let resp = ebus_command(&format!("write -c 700 Z1OpMode {mode}"))?;
    if resp != "done" {
        return Err(format!("unexpected response: {resp}"));
    }
    Ok(())
}

fn decide_dhw_mode(outside_t: f64) -> &'static str {
    if outside_t < COLD_THRESHOLD {
        "normal" // faster DHW, more Cosy time for heating recovery
    } else {
        "eco" // better COP, house doesn't need as much recovery
    }
}

fn run_period(period: &str) -> Result<(), String> {
    let outside_t = read_outside_temp()?;
    let current_mode = read_hwc_mode().unwrap_or_else(|_| "unknown".into());

    eprintln!(
        "cosy-scheduler: period={period}, outside={outside_t:.1}°C, current_hwc={current_mode}"
    );

    match period {
        "midnight" => {
            // 00:00: End of evening Cosy → turn heating OFF for dead zone
            set_heating(false)?;
            eprintln!("  → heating OFF (00:00–04:00 mid-peak dead zone)");
        }

        "morning" => {
            // 04:00: Morning Cosy starts → heating ON + set DHW mode
            set_heating(true)?;
            let mode = decide_dhw_mode(outside_t);
            if mode != current_mode {
                set_hwc_mode(mode)?;
                eprintln!("  → heating ON, DHW switched to {mode} (was {current_mode})");
            } else {
                eprintln!("  → heating ON, DHW stays {mode}");
            }
        }

        "afternoon" => {
            // 13:00: Afternoon Cosy → set DHW mode (heating already on)
            let mode = decide_dhw_mode(outside_t);
            if mode != current_mode {
                set_hwc_mode(mode)?;
                eprintln!("  → DHW switched to {mode} (was {current_mode})");
            } else {
                eprintln!("  → DHW stays {mode}");
            }
        }

        "evening" => {
            // 22:00: Evening Cosy → DHW mode for any evening top-up
            // Always eco in evening — house is warm, no recovery pressure
            if current_mode != "eco" {
                set_hwc_mode("eco")?;
                eprintln!("  → DHW switched to eco for evening");
            } else {
                eprintln!("  → DHW stays eco");
            }
        }

        _ => {
            return Err(format!(
                "unknown period: {period}. Use: midnight, morning, afternoon, evening"
            ));
        }
    }

    Ok(())
}

fn main() {
    let period = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cosy-scheduler <midnight|morning|afternoon|evening>");
        std::process::exit(1);
    });

    match run_period(&period) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("cosy-scheduler ERROR: {e}");
            std::process::exit(1);
        }
    }
}
