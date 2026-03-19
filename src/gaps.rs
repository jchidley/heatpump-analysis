//! Gap detection and synthetic data generation.
//!
//! # Strategy
//!
//! We have cumulative energy meters (elec_kwh, heat_kwh) that run continuously
//! even when the monitoring logger drops out. This gives us the **total energy**
//! consumed/produced during each gap. We use this as a hard constraint.
//!
//! For instantaneous feeds (power, temps, flow rate), we estimate values based
//! on the outside temperature at each gap minute, using a regression model
//! built from the real data at similar outside temperatures.
//!
//! ## Gap classification
//!
//! 1. **Short gaps (< 10 min)**: Linear interpolation. Not worth modelling.
//! 2. **Medium gaps (10 min – 48 hrs)**: Model from outside temp + time of day.
//!    Scale power estimates so integrated energy matches cumulative meters.
//! 3. **Long gaps (> 48 hrs)**: Same model, but flag as lower confidence.
//!    The 54-day summer gap (May–Jul 2025) falls here — mostly DHW only.
//!
//! ## Marking
//!
//! All synthetic data is stored with `is_simulated = 1` in the database.
//! Analysis functions can choose to include or exclude it.

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::config::config;

/// Gap in a feed's data.
#[derive(Debug)]
pub struct Gap {
    pub start_ts: i64,   // unix ms, last real sample before gap
    pub end_ts: i64,     // unix ms, first real sample after gap
    pub duration_min: f64,
    pub elec_before: f64, // cumulative kWh at gap start
    pub elec_after: f64,  // cumulative kWh at gap end
    pub heat_before: f64,
    pub heat_after: f64,
}

/// Ensure the simulated data table exists.
pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS simulated_samples (
            feed_id      TEXT NOT NULL,
            timestamp    INTEGER NOT NULL,
            value        REAL,
            gap_start_ts INTEGER NOT NULL,  -- links back to which gap produced this
            PRIMARY KEY (feed_id, timestamp)
        ) WITHOUT ROWID;

        CREATE TABLE IF NOT EXISTS gap_log (
            start_ts     INTEGER PRIMARY KEY,
            end_ts       INTEGER NOT NULL,
            duration_min REAL NOT NULL,
            elec_kwh     REAL,  -- energy consumed during gap (from cumulative meter)
            heat_kwh     REAL,  -- heat delivered during gap
            method       TEXT NOT NULL,  -- 'interpolate', 'model', 'model_low_confidence'
            samples_generated INTEGER NOT NULL DEFAULT 0
        );
        ",
    )?;
    Ok(())
}

/// Find all gaps in the heat pump data feeds (> min_gap_minutes).
pub fn find_gaps(conn: &Connection, min_gap_minutes: f64) -> Result<Vec<Gap>> {
    let feeds = &config().emoncms;
    let elec_power_id = feeds.feed_id("elec_power");
    let elec_energy_id = feeds.feed_id("elec_energy");
    let heat_energy_id = feeds.feed_id("heat_energy");
    let min_gap_ms = (min_gap_minutes * 60_000.0) as i64;

    // Find gaps in elec_power feed as the primary indicator
    let mut stmt = conn.prepare(&format!(
        "SELECT timestamp, value,
                LEAD(timestamp) OVER (ORDER BY timestamp) AS next_ts,
                LEAD(value) OVER (ORDER BY timestamp) AS next_val
         FROM samples WHERE feed_id = '{}'",
        elec_power_id
    ))?;

    let mut gaps = Vec::new();

    let rows: Vec<(i64, f64, Option<i64>, Option<f64>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (ts, _val, next_ts, _next_val) in &rows {
        if let Some(nts) = next_ts {
            let gap_ms = nts - ts;
            if gap_ms > min_gap_ms {
                // Look up cumulative energy at boundaries
                let elec_before = get_nearest_value(conn, elec_energy_id, *ts)?;
                let elec_after = get_nearest_value(conn, elec_energy_id, *nts)?;
                let heat_before = get_nearest_value(conn, heat_energy_id, *ts)?;
                let heat_after = get_nearest_value(conn, heat_energy_id, *nts)?;

                gaps.push(Gap {
                    start_ts: *ts,
                    end_ts: *nts,
                    duration_min: gap_ms as f64 / 60_000.0,
                    elec_before,
                    elec_after,
                    heat_before,
                    heat_after,
                });
            }
        }
    }

    Ok(gaps)
}

/// Get the nearest value for a cumulative feed at a timestamp.
fn get_nearest_value(conn: &Connection, feed_id: &str, ts: i64) -> Result<f64> {
    // Try exact match first, then nearest before, then nearest after
    let val: f64 = conn
        .query_row(
            "SELECT value FROM samples
             WHERE feed_id = ?1 AND timestamp <= ?2
             ORDER BY timestamp DESC LIMIT 1",
            params![feed_id, ts],
            |row| row.get(0),
        )
        .unwrap_or(0.0);
    Ok(val)
}

/// Build a simple model: given outside temperature, what's the typical
/// elec_w, heat_w, flow_t, return_t, flow_rate for heating and DHW?
///
/// Returns binned averages in 1°C outside temp bands.
pub struct TempBinModel {
    /// For each integer outside temp: (avg_elec, avg_heat, avg_flow_t, avg_return_t, avg_flow_rate, fraction_running)
    pub heating_bins: std::collections::HashMap<i32, BinStats>,
    pub dhw_bins: std::collections::HashMap<i32, BinStats>,
    /// Fraction of time spent in DHW vs heating (by hour of day)
    pub dhw_fraction_by_hour: [f64; 24],
}

#[derive(Debug, Clone, Default)]
pub struct BinStats {
    pub avg_elec: f64,
    pub avg_heat: f64,
    pub avg_flow_t: f64,
    pub avg_return_t: f64,
    pub avg_flow_rate: f64,
    pub fraction_running: f64,
    pub _count: u64,
}

impl TempBinModel {
    /// Build the model from real (non-simulated) data in the database.
    pub fn from_db(conn: &Connection) -> Result<Self> {
        use std::collections::HashMap;

        let cfg = config();
        let feeds = &cfg.emoncms;
        let thresholds = &cfg.thresholds;

        let mut heating_accum: HashMap<i32, (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>)> =
            HashMap::new();
        let mut dhw_accum: HashMap<i32, (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>)> =
            HashMap::new();
        let mut total_by_temp: HashMap<i32, u64> = HashMap::new();
        let mut dhw_count_by_hour: [u64; 24] = [0; 24];
        let mut total_count_by_hour: [u64; 24] = [0; 24];

        // Query all real data with all feeds joined
        // This is a big query but SQLite handles it fine
        let mut stmt = conn.prepare(&format!(
            "SELECT
                s_elec.timestamp,
                s_elec.value AS elec_w,
                s_heat.value AS heat_w,
                s_ft.value AS flow_t,
                s_rt.value AS return_t,
                s_fr.value AS flow_rate,
                s_ot.value AS outside_t
             FROM samples s_elec
             JOIN samples s_heat ON s_heat.feed_id = '{}' AND s_heat.timestamp = s_elec.timestamp
             JOIN samples s_ft   ON s_ft.feed_id   = '{}' AND s_ft.timestamp   = s_elec.timestamp
             JOIN samples s_rt   ON s_rt.feed_id   = '{}' AND s_rt.timestamp   = s_elec.timestamp
             JOIN samples s_fr   ON s_fr.feed_id   = '{}' AND s_fr.timestamp   = s_elec.timestamp
             JOIN samples s_ot   ON s_ot.feed_id   = '{}' AND s_ot.timestamp   = s_elec.timestamp
             WHERE s_elec.feed_id = '{}'
               AND s_elec.value > {}
               AND s_heat.value > 0
               AND (s_ft.value - s_rt.value) > {}",
            feeds.feed_id("heat_power"),
            feeds.feed_id("flow_temp"),
            feeds.feed_id("return_temp"),
            feeds.feed_id("flow_rate"),
            feeds.feed_id("outside_temp"),
            feeds.feed_id("elec_power"),
            thresholds.elec_running_w,
            thresholds.defrost_dt_threshold,
        ))?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?;

        for row in rows {
            let (ts, elec, heat, flow_t, return_t, flow_rate, outside_t) = row?;
            let temp_bin = outside_t.round() as i32;
            let hour = ((ts / 1000 % 86400) / 3600) as usize;
            let is_dhw = flow_rate >= thresholds.dhw_enter_flow_rate;

            let accum = if is_dhw {
                &mut dhw_accum
            } else {
                &mut heating_accum
            };

            let entry = accum.entry(temp_bin).or_insert_with(|| {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
            });
            entry.0.push(elec);
            entry.1.push(heat);
            entry.2.push(flow_t);
            entry.3.push(return_t);
            entry.4.push(flow_rate);

            *total_by_temp.entry(temp_bin).or_default() += 1;

            if is_dhw {
                dhw_count_by_hour[hour] += 1;
            }
            total_count_by_hour[hour] += 1;
        }

        // Convert accumulators to BinStats
        let to_bins =
            |accum: HashMap<i32, (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>)>,
             total: &HashMap<i32, u64>|
             -> HashMap<i32, BinStats> {
                accum
                    .into_iter()
                    .map(|(temp, (e, h, ft, rt, fr))| {
                        let n = e.len() as f64;
                        let total_n = *total.get(&temp).unwrap_or(&1) as f64;
                        (
                            temp,
                            BinStats {
                                avg_elec: e.iter().sum::<f64>() / n,
                                avg_heat: h.iter().sum::<f64>() / n,
                                avg_flow_t: ft.iter().sum::<f64>() / n,
                                avg_return_t: rt.iter().sum::<f64>() / n,
                                avg_flow_rate: fr.iter().sum::<f64>() / n,
                                fraction_running: n / total_n,
                                _count: e.len() as u64,
                            },
                        )
                    })
                    .collect()
            };

        let heating_bins = to_bins(heating_accum, &total_by_temp);
        let dhw_bins = to_bins(dhw_accum, &total_by_temp);

        let mut dhw_fraction_by_hour = [0.0; 24];
        for h in 0..24 {
            if total_count_by_hour[h] > 0 {
                dhw_fraction_by_hour[h] =
                    dhw_count_by_hour[h] as f64 / total_count_by_hour[h] as f64;
            }
        }

        Ok(TempBinModel {
            heating_bins,
            dhw_bins,
            dhw_fraction_by_hour,
        })
    }

    /// Estimate values for a given outside temperature and hour.
    /// Returns (elec_w, heat_w, flow_t, return_t, flow_rate, is_dhw)
    /// or None if the system would likely be idle.
    fn get_heating(&self, temp_bin: i32) -> Option<&BinStats> {
        // Try exact, then ±1, ±2
        self.heating_bins
            .get(&temp_bin)
            .or_else(|| self.heating_bins.get(&(temp_bin - 1)))
            .or_else(|| self.heating_bins.get(&(temp_bin + 1)))
            .or_else(|| self.heating_bins.get(&(temp_bin - 2)))
            .or_else(|| self.heating_bins.get(&(temp_bin + 2)))
    }

    pub fn get_dhw(&self, temp_bin: i32) -> Option<&BinStats> {
        self.dhw_bins
            .get(&temp_bin)
            .or_else(|| self.dhw_bins.get(&(temp_bin - 1)))
            .or_else(|| self.dhw_bins.get(&(temp_bin + 1)))
            .or_else(|| self.dhw_bins.get(&(temp_bin - 2)))
            .or_else(|| self.dhw_bins.get(&(temp_bin + 2)))
    }
}

/// Fill a single gap with synthetic data.
///
/// 1. Look up outside temperature for each minute in the gap.
/// 2. Estimate instantaneous values from the model.
/// 3. Scale so that integrated energy matches cumulative meters.
/// 4. Store in `simulated_samples` table.
pub fn fill_gap(conn: &Connection, gap: &Gap, model: &TempBinModel) -> Result<u64> {
    let method = if gap.duration_min < 10.0 {
        "interpolate"
    } else if gap.duration_min <= 48.0 * 60.0 {
        "model"
    } else {
        "model_low_confidence"
    };

    let elec_kwh_gap = gap.elec_after - gap.elec_before;
    let heat_kwh_gap = gap.heat_after - gap.heat_before;

    // For very short gaps, just interpolate
    if method == "interpolate" {
        return fill_gap_interpolate(conn, gap);
    }

    // Get outside temperature for each minute in the gap
    let outside_temp_id = config().emoncms.feed_id("outside_temp");
    let mut minutes: Vec<(i64, f64)> = Vec::new(); // (timestamp_ms, outside_t)
    let mut ts = gap.start_ts + 60_000; // skip the boundary sample

    while ts < gap.end_ts {
        let outside_t: f64 = conn
            .query_row(
                &format!(
                    "SELECT value FROM samples
                     WHERE feed_id = '{}' AND timestamp <= ?1
                     ORDER BY timestamp DESC LIMIT 1",
                    outside_temp_id
                ),
                params![ts],
                |row| row.get(0),
            )
            .unwrap_or(10.0); // fallback

        minutes.push((ts, outside_t));
        ts += 60_000;
    }

    if minutes.is_empty() {
        return Ok(0);
    }

    // Generate raw estimates
    struct Estimate {
        ts: i64,
        elec: f64,
        heat: f64,
        flow_t: f64,
        return_t: f64,
        flow_rate: f64,
    }

    let mut estimates: Vec<Estimate> = Vec::new();
    let mut raw_elec_sum = 0.0;
    let mut raw_heat_sum = 0.0;

    for (ts, outside_t) in &minutes {
        let hour = ((*ts / 1000 % 86400) / 3600) as usize;
        let temp_bin = outside_t.round() as i32;

        // Decide DHW or heating for this minute based on hourly fraction
        let dhw_frac = model.dhw_fraction_by_hour[hour];
        let minute_in_hour = ((*ts / 1000 % 3600) / 60) as f64;
        let is_dhw_minute = minute_in_hour < (dhw_frac * 60.0);

        let (elec, heat, ft, rt, fr) = if is_dhw_minute {
            if let Some(dhw) = model.get_dhw(temp_bin) {
                (dhw.avg_elec, dhw.avg_heat, dhw.avg_flow_t, dhw.avg_return_t, dhw.avg_flow_rate)
            } else if let Some(stats) = model.get_heating(temp_bin) {
                (stats.avg_elec, stats.avg_heat, stats.avg_flow_t, stats.avg_return_t, stats.avg_flow_rate)
            } else {
                continue;
            }
        } else if let Some(stats) = model.get_heating(temp_bin) {
            // Apply fraction_running: some minutes the HP is idle
            if minute_in_hour >= (stats.fraction_running * 60.0) {
                // Idle minute
                estimates.push(Estimate {
                    ts: *ts,
                    elec: 0.0,
                    heat: 0.0,
                    flow_t: 20.0, // ambient-ish
                    return_t: 20.0,
                    flow_rate: 0.0,
                });
                continue;
            }
            (stats.avg_elec, stats.avg_heat, stats.avg_flow_t, stats.avg_return_t, stats.avg_flow_rate)
        } else {
            continue;
        };

        raw_elec_sum += elec;
        raw_heat_sum += heat;
        estimates.push(Estimate {
            ts: *ts,
            elec,
            heat,
            flow_t: ft,
            return_t: rt,
            flow_rate: fr,
        });
    }

    // Scale power estimates so integrated energy matches cumulative meters
    // Energy (kWh) = sum of power (W) * 1 minute / 60 / 1000
    let raw_elec_kwh = raw_elec_sum / 60.0 / 1000.0;
    let raw_heat_kwh = raw_heat_sum / 60.0 / 1000.0;

    let elec_scale = if raw_elec_kwh > 0.01 {
        elec_kwh_gap / raw_elec_kwh
    } else {
        1.0
    };
    let heat_scale = if raw_heat_kwh > 0.01 {
        heat_kwh_gap / raw_heat_kwh
    } else {
        1.0
    };

    // Insert scaled estimates
    let tx = conn.unchecked_transaction()?;
    let mut count = 0u64;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO simulated_samples
             (feed_id, timestamp, value, gap_start_ts)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        let sim_feeds = &config().emoncms;
        for est in &estimates {
            let values: [(&str, f64); 5] = [
                (sim_feeds.feed_id("elec_power"), est.elec * elec_scale),
                (sim_feeds.feed_id("heat_power"), est.heat * heat_scale),
                (sim_feeds.feed_id("flow_temp"), est.flow_t),
                (sim_feeds.feed_id("return_temp"), est.return_t),
                (sim_feeds.feed_id("flow_rate"), est.flow_rate),
            ];

            for (fid, val) in &values {
                stmt.execute(params![fid, est.ts, val, gap.start_ts])?;
                count += 1;
            }
        }
    }

    // Log the gap
    tx.execute(
        "INSERT OR REPLACE INTO gap_log
         (start_ts, end_ts, duration_min, elec_kwh, heat_kwh, method, samples_generated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            gap.start_ts,
            gap.end_ts,
            gap.duration_min,
            elec_kwh_gap,
            heat_kwh_gap,
            method,
            count,
        ],
    )?;

    tx.commit()?;
    Ok(count)
}

/// Simple linear interpolation for short gaps.
fn fill_gap_interpolate(conn: &Connection, gap: &Gap) -> Result<u64> {
    let feeds = ["503094", "503096", "503098", "503099", "503100"];
    let mut count = 0u64;

    let tx = conn.unchecked_transaction()?;

    for feed_id in &feeds {
        let start_val: f64 = conn
            .query_row(
                "SELECT value FROM samples WHERE feed_id = ?1 AND timestamp = ?2",
                params![feed_id, gap.start_ts],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        let end_val: f64 = conn
            .query_row(
                "SELECT value FROM samples WHERE feed_id = ?1 AND timestamp = ?2",
                params![feed_id, gap.end_ts],
                |row| row.get(0),
            )
            .unwrap_or(start_val);

        let n_minutes = ((gap.end_ts - gap.start_ts) / 60_000 - 1).max(0);

        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO simulated_samples
             (feed_id, timestamp, value, gap_start_ts)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        for i in 1..=n_minutes {
            let ts = gap.start_ts + i * 60_000;
            let frac = i as f64 / (n_minutes + 1) as f64;
            let val = start_val + (end_val - start_val) * frac;
            stmt.execute(params![feed_id, ts, val, gap.start_ts])?;
            count += 1;
        }
    }

    tx.execute(
        "INSERT OR REPLACE INTO gap_log
         (start_ts, end_ts, duration_min, elec_kwh, heat_kwh, method, samples_generated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            gap.start_ts,
            gap.end_ts,
            gap.duration_min,
            gap.elec_after - gap.elec_before,
            gap.heat_after - gap.heat_before,
            "interpolate",
            count,
        ],
    )?;

    tx.commit()?;
    Ok(count)
}

/// Print a summary of all gaps and their fill status.
pub fn print_gap_report(conn: &Connection) -> Result<()> {
    let gaps = find_gaps(conn, 3.0)?;

    println!("\n=== Gap Report ===");
    println!(
        "{:<22} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "Start", "Duration", "Elec kWh", "Heat kWh", "Gap COP", "Status"
    );
    println!("{}", "-".repeat(80));

    for gap in &gaps {
        let start = chrono::DateTime::from_timestamp_millis(gap.start_ts)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();

        let elec = gap.elec_after - gap.elec_before;
        let heat = gap.heat_after - gap.heat_before;
        let cop = if elec > 0.1 {
            format!("{:.2}", heat / elec)
        } else {
            "-".to_string()
        };

        let dur = if gap.duration_min > 1440.0 {
            format!("{:.1}d", gap.duration_min / 1440.0)
        } else if gap.duration_min > 60.0 {
            format!("{:.1}h", gap.duration_min / 60.0)
        } else {
            format!("{:.0}m", gap.duration_min)
        };

        // Check if filled
        let filled: u64 = conn
            .query_row(
                "SELECT COALESCE(samples_generated, 0) FROM gap_log WHERE start_ts = ?1",
                params![gap.start_ts],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let status = if filled > 0 {
            format!("filled ({})", filled)
        } else {
            "unfilled".to_string()
        };

        println!(
            "{:<22} {:>10} {:>10.1} {:>10.1} {:>10} {:>12}",
            start, dur, elec, heat, cop, status
        );
    }

    // Summary
    let total_gap_min: f64 = gaps.iter().map(|g| g.duration_min).sum();
    let total_elec: f64 = gaps.iter().map(|g| g.elec_after - g.elec_before).sum();
    let total_heat: f64 = gaps.iter().map(|g| g.heat_after - g.heat_before).sum();

    println!(
        "\nTotal gap time: {:.1} days ({:.1} hours)",
        total_gap_min / 1440.0,
        total_gap_min / 60.0,
    );
    println!(
        "Energy during gaps: {:.1} kWh elec, {:.1} kWh heat (COP {:.2})",
        total_elec,
        total_heat,
        if total_elec > 0.1 {
            total_heat / total_elec
        } else {
            0.0
        },
    );

    let sim_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM simulated_samples",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("Simulated samples in DB: {}", sim_count);

    Ok(())
}
