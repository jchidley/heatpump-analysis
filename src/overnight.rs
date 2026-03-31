//! Overnight heating strategy optimizer.
//!
//! Analyses 2 years of emonhp data to:
//! 1. Calibrate empirical models (cooling, heating, DHW, COP)
//! 2. For each historical winter night, calculate actual cost
//! 3. Simulate alternative schedules and find the optimal one
//! 4. Compare actual vs optimal across the full dataset
//!
//! Two-rate tariff model (Octopus Cosy + Tesla Powerwall):
//!   - Cosy: 14.05p/kWh during 04:00–07:00 (morning) and 13:00–16:00 (afternoon)
//!   - Blended: 17.0p/kWh all other times (battery absorbs peaks)

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, Timelike};
use polars::prelude::*;

use crate::config::config;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Cosy rate (p/kWh) — applies during three Cosy windows
const COSY_RATE: f64 = 14.05;
/// Mid-peak rate (p/kWh) — 00:00–04:00, 07:00–13:00, 19:00–22:00
const MID_RATE: f64 = 28.65;
/// Peak rate (p/kWh) — 16:00–19:00
const PEAK_RATE: f64 = 42.97;

/// Battery covers ~95% of non-Cosy usage at effective Cosy rate.
/// The 5% leakage hits grid at mid/peak rates.
const BATTERY_COVERAGE: f64 = 0.95;

/// Night analysis window: 20:00 to 09:00 next day
const NIGHT_START_HOUR: u32 = 20;
const NIGHT_END_HOUR: u32 = 9;

/// Earliest OFF time — midnight (end of evening Cosy)
const EARLIEST_OFF_HOUR: u32 = 0;

/// Minimum indoor_t samples required for a valid night (out of ~780 max)
const MIN_NIGHT_SAMPLES: usize = 500;

/// Minimum ΔT (indoor − outside) for "heating relevant" night
const MIN_DELTA_T: f64 = 5.0;

/// Default target indoor temperature at target hour (°C)
const TARGET_TEMP: f64 = 19.5;

/// Sweep of target temps for sensitivity analysis
const TARGET_SWEEP: [f64; 6] = [19.5, 19.0, 18.5, 18.0, 17.5, 17.0];

/// Hour by which target must be met
const TARGET_HOUR: u32 = 7;

/// DHW normal mode: ~60 min, higher MWT (~55°C), COP ~2.5
const DHW_NORMAL_MIN: u32 = 60;
/// DHW eco mode: ~115 min, lower MWT (~45°C), COP ~3.1
const DHW_ECO_MIN: u32 = 115;
/// DHW electricity — normal mode (kWh): 6.0 kWh heat / COP 2.5
const DHW_NORMAL_ELEC_KWH: f64 = 2.4;
/// DHW electricity — eco mode (kWh): 6.0 kWh heat / COP 3.1
const DHW_ECO_ELEC_KWH: f64 = 1.9;

/// Minimum DHW cycle duration to include in calibration (filters short top-ups)
const MIN_DHW_CYCLE_MIN: u32 = 30;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One minute of overnight data
#[derive(Clone)]
struct Minute {
    /// Minutes since 20:00 on the evening date (0 = 20:00, 480 = 04:00, 660 = 07:00)
    offset_min: u32,
    outside_t: f64,
    indoor_t: f64,
    elec_w: f64,
    heat_w: f64,
    mwt: f64,
    state: &'static str,
}

/// One complete overnight period
struct Night {
    date: NaiveDate,
    minutes: Vec<Minute>,
    avg_outside_t: f64,
    /// Indoor temp at 20:00
    indoor_t_start: f64,
    /// Indoor temp at target hour
    indoor_t_target: f64,
}

/// Calibrated cooling model: dT/dt = -k × (T_indoor − T_outside)
#[allow(dead_code)]
struct CoolingModel {
    k: f64,
    capacity_wh: f64,
    n_samples: usize,
    n_dhw_samples: usize,
    n_idle_samples: usize,
}

/// Heating performance binned by outside temperature
struct HeatingBin {
    t_out_low: f64,
    t_out_high: f64,
    avg_heat_w: f64,
    avg_elec_w: f64,
    avg_cop: f64,
    avg_mwt: f64,
    n_samples: usize,
}

/// DHW cycle statistics
struct DhwStats {
    /// Average duration of morning DHW cycles (≥30 min)
    avg_duration_min: f64,
    /// Average electricity per morning DHW cycle
    avg_elec_kwh: f64,
    n_cycles: usize,
}

/// A candidate overnight schedule
#[derive(Clone)]
struct Schedule {
    label: String,
    /// Minute offset (from 20:00) to turn heating off. None = heating on all night.
    off_at: Option<u32>,
    /// Minute offset to start DHW. None = no DHW in overnight window.
    dhw_start: Option<u32>,
    /// DHW duration in minutes.
    dhw_duration: u32,
    /// DHW electricity for this mode (kWh).
    dhw_elec_kwh: f64,
    /// Minute offset to restart space heating.
    heat_on: u32,
}

/// Result of simulating one schedule on one night
#[derive(Clone)]
#[allow(dead_code)]
struct SimResult {
    schedule_idx: usize,
    cosy_kwh: f64,
    blended_kwh: f64,
    total_kwh: f64,
    cost_pence: f64,
    indoor_t_07: f64,
    feasible: bool,
}

// ---------------------------------------------------------------------------
// Tariff helpers
// ---------------------------------------------------------------------------

/// Tariff band for a given clock hour.
#[derive(Clone, Copy, PartialEq)]
enum TariffBand {
    Cosy, // 14.05p — 04:00-07:00, 13:00-16:00, 22:00-00:00
    Mid,  // 28.65p — 00:00-04:00, 07:00-13:00, 19:00-22:00
    Peak, // 42.97p — 16:00-19:00
}

fn tariff_band(offset_min: u32) -> TariffBand {
    let hour = (NIGHT_START_HOUR + offset_min / 60) % 24;
    match hour {
        4..=6 => TariffBand::Cosy,   // 04:00-07:00
        13..=15 => TariffBand::Cosy, // 13:00-16:00
        22..=23 => TariffBand::Cosy, // 22:00-00:00
        16..=18 => TariffBand::Peak, // 16:00-19:00
        _ => TariffBand::Mid,        // everything else
    }
}

/// Effective rate for a kWh consumed at this time, accounting for battery.
/// During Cosy: always grid at Cosy rate (battery charges too).
/// During Mid/Peak: battery covers 95%, rest hits grid at full rate.
fn effective_rate(offset_min: u32) -> f64 {
    match tariff_band(offset_min) {
        TariffBand::Cosy => COSY_RATE,
        TariffBand::Mid => BATTERY_COVERAGE * COSY_RATE + (1.0 - BATTERY_COVERAGE) * MID_RATE,
        TariffBand::Peak => BATTERY_COVERAGE * COSY_RATE + (1.0 - BATTERY_COVERAGE) * PEAK_RATE,
    }
}

fn is_cosy(offset_min: u32) -> bool {
    tariff_band(offset_min) == TariffBand::Cosy
}

/// Minute offset for a given clock hour (e.g., 4 → 480, 7 → 660, 22 → 120)
fn offset_for_hour(hour: u32) -> u32 {
    if hour >= NIGHT_START_HOUR {
        (hour - NIGHT_START_HOUR) * 60
    } else {
        (24 - NIGHT_START_HOUR + hour) * 60
    }
}

fn fmt_offset(offset: u32) -> String {
    let hour = (NIGHT_START_HOUR + offset / 60) % 24;
    let min = offset % 60;
    format!("{:02}:{:02}", hour, min)
}

// ---------------------------------------------------------------------------
// Night extraction
// ---------------------------------------------------------------------------

fn extract_nights(df: &DataFrame) -> Result<Vec<Night>> {
    let ts_col = df.column("timestamp")?.datetime()?;
    let outside_col = df.column("outside_t")?.f64()?;
    let indoor_col = df.column("indoor_t")?.f64()?;
    let elec_col = df.column("elec_w")?.f64()?;
    let heat_col = df.column("heat_w")?.f64()?;
    let flow_col = df.column("flow_t")?.f64()?;
    let return_col = df.column("return_t")?.f64()?;
    let state_col = df.column("state")?.str()?;

    let n = df.height();

    let mut night_map: std::collections::BTreeMap<NaiveDate, Vec<usize>> =
        std::collections::BTreeMap::new();

    for i in 0..n {
        let Some(ts_ms) = ts_col.get(i) else { continue };
        let Some(dt) = DateTime::from_timestamp_millis(ts_ms) else {
            continue;
        };
        let hour = dt.hour();
        let date = dt.date_naive();

        let night_date = if hour >= NIGHT_START_HOUR {
            date
        } else if hour < NIGHT_END_HOUR {
            date - chrono::Duration::days(1)
        } else {
            continue;
        };

        // Winter months only (Oct–Mar)
        let month = night_date.month();
        if (4..=9).contains(&month) {
            continue;
        }

        night_map.entry(night_date).or_default().push(i);
    }

    let mut nights = Vec::new();

    for (date, indices) in &night_map {
        let mut minutes = Vec::new();

        for &i in indices {
            let Some(ts_ms) = ts_col.get(i) else { continue };
            let Some(dt) = DateTime::from_timestamp_millis(ts_ms) else {
                continue;
            };
            let Some(outside) = outside_col.get(i) else {
                continue;
            };
            let Some(indoor) = indoor_col.get(i) else {
                continue;
            };

            let elec = elec_col.get(i).unwrap_or(0.0);
            let heat = heat_col.get(i).unwrap_or(0.0);
            let flow = flow_col.get(i).unwrap_or(0.0);
            let ret = return_col.get(i).unwrap_or(0.0);
            let mwt = (flow + ret) / 2.0;

            let state: &'static str = match state_col.get(i).unwrap_or("idle") {
                "heating" => "heating",
                "dhw" => "dhw",
                "defrost" => "defrost",
                _ => "idle",
            };

            let hour = dt.hour();
            let min = dt.minute();
            let offset = if hour >= NIGHT_START_HOUR {
                (hour - NIGHT_START_HOUR) * 60 + min
            } else {
                (24 - NIGHT_START_HOUR + hour) * 60 + min
            };

            minutes.push(Minute {
                offset_min: offset,
                outside_t: outside,
                indoor_t: indoor,
                elec_w: elec,
                heat_w: heat,
                mwt,
                state,
            });
        }

        minutes.sort_by_key(|m| m.offset_min);
        if minutes.len() < MIN_NIGHT_SAMPLES {
            continue;
        }

        let avg_outside = minutes.iter().map(|m| m.outside_t).sum::<f64>() / minutes.len() as f64;
        let indoor_start = minutes.first().map(|m| m.indoor_t).unwrap_or(20.0);

        // Indoor temp at target hour — take nearest minute
        let target_offset = offset_for_hour(TARGET_HOUR);
        let indoor_target = minutes
            .iter()
            .filter(|m| {
                m.offset_min >= target_offset.saturating_sub(2) && m.offset_min <= target_offset + 2
            })
            .map(|m| m.indoor_t)
            .next()
            .unwrap_or(indoor_start);

        if indoor_start - avg_outside < MIN_DELTA_T {
            continue;
        }

        nights.push(Night {
            date: *date,
            minutes,
            avg_outside_t: avg_outside,
            indoor_t_start: indoor_start,
            indoor_t_target: indoor_target,
        });
    }

    Ok(nights)
}

// ---------------------------------------------------------------------------
// Model calibration
// ---------------------------------------------------------------------------

/// Calibrate cooling from DHW events and idle periods.
///
/// During DHW, the diverter sends all heat to the cylinder — zero radiator
/// heat reaches the house. This is a genuine "heating off" condition, and
/// we have hundreds of these ~1h windows in the data. Much better than
/// short idle cycles between compressor runs (which have warm walls from
/// recent heating, giving artificially slow cooling).
///
/// We also include genuine idle periods ≥5 minutes (filters out brief
/// cycling gaps) for additional data.
fn calibrate_cooling(nights: &[Night]) -> CoolingModel {
    let mut sum_xy = 0.0;
    let mut sum_xx = 0.0;
    let mut n_dhw = 0usize;
    let mut n_idle = 0usize;

    for night in nights {
        let mins = &night.minutes;

        for pair in mins.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if b.offset_min != a.offset_min + 1 {
                continue;
            }

            // DHW state: house genuinely cooling (no radiator heat)
            // Idle state: only count if part of a longer idle run (≥5 consecutive)
            let is_dhw = a.state == "dhw" && b.state == "dhw";
            let is_long_idle = if a.state == "idle" && b.state == "idle" {
                // Check for ≥5 consecutive idle minutes around this pair
                let idx = mins.iter().position(|m| std::ptr::eq(m, a)).unwrap_or(0);
                let start = idx.saturating_sub(2);
                let end = (idx + 3).min(mins.len());
                (start..end).all(|i| mins[i].state == "idle")
            } else {
                false
            };

            if !is_dhw && !is_long_idle {
                continue;
            }

            let delta_indoor = b.indoor_t - a.indoor_t;
            let avg_dt = ((a.indoor_t + b.indoor_t) / 2.0) - ((a.outside_t + b.outside_t) / 2.0);
            if avg_dt.abs() < 1.0 {
                continue;
            }

            // Only count minutes where indoor is actually cooling (not noise)
            if delta_indoor > 0.05 {
                continue; // indoor rising during supposed cooling — skip noise
            }

            let dt_per_hr = delta_indoor * 60.0;
            sum_xy += (-dt_per_hr) * avg_dt;
            sum_xx += avg_dt * avg_dt;
            if is_dhw {
                n_dhw += 1;
            } else {
                n_idle += 1;
            }
        }
    }

    let k = if sum_xx > 0.0 {
        (sum_xy / sum_xx).clamp(0.005, 0.05)
    } else {
        0.023 // fallback: whole-house value from thermal model experiments
    };
    let htc = config().house.htc_w_per_c;

    CoolingModel {
        k,
        capacity_wh: htc / k,
        n_samples: n_dhw + n_idle,
        n_dhw_samples: n_dhw,
        n_idle_samples: n_idle,
    }
}

/// Heating mode classification based on indoor temperature trajectory.
/// During setback, indoor_t is flat or falling slowly while heating — the HP
/// is maintaining ~17°C. During recovery, indoor_t is rising — the HP is
/// driving towards 21°C with higher MWT and output.
#[derive(Clone, Copy, PartialEq)]
enum HeatingMode {
    /// Indoor temp rising (recovery after setback or off period)
    Recovery,
    /// Indoor temp flat/falling (setback maintenance or steady state)
    Maintenance,
}

/// Calibrate heating performance by outside temperature bin, separated by mode.
///
/// Recovery mode (indoor_t rising) is what happens when we turn heating ON
/// after an off period — higher MWT, higher heat output, lower COP.
/// Maintenance mode (indoor_t flat) is the setback cycling.
///
/// We return two sets of bins: recovery (for simulating ON after OFF)
/// and maintenance (for comparison / "continuous" strategy).
fn calibrate_heating(nights: &[Night]) -> (Vec<HeatingBin>, Vec<HeatingBin>) {
    let bin_edges: Vec<f64> = (-3..=7).map(|i| i as f64 * 2.0).collect();

    let mut recovery_bins = Vec::new();
    let mut maint_bins = Vec::new();

    // Classify each heating minute: look at indoor_t trend over ±5 min window
    for target_mode in [HeatingMode::Recovery, HeatingMode::Maintenance] {
        for i in 0..bin_edges.len() - 1 {
            let lo = bin_edges[i];
            let hi = bin_edges[i + 1];

            let mut heat_sum = 0.0;
            let mut elec_sum = 0.0;
            let mut mwt_sum = 0.0;
            let mut count = 0usize;

            for night in nights {
                let mins = &night.minutes;
                for (j, m) in mins.iter().enumerate() {
                    if m.state != "heating" || m.outside_t < lo || m.outside_t >= hi {
                        continue;
                    }
                    if m.heat_w <= 0.0 || m.elec_w <= 50.0 {
                        continue;
                    }

                    // Classify: look at indoor_t change over ±5 min window
                    let start_idx = j.saturating_sub(5);
                    let end_idx = (j + 5).min(mins.len() - 1);
                    if end_idx <= start_idx {
                        continue;
                    }
                    let dt_indoor = mins[end_idx].indoor_t - mins[start_idx].indoor_t;
                    let mode = if dt_indoor > 0.02 {
                        HeatingMode::Recovery
                    } else {
                        HeatingMode::Maintenance
                    };

                    if mode != target_mode {
                        continue;
                    }

                    heat_sum += m.heat_w;
                    elec_sum += m.elec_w;
                    mwt_sum += m.mwt;
                    count += 1;
                }
            }

            if count >= 50 {
                let bin = HeatingBin {
                    t_out_low: lo,
                    t_out_high: hi,
                    avg_heat_w: heat_sum / count as f64,
                    avg_elec_w: elec_sum / count as f64,
                    avg_cop: heat_sum / elec_sum,
                    avg_mwt: mwt_sum / count as f64,
                    n_samples: count,
                };
                match target_mode {
                    HeatingMode::Recovery => recovery_bins.push(bin),
                    HeatingMode::Maintenance => maint_bins.push(bin),
                }
            }
        }
    }

    (recovery_bins, maint_bins)
}

/// Extract DHW cycle stats. Filters for cycles ≥30 min (real morning DHW, not top-ups).
fn calibrate_dhw(nights: &[Night]) -> DhwStats {
    let mut durations = Vec::new();
    let mut elec_totals = Vec::new();

    for night in nights {
        let mut in_dhw = false;
        let mut dhw_start_offset = 0u32;
        let mut dhw_elec = 0.0;

        for m in &night.minutes {
            if m.state == "dhw" && !in_dhw {
                in_dhw = true;
                dhw_start_offset = m.offset_min;
                dhw_elec = m.elec_w / 60.0 / 1000.0;
            } else if m.state == "dhw" && in_dhw {
                dhw_elec += m.elec_w / 60.0 / 1000.0;
            } else if m.state != "dhw" && in_dhw {
                let duration = m.offset_min.saturating_sub(dhw_start_offset);
                if (MIN_DHW_CYCLE_MIN..=180).contains(&duration) {
                    durations.push(duration as f64);
                    elec_totals.push(dhw_elec);
                }
                in_dhw = false;
            }
        }
    }

    let n = durations.len();
    if n == 0 {
        return DhwStats {
            avg_duration_min: 60.0,
            avg_elec_kwh: DHW_NORMAL_ELEC_KWH,
            n_cycles: 0,
        };
    }

    DhwStats {
        avg_duration_min: durations.iter().sum::<f64>() / n as f64,
        avg_elec_kwh: elec_totals.iter().sum::<f64>() / n as f64,
        n_cycles: n,
    }
}

/// Look up heating performance for a given outside temperature.
fn lookup_heating(bins: &[HeatingBin], t_out: f64) -> (f64, f64, f64) {
    for bin in bins {
        if t_out >= bin.t_out_low && t_out < bin.t_out_high {
            return (bin.avg_heat_w, bin.avg_elec_w, bin.avg_cop);
        }
    }
    if let Some(bin) = bins.first() {
        if t_out < bin.t_out_low {
            return (bin.avg_heat_w, bin.avg_elec_w, bin.avg_cop);
        }
    }
    if let Some(bin) = bins.last() {
        if t_out >= bin.t_out_high {
            return (bin.avg_heat_w, bin.avg_elec_w, bin.avg_cop);
        }
    }
    (3500.0, 700.0, 5.0)
}

// ---------------------------------------------------------------------------
// Actual cost calculation
// ---------------------------------------------------------------------------

fn actual_cost(night: &Night) -> SimResult {
    let mut cosy_kwh = 0.0;
    let mut non_cosy_kwh = 0.0;
    let mut cost = 0.0;

    for m in &night.minutes {
        let kwh = m.elec_w / 60.0 / 1000.0;
        cost += kwh * effective_rate(m.offset_min);
        if is_cosy(m.offset_min) {
            cosy_kwh += kwh;
        } else {
            non_cosy_kwh += kwh;
        }
    }

    let total = cosy_kwh + non_cosy_kwh;

    SimResult {
        schedule_idx: 0,
        cosy_kwh,
        blended_kwh: non_cosy_kwh,
        total_kwh: total,
        cost_pence: cost,
        indoor_t_07: night.indoor_t_target,
        feasible: night.indoor_t_target >= TARGET_TEMP,
    }
}

// ---------------------------------------------------------------------------
// Schedule generation
// ---------------------------------------------------------------------------

fn generate_schedules(_dhw_stats: &DhwStats) -> Vec<Schedule> {
    let mut schedules = Vec::new();
    let cosy_start = offset_for_hour(4); // 04:00 = offset 480
    let cosy_end = offset_for_hour(7); // 07:00 = offset 660

    // DHW modes to try
    struct DhwMode {
        name: &'static str,
        duration: u32,
        elec_kwh: f64,
    }
    let dhw_modes = [
        DhwMode {
            name: "norm",
            duration: DHW_NORMAL_MIN,
            elec_kwh: DHW_NORMAL_ELEC_KWH,
        },
        DhwMode {
            name: "eco",
            duration: DHW_ECO_MIN,
            elec_kwh: DHW_ECO_ELEC_KWH,
        },
    ];

    // Strategy A: Never off (continuous heating), DHW at 05:00
    for mode in &dhw_modes {
        schedules.push(Schedule {
            label: format!("continuous, DHW {} 05:00", mode.name),
            off_at: None,
            dhw_start: Some(540),
            dhw_duration: mode.duration,
            dhw_elec_kwh: mode.elec_kwh,
            heat_on: 0,
        });
    }

    // OFF at 30-min steps, combined with DHW placement and mode
    for off_min in (offset_for_hour(EARLIEST_OFF_HOUR)..=offset_for_hour(2)).step_by(30) {
        let off_label = fmt_offset(off_min);

        for mode in &dhw_modes {
            // B: DHW first at Cosy start, then heat
            let heat_after_dhw = cosy_start + mode.duration;
            if heat_after_dhw <= cosy_end {
                schedules.push(Schedule {
                    label: format!(
                        "OFF {} → DHW {} 04:00 → heat {}",
                        off_label,
                        mode.name,
                        fmt_offset(heat_after_dhw)
                    ),
                    off_at: Some(off_min),
                    dhw_start: Some(cosy_start),
                    dhw_duration: mode.duration,
                    dhw_elec_kwh: mode.elec_kwh,
                    heat_on: heat_after_dhw,
                });
            }

            // C: Heat first at Cosy start, DHW at end of Cosy
            if cosy_end >= mode.duration {
                let dhw_at = cosy_end - mode.duration;
                if dhw_at >= cosy_start {
                    schedules.push(Schedule {
                        label: format!(
                            "OFF {} → heat 04:00 → DHW {} {}",
                            off_label,
                            mode.name,
                            fmt_offset(dhw_at)
                        ),
                        off_at: Some(off_min),
                        dhw_start: Some(dhw_at),
                        dhw_duration: mode.duration,
                        dhw_elec_kwh: mode.elec_kwh,
                        heat_on: cosy_start,
                    });
                }
            }
        }
    }

    schedules
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

/// Simulate a schedule on one night using calibrated models + actual outside temps.
fn simulate_schedule(
    schedule: &Schedule,
    sched_idx: usize,
    night: &Night,
    cooling: &CoolingModel,
    recovery_bins: &[HeatingBin],
    maint_bins: &[HeatingBin],
) -> SimResult {
    let htc = config().house.htc_w_per_c;

    // Build outside temperature lookup
    let outside_temps: Vec<(u32, f64)> = night
        .minutes
        .iter()
        .map(|m| (m.offset_min, m.outside_t))
        .collect();

    let lookup_outside = |offset: u32| -> f64 {
        match outside_temps.binary_search_by_key(&offset, |&(o, _)| o) {
            Ok(i) => outside_temps[i].1,
            Err(i) => {
                if i == 0 {
                    outside_temps[0].1
                } else if i >= outside_temps.len() {
                    outside_temps.last().unwrap().1
                } else {
                    let (o1, t1) = outside_temps[i - 1];
                    let (o2, t2) = outside_temps[i];
                    if o2 == o1 {
                        t1
                    } else {
                        t1 + (t2 - t1) * (offset - o1) as f64 / (o2 - o1) as f64
                    }
                }
            }
        }
    };

    let mut t_indoor = night.indoor_t_start;
    let mut cosy_kwh = 0.0;
    let mut blended_kwh = 0.0;
    let mut cost = 0.0;
    let mut t_at_07 = t_indoor;

    let first_offset = night.minutes.first().map(|m| m.offset_min).unwrap_or(0);
    let last_offset = night.minutes.last().map(|m| m.offset_min).unwrap_or(780);

    let mut offset = first_offset;
    while offset <= last_offset {
        let t_out = lookup_outside(offset);

        // Is the HP doing DHW right now?
        let in_dhw = match schedule.dhw_start {
            Some(ds) => offset >= ds && offset < ds + schedule.dhw_duration,
            None => false,
        };

        // Is space heating on?
        let heating_on = if in_dhw {
            false
        } else if let Some(off_at) = schedule.off_at {
            // Heating off from off_at until heat_on
            if schedule.heat_on > off_at {
                !(offset >= off_at && offset < schedule.heat_on)
            } else {
                // off wraps past midnight: off_at..end_of_day + start..heat_on
                offset < off_at || offset >= schedule.heat_on
            }
        } else {
            true // never off
        };

        let dt_hr = 1.0 / 60.0; // 1 minute in hours

        if in_dhw {
            // House cools during DHW (no space heating)
            let cooling_rate = cooling.k * (t_indoor - t_out);
            t_indoor -= cooling_rate * dt_hr;

            // DHW electricity spread evenly over cycle
            let dhw_elec_per_min = schedule.dhw_elec_kwh / schedule.dhw_duration as f64;
            cost += dhw_elec_per_min * effective_rate(offset);
            if is_cosy(offset) {
                cosy_kwh += dhw_elec_per_min;
            } else {
                blended_kwh += dhw_elec_per_min;
            }
        } else if heating_on {
            // Space heating: HP delivers heat, house loses to outside.
            // Use recovery bins if heating just came on after an off period
            // (HP driving to 21°C with higher MWT), maintenance bins if
            // the HP has been running continuously (setback cycling).
            let bins = if schedule.off_at.is_some() {
                recovery_bins
            } else {
                maint_bins
            };
            let (heat_w, elec_w, _cop) = lookup_heating(bins, t_out);

            let q_loss = htc * (t_indoor - t_out); // W
            let net_q = heat_w - q_loss; // W
                                         // dT = net_q × dt_seconds / (capacity × 3600)
            let dt_deg = net_q * 60.0 / (cooling.capacity_wh * 3600.0);
            t_indoor += dt_deg;

            let elec_kwh = elec_w / 60.0 / 1000.0;
            cost += elec_kwh * effective_rate(offset);
            if is_cosy(offset) {
                cosy_kwh += elec_kwh;
            } else {
                blended_kwh += elec_kwh;
            }
        } else {
            // Off: house cools
            let cooling_rate = cooling.k * (t_indoor - t_out);
            t_indoor -= cooling_rate * dt_hr;
        }

        if offset == offset_for_hour(TARGET_HOUR) {
            t_at_07 = t_indoor;
        }

        offset += 1;
    }

    let total = cosy_kwh + blended_kwh;

    SimResult {
        schedule_idx: sched_idx,
        cosy_kwh,
        blended_kwh,
        total_kwh: total,
        cost_pence: cost,
        indoor_t_07: t_at_07,
        feasible: t_at_07 >= TARGET_TEMP,
    }
}

// ---------------------------------------------------------------------------
// Main analysis
// ---------------------------------------------------------------------------

pub fn overnight_analysis(df: &DataFrame) -> Result<()> {
    println!("\n{}", "=".repeat(78));
    println!("OVERNIGHT HEATING STRATEGY OPTIMIZER");
    println!("{}", "─".repeat(78));
    println!("Baseline: 4°C setback (Z1NightTemp=17°C, HP cycles all night)");
    println!("Proposed: Heat in evening Cosy → OFF 00:00–04:00 → ON at morning Cosy");
    println!(
        "Tariff:   Cosy {:.2}p (04–07, 13–16, 22–00), Mid {:.2}p, Peak {:.2}p (16–19)",
        COSY_RATE, MID_RATE, PEAK_RATE
    );
    println!(
        "Battery:  covers {:.0}% of non-Cosy → effective mid {:.1}p, peak {:.1}p",
        BATTERY_COVERAGE * 100.0,
        effective_rate(offset_for_hour(1)), // sample mid-peak hour
        effective_rate(offset_for_hour(17)), // sample peak hour (mapped via offset)
    );
    println!(
        "Target:   ≥{:.1}°C indoor at {:02}:00",
        TARGET_TEMP, TARGET_HOUR
    );
    println!("{}", "=".repeat(78));

    // 1. Extract nights
    eprintln!("Extracting winter nights...");
    let nights = extract_nights(df).context("Failed to extract nights")?;
    println!(
        "\n{} winter nights (Oct–Mar, ΔT ≥ {:.0}°C, ≥{} samples)",
        nights.len(),
        MIN_DELTA_T,
        MIN_NIGHT_SAMPLES
    );
    if nights.is_empty() {
        println!("No valid nights found.");
        return Ok(());
    }

    let first = nights.first().unwrap().date;
    let last = nights.last().unwrap().date;
    println!("Date range: {} to {}", first, last);

    // 2. Calibrate models
    eprintln!("Calibrating models...");
    let cooling = calibrate_cooling(&nights);
    let (recovery_bins, maint_bins) = calibrate_heating(&nights);
    let dhw = calibrate_dhw(&nights);

    println!("\n--- Calibrated Models ---");
    println!(
        "\nCooling (from {} DHW + {} long-idle minutes):",
        cooling.n_dhw_samples, cooling.n_idle_samples
    );
    println!(
        "  k = {:.4}/hr → {:.2}°C/hr per °C of ΔT",
        cooling.k, cooling.k
    );
    println!(
        "  Thermal capacity: {:.0} Wh/°C (τ = {:.1} hours)",
        cooling.capacity_wh,
        1.0 / cooling.k
    );
    println!(
        "  At 12°C ΔT: {:.2}°C/hr cooling → {:.1}°C drop over 7h off",
        cooling.k * 12.0,
        cooling.k * 12.0 * 7.0
    );

    println!("\nHeating performance by outside temp:");
    println!("\n  RECOVERY (indoor_t rising — after off period, driving to 21°C):");
    println!(
        "  {:>8} {:>7} {:>7} {:>5} {:>5} {:>8}",
        "T_out", "Heat W", "Elec W", "COP", "MWT°", "Samples"
    );
    for bin in &recovery_bins {
        println!(
            "  {:>3.0}–{:<3.0}°C {:>6.0}  {:>6.0}  {:>5.2} {:>5.1} {:>7}",
            bin.t_out_low,
            bin.t_out_high,
            bin.avg_heat_w,
            bin.avg_elec_w,
            bin.avg_cop,
            bin.avg_mwt,
            bin.n_samples
        );
    }
    println!("\n  MAINTENANCE (indoor_t flat — setback cycling at ~17°C):");
    println!(
        "  {:>8} {:>7} {:>7} {:>5} {:>5} {:>8}",
        "T_out", "Heat W", "Elec W", "COP", "MWT°", "Samples"
    );
    for bin in &maint_bins {
        println!(
            "  {:>3.0}–{:<3.0}°C {:>6.0}  {:>6.0}  {:>5.2} {:>5.1} {:>7}",
            bin.t_out_low,
            bin.t_out_high,
            bin.avg_heat_w,
            bin.avg_elec_w,
            bin.avg_cop,
            bin.avg_mwt,
            bin.n_samples
        );
    }

    println!(
        "\nDHW (≥{} min cycles): avg {:.0} min, {:.2} kWh elec ({} cycles)",
        MIN_DHW_CYCLE_MIN, dhw.avg_duration_min, dhw.avg_elec_kwh, dhw.n_cycles
    );

    // 3. Generate schedules and run backtest
    let schedules = generate_schedules(&dhw);
    println!(
        "\n{} strategies × {} nights = {} simulations",
        schedules.len(),
        nights.len(),
        schedules.len() * nights.len()
    );
    eprintln!("Running backtest...");

    // Store all simulation results per night for sensitivity sweep
    struct NightResults {
        date: NaiveDate,
        avg_outside: f64,
        indoor_start: f64,
        actual: SimResult,
        sim_results: Vec<SimResult>, // one per strategy
    }
    let mut all_nights: Vec<NightResults> = Vec::new();

    for night in &nights {
        let actual = actual_cost(night);
        let sim_results: Vec<SimResult> = schedules
            .iter()
            .enumerate()
            .map(|(i, sched)| {
                simulate_schedule(sched, i, night, &cooling, &recovery_bins, &maint_bins)
            })
            .collect();

        all_nights.push(NightResults {
            date: night.date,
            avg_outside: night.avg_outside_t,
            indoor_start: night.indoor_t_start,
            actual,
            sim_results,
        });
    }

    // Helper: compute adaptive strategy for a given target temperature
    struct AdaptiveResult {
        total_cost: f64,
        total_kwh: f64,
        total_cosy_kwh: f64,
        feasible_count: usize,
        avg_min_t07: f64,
    }

    let compute_adaptive = |target: f64| -> AdaptiveResult {
        let mut cost = 0.0;
        let mut kwh = 0.0;
        let mut cosy = 0.0;
        let mut feasible = 0usize;
        let mut min_t07_sum = 0.0;

        for nr in &all_nights {
            // Best feasible = cheapest that reaches target
            let best_feasible = nr
                .sim_results
                .iter()
                .filter(|r| r.indoor_t_07 >= target)
                .min_by(|a, b| a.cost_pence.partial_cmp(&b.cost_pence).unwrap());

            let (night_cost, night_kwh, night_cosy, night_ok) = if let Some(bf) = best_feasible {
                if bf.cost_pence <= nr.actual.cost_pence {
                    (bf.cost_pence, bf.total_kwh, bf.cosy_kwh, true)
                } else {
                    (
                        nr.actual.cost_pence,
                        nr.actual.total_kwh,
                        nr.actual.cosy_kwh,
                        nr.actual.indoor_t_07 >= target,
                    )
                }
            } else {
                (
                    nr.actual.cost_pence,
                    nr.actual.total_kwh,
                    nr.actual.cosy_kwh,
                    nr.actual.indoor_t_07 >= target,
                )
            };

            cost += night_cost;
            kwh += night_kwh;
            cosy += night_cosy;
            if night_ok {
                feasible += 1;
            }

            // Track worst-case T at 07:00 for the chosen strategy
            let chosen_t = if let Some(bf) = best_feasible {
                if bf.cost_pence <= nr.actual.cost_pence {
                    bf.indoor_t_07
                } else {
                    nr.actual.indoor_t_07
                }
            } else {
                nr.actual.indoor_t_07
            };
            min_t07_sum += chosen_t;
        }

        AdaptiveResult {
            total_cost: cost,
            total_kwh: kwh,
            total_cosy_kwh: cosy,
            feasible_count: feasible,
            avg_min_t07: min_t07_sum / all_nights.len() as f64,
        }
    };

    // Compute per-strategy totals and adaptive at default target
    struct StratAccum {
        total_cost: f64,
        total_kwh: f64,
        total_cosy_kwh: f64,
        total_blended_kwh: f64,
        feasible_nights: usize,
    }

    let n_strats = schedules.len();
    let mut strat_accum: Vec<StratAccum> = (0..n_strats)
        .map(|_| StratAccum {
            total_cost: 0.0,
            total_kwh: 0.0,
            total_cosy_kwh: 0.0,
            total_blended_kwh: 0.0,
            feasible_nights: 0,
        })
        .collect();

    let mut actual_total_cost = 0.0;
    let mut actual_total_kwh = 0.0;
    let mut actual_feasible = 0usize;
    let mut actual_cosy_kwh_total = 0.0;

    for nr in &all_nights {
        actual_total_cost += nr.actual.cost_pence;
        actual_total_kwh += nr.actual.total_kwh;
        actual_cosy_kwh_total += nr.actual.cosy_kwh;
        if nr.actual.indoor_t_07 >= TARGET_TEMP {
            actual_feasible += 1;
        }
        for (i, r) in nr.sim_results.iter().enumerate() {
            strat_accum[i].total_cost += r.cost_pence;
            strat_accum[i].total_kwh += r.total_kwh;
            strat_accum[i].total_cosy_kwh += r.cosy_kwh;
            strat_accum[i].total_blended_kwh += r.blended_kwh;
            if r.indoor_t_07 >= TARGET_TEMP {
                strat_accum[i].feasible_nights += 1;
            }
        }
    }

    let adaptive = compute_adaptive(TARGET_TEMP);

    // Build night summaries for display (at default target)
    struct NightSummary {
        date: NaiveDate,
        avg_outside: f64,
        indoor_start: f64,
        actual: SimResult,
        best: SimResult,
        best_label: String,
    }
    let mut night_summaries: Vec<NightSummary> = Vec::new();
    for nr in &all_nights {
        let best_feasible = nr
            .sim_results
            .iter()
            .filter(|r| r.indoor_t_07 >= TARGET_TEMP)
            .min_by(|a, b| a.cost_pence.partial_cmp(&b.cost_pence).unwrap());

        let (best, best_label) = if let Some(bf) = best_feasible {
            if bf.cost_pence <= nr.actual.cost_pence {
                (bf.clone(), schedules[bf.schedule_idx].label.clone())
            } else {
                (nr.actual.clone(), "actual (cheapest)".into())
            }
        } else {
            // No feasible — pick warmest
            let warmest = nr
                .sim_results
                .iter()
                .max_by(|a, b| a.indoor_t_07.partial_cmp(&b.indoor_t_07).unwrap())
                .unwrap();
            if warmest.cost_pence <= nr.actual.cost_pence {
                (
                    warmest.clone(),
                    schedules[warmest.schedule_idx].label.clone(),
                )
            } else {
                (nr.actual.clone(), "actual (cheapest)".into())
            }
        };

        night_summaries.push(NightSummary {
            date: nr.date,
            avg_outside: nr.avg_outside,
            indoor_start: nr.indoor_start,
            actual: nr.actual.clone(),
            best,
            best_label,
        });
    }

    // 4. Strategy comparison table
    println!("\n{}", "=".repeat(78));
    println!("STRATEGY COMPARISON ({} winter nights)", nights.len());
    println!("{}", "=".repeat(78));
    println!(
        "\n{:<45} {:>7} {:>7} {:>6} {:>6} {:>8}",
        "Strategy", "Total£", "kWh", "Cosy%", "≥tgt", "vs actual"
    );
    println!("{}", "─".repeat(82));

    let actual_cosy_pct = if actual_total_kwh > 0.0 {
        actual_cosy_kwh_total / actual_total_kwh * 100.0
    } else {
        0.0
    };

    println!(
        "{:<45} {:>7.2} {:>7.1} {:>5.1}% {:>3}/{:<3} baseline",
        "ACTUAL (measured)",
        actual_total_cost / 100.0,
        actual_total_kwh,
        actual_cosy_pct,
        actual_feasible,
        nights.len()
    );

    // Strategy rows
    for (i, sched) in schedules.iter().enumerate() {
        let sa = &strat_accum[i];
        let pct_cosy = if sa.total_kwh > 0.0 {
            sa.total_cosy_kwh / sa.total_kwh * 100.0
        } else {
            0.0
        };
        let saving = actual_total_cost - sa.total_cost;
        let saving_str = if saving > 0.0 {
            format!("save £{:.2}", saving / 100.0)
        } else {
            format!("+£{:.2}", -saving / 100.0)
        };
        println!(
            "{:<45} {:>7.2} {:>7.1} {:>5.1}% {:>3}/{:<3} {}",
            sched.label,
            sa.total_cost / 100.0,
            sa.total_kwh,
            pct_cosy,
            sa.feasible_nights,
            nights.len(),
            saving_str
        );
    }

    // Adaptive row
    let adaptive_saving = actual_total_cost - adaptive.total_cost;
    let adaptive_cosy_pct = if adaptive.total_kwh > 0.0 {
        adaptive.total_cosy_kwh / adaptive.total_kwh * 100.0
    } else {
        0.0
    };
    println!("{}", "─".repeat(82));
    println!(
        "{:<45} {:>7.2} {:>7.1} {:>5.1}% {:>3}/{:<3} save £{:.2}",
        "★ ADAPTIVE (best per night)",
        adaptive.total_cost / 100.0,
        adaptive.total_kwh,
        adaptive_cosy_pct,
        adaptive.feasible_count,
        nights.len(),
        adaptive_saving / 100.0
    );

    // 5. Night-by-night sample
    println!("\n{}", "=".repeat(78));
    println!("SAMPLE NIGHTS (every ~{}th)", (nights.len() / 20).max(1));
    println!("{}", "=".repeat(78));
    println!(
        "{:<12} {:>5} {:>5} {:>6} {:>6} {:>5} {:>5}  Strategy",
        "Date", "T_out", "T_in", "Act", "Best", "Save", "T@07"
    );
    println!("{}", "─".repeat(90));

    let step = (night_summaries.len() / 20).max(1);
    for (i, ns) in night_summaries.iter().enumerate() {
        if i % step != 0 {
            continue;
        }
        let saving = ns.actual.cost_pence - ns.best.cost_pence;
        println!(
            "{:<12} {:>4.1}° {:>4.1}° {:>5.0}p {:>5.0}p {:>+4.0}p {:>4.1}°  {}",
            ns.date,
            ns.avg_outside,
            ns.indoor_start,
            ns.actual.cost_pence,
            ns.best.cost_pence,
            saving,
            ns.best.indoor_t_07,
            ns.best_label,
        );
    }

    // 6. Monthly summary
    println!("\n{}", "=".repeat(78));
    println!("MONTHLY SUMMARY (adaptive strategy)");
    println!("{}", "=".repeat(78));
    println!(
        "{:<10} {:>6} {:>8} {:>8} {:>8} {:>7}",
        "Month", "Nights", "Actual£", "Optim£", "Save£", "Save%"
    );
    println!("{}", "─".repeat(52));

    let mut monthly: std::collections::BTreeMap<(i32, u32), (usize, f64, f64)> =
        std::collections::BTreeMap::new();
    for ns in &night_summaries {
        let key = (ns.date.year(), ns.date.month());
        let e = monthly.entry(key).or_insert((0, 0.0, 0.0));
        e.0 += 1;
        e.1 += ns.actual.cost_pence;
        e.2 += ns.best.cost_pence;
    }

    let mut grand_actual = 0.0;
    let mut grand_optimal = 0.0;
    for ((year, month), (n, act, opt)) in &monthly {
        let saving = act - opt;
        let pct = if *act > 0.0 {
            saving / act * 100.0
        } else {
            0.0
        };
        println!(
            "{}-{:02}    {:>5}  {:>7.2}  {:>7.2}  {:>7.2}  {:>5.1}%",
            year,
            month,
            n,
            act / 100.0,
            opt / 100.0,
            saving / 100.0,
            pct
        );
        grand_actual += act;
        grand_optimal += opt;
    }

    let grand_saving = grand_actual - grand_optimal;
    println!("{}", "─".repeat(52));
    println!(
        "{:<10} {:>6} {:>8.2} {:>8.2} {:>8.2}  {:>5.1}%",
        "TOTAL",
        nights.len(),
        grand_actual / 100.0,
        grand_optimal / 100.0,
        grand_saving / 100.0,
        if grand_actual > 0.0 {
            grand_saving / grand_actual * 100.0
        } else {
            0.0
        }
    );

    // Annual projection
    let days_covered = (last - first).num_days().max(1) as f64;
    let annual_factor = 365.0 / days_covered;
    println!(
        "\nProjected annual: actual £{:.0}, optimal £{:.0}, saving £{:.0}/yr ({:.0}%)",
        grand_actual / 100.0 * annual_factor,
        grand_optimal / 100.0 * annual_factor,
        grand_saving / 100.0 * annual_factor,
        if grand_actual > 0.0 {
            grand_saving / grand_actual * 100.0
        } else {
            0.0
        }
    );

    // 7. Sensitivity sweep: how much is comfort flexibility worth?
    println!("\n{}", "=".repeat(78));
    println!("TARGET TEMPERATURE SENSITIVITY");
    println!("(HP stays ON from 04:00 — lower target just means cooler at 07:00,");
    println!(" house catches up during morning at blended rate regardless)");
    println!("{}", "=".repeat(78));
    println!(
        "\n{:>10} {:>8} {:>8} {:>8} {:>7} {:>6} {:>8}",
        "Target", "Optimal£", "Save£", "Save/yr", "Save%", "≥tgt", "Avg T@07"
    );
    println!("{}", "─".repeat(62));

    for &target in &TARGET_SWEEP {
        let ar = compute_adaptive(target);
        let saving = actual_total_cost - ar.total_cost;
        let pct = if actual_total_cost > 0.0 {
            saving / actual_total_cost * 100.0
        } else {
            0.0
        };
        let annual = saving / 100.0 * annual_factor;
        println!(
            "{:>8.1}°C {:>8.2} {:>8.2} {:>6.0}/yr {:>5.1}% {:>3}/{:<3} {:>6.1}°C",
            target,
            ar.total_cost / 100.0,
            saving / 100.0,
            annual,
            pct,
            ar.feasible_count,
            nights.len(),
            ar.avg_min_t07,
        );
    }

    // Key insight
    let strict = compute_adaptive(19.5);
    let relaxed = compute_adaptive(18.0);
    let flex_value = strict.total_cost - relaxed.total_cost;
    println!(
        "\nFlexing from 19.5°C → 18.0°C target saves an extra £{:.2} ({:.0}/yr)",
        flex_value / 100.0,
        flex_value / 100.0 * annual_factor,
    );
    println!(
        "  19.5°C: {} nights use OFF strategy, rest fall back to actual",
        strict.feasible_count
    );
    println!(
        "  18.0°C: {} nights use OFF strategy — {} more cold-night savings",
        relaxed.feasible_count,
        relaxed.feasible_count - strict.feasible_count
    );

    // 8. Strategy breakdown: what gets picked at 18.5°C target?
    let recommended_target = 18.5;
    println!("\n{}", "=".repeat(78));
    println!(
        "STRATEGY BREAKDOWN (adaptive at {:.1}°C target)",
        recommended_target
    );
    println!("{}", "=".repeat(78));

    // Count how often each strategy is picked
    let mut picks: std::collections::HashMap<String, (usize, f64)> =
        std::collections::HashMap::new();
    let mut dhw_first_count = 0usize;
    let mut heat_first_count = 0usize;
    let mut dhw_norm_count = 0usize;
    let mut dhw_eco_count = 0usize;
    let mut off_count = 0usize;
    let mut continuous_count = 0usize;
    let mut actual_count = 0usize;

    for nr in &all_nights {
        let best_feasible = nr
            .sim_results
            .iter()
            .filter(|r| r.indoor_t_07 >= recommended_target)
            .min_by(|a, b| a.cost_pence.partial_cmp(&b.cost_pence).unwrap());

        let label = if let Some(bf) = best_feasible {
            if bf.cost_pence <= nr.actual.cost_pence {
                let l = &schedules[bf.schedule_idx].label;
                if let (Some(dhw_pos), Some(heat_pos)) = (l.find("DHW"), l.find("heat")) {
                    if heat_pos < dhw_pos {
                        heat_first_count += 1;
                    } else {
                        dhw_first_count += 1;
                    }
                }
                if l.contains("norm") {
                    dhw_norm_count += 1;
                } else if l.contains("eco") {
                    dhw_eco_count += 1;
                }
                if l.starts_with("OFF") {
                    off_count += 1;
                } else if l.starts_with("continuous") {
                    continuous_count += 1;
                }
                l.clone()
            } else {
                actual_count += 1;
                "actual (cheapest)".into()
            }
        } else {
            actual_count += 1;
            "actual (cheapest)".into()
        };

        let entry = picks.entry(label).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += nr.actual.cost_pence
            - if let Some(bf) = best_feasible {
                if bf.cost_pence <= nr.actual.cost_pence {
                    bf.cost_pence
                } else {
                    nr.actual.cost_pence
                }
            } else {
                nr.actual.cost_pence
            };
    }

    // Sort by frequency
    let mut pick_list: Vec<_> = picks.into_iter().collect();
    pick_list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

    println!("\n{:<50} {:>6} {:>8}", "Strategy", "Nights", "Saved");
    println!("{}", "─".repeat(66));
    for (label, (count, saving)) in &pick_list {
        println!("{:<50} {:>5}  {:>7.2}", label, count, saving / 100.0,);
    }

    println!("\n--- DHW Summary ---");
    println!("  Heating OFF + Cosy recovery: {} nights", off_count);
    println!(
        "  Continuous (no OFF):          {} nights",
        continuous_count
    );
    println!("  Actual (no change):           {} nights", actual_count);
    println!();
    println!("  DHW normal (~1h):  {} nights", dhw_norm_count);
    println!("  DHW eco (~2h):     {} nights", dhw_eco_count);
    println!();
    println!("  Heat first, DHW at end:  {} nights", heat_first_count);
    println!("  DHW first, heat after:   {} nights", dhw_first_count);

    Ok(())
}
