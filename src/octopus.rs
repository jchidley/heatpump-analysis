//! Load Octopus Energy consumption and weather data, combining the octopus
//! project's CSV + JSON with emoncms outside temperature for accuracy.
//!
//! Temperature hierarchy:
//!   1. emoncms feed 503093 (Met Office hourly, Oct 2024+) — most accurate
//!   2. ERA5-Land from octopus weather.json (Apr 2020+) — bias-corrected using
//!      the overlap period (+1.0°C systematic offset vs emoncms)
//!
//! Consumption data lives at `~/github/octopus/data/usage_merged.csv`.
//! Gas values are stored in m³ and converted to kWh here using the calorific
//! value and correction factor from the octopus project's config.json.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use polars::prelude::*;
use serde::Deserialize;

use crate::config::config;

/// Mean bias: emoncms reads this much warmer than ERA5 (from 507-day overlap).
/// Applied to ERA5 temps for gas-era days where no emoncms data exists.
const ERA5_BIAS_CORRECTION_C: f64 = 1.0;

/// Default location of the octopus project's data directory.
fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/jack".to_string());
    PathBuf::from(home)
        .join("github")
        .join("octopus")
        .join("data")
}

// ── Data schemas ─────────────────────────────────────────────────────────────

/// Gas unit conversion config from the octopus project's config.json.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OctopusConfig {
    #[serde(default = "default_gas_units")]
    gas_units: String,
    #[serde(default = "default_calorific_value")]
    gas_calorific_value: f64,
    #[serde(default = "default_correction_factor")]
    gas_correction_factor: f64,
}

fn default_gas_units() -> String {
    "m3".to_string()
}
fn default_calorific_value() -> f64 {
    39.2
}
fn default_correction_factor() -> f64 {
    1.02264
}

/// Convert gas m³ to kWh using standard UK formula.
fn gas_m3_to_kwh(m3: f64, calorific_value: f64, correction_factor: f64) -> f64 {
    (m3 * correction_factor * calorific_value) / 3.6
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WeatherRecord {
    date: String,
    tmean_c: Option<f64>,
    #[allow(dead_code)]
    hdd: f64,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Load half-hourly consumption data from CSV as a Polars DataFrame.
///
/// Gas values are stored in m³ in the CSV and converted to kWh here using
/// the calorific value and correction factor from config.json.
///
/// Columns: timestamp (Datetime ms UTC), fuel (Utf8), kwh (f64)
pub fn load_consumption(data_dir: Option<&Path>) -> Result<DataFrame> {
    let dir = data_dir.map(PathBuf::from).unwrap_or_else(default_data_dir);

    // Load gas conversion config
    let config_path = dir.join("config.json");
    let octopus_cfg: OctopusConfig = if config_path.exists() {
        let text = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Cannot read {}", config_path.display()))?;
        serde_json::from_str(&text).context("Failed to parse config.json")?
    } else {
        OctopusConfig {
            gas_units: default_gas_units(),
            gas_calorific_value: default_calorific_value(),
            gas_correction_factor: default_correction_factor(),
        }
    };
    let need_gas_conversion = octopus_cfg.gas_units == "m3";

    // Read CSV: fuel,interval_start,interval_end,consumption_kwh
    let csv_path = dir.join("usage_merged.csv");
    let text = std::fs::read_to_string(&csv_path)
        .with_context(|| format!("Cannot read {}", csv_path.display()))?;

    let mut timestamps: Vec<i64> = Vec::new();
    let mut fuels: Vec<String> = Vec::new();
    let mut kwhs: Vec<f64> = Vec::new();

    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            continue; // skip header
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 4 {
            continue;
        }
        let fuel = fields[0].to_string();
        let interval_start = fields[1];
        let raw_value: f64 = fields[3]
            .parse()
            .with_context(|| format!("Bad value on line {}: {}", i + 1, fields[3]))?;

        let kwh = if fuel == "gas" && need_gas_conversion {
            gas_m3_to_kwh(
                raw_value,
                octopus_cfg.gas_calorific_value,
                octopus_cfg.gas_correction_factor,
            )
        } else {
            raw_value
        };

        let ts = chrono::DateTime::parse_from_rfc3339(interval_start)
            .with_context(|| format!("Bad timestamp on line {}: {}", i + 1, interval_start))?
            .timestamp_millis();

        timestamps.push(ts);
        fuels.push(fuel);
        kwhs.push(kwh);
    }

    let ts_series = Series::new("timestamp".into(), &timestamps)
        .cast(&DataType::Datetime(
            TimeUnit::Milliseconds,
            Some("UTC".into()),
        ))
        .context("timestamp cast")?;

    let df = DataFrame::new(vec![
        ts_series.into(),
        Series::new("fuel".into(), &fuels).into(),
        Series::new("kwh".into(), &kwhs).into(),
    ])
    .context("build consumption DataFrame")?;

    eprintln!(
        "Octopus consumption: {} records loaded from {}{}",
        df.height(),
        csv_path.display(),
        if need_gas_conversion {
            format!(
                " (gas: m³→kWh, CV={}, CF={})",
                octopus_cfg.gas_calorific_value, octopus_cfg.gas_correction_factor
            )
        } else {
            String::new()
        },
    );

    Ok(df)
}

/// Load daily weather data using a hybrid approach:
///   - emoncms outside_temp (feed 503093) where available — accurate local data
///   - ERA5-Land (weather.json) for earlier dates — bias-corrected by +1.0°C
///
/// Returns DataFrame with columns: date (Utf8), tmean_c (f64), hdd (f64), source (Utf8)
pub fn load_weather(
    data_dir: Option<&Path>,
    db_conn: Option<&rusqlite::Connection>,
) -> Result<DataFrame> {
    // 1. Load ERA5 baseline from octopus project
    let dir = data_dir.map(PathBuf::from).unwrap_or_else(default_data_dir);
    let path = dir.join("weather.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read {}", path.display()))?;
    let records: Vec<WeatherRecord> =
        serde_json::from_str(&text).context("Failed to parse weather.json")?;

    let mut era5: HashMap<String, f64> = HashMap::new();
    for r in &records {
        if let Some(t) = r.tmean_c {
            era5.insert(r.date.clone(), t);
        }
    }

    // 2. Load emoncms outside_temp if database available
    let mut emoncms: HashMap<String, f64> = HashMap::new();
    if let Some(conn) = db_conn {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(timestamp/1000, 'unixepoch') AS day, AVG(value) AS avg_t
             FROM samples
             WHERE feed_id = '{}'
             GROUP BY day
             ORDER BY day",
            config().emoncms.feed_id("outside_temp"),
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?;
        for row in rows {
            let (date, temp) = row?;
            emoncms.insert(date, temp);
        }
    }

    // 3. Build unified daily weather: prefer emoncms, fall back to bias-corrected ERA5
    let mut all_dates: Vec<String> = era5.keys().chain(emoncms.keys()).cloned().collect();
    all_dates.sort();
    all_dates.dedup();

    let mut dates: Vec<String> = Vec::with_capacity(all_dates.len());
    let mut temps: Vec<f64> = Vec::with_capacity(all_dates.len());
    let mut hdds: Vec<f64> = Vec::with_capacity(all_dates.len());
    let mut sources: Vec<String> = Vec::with_capacity(all_dates.len());

    let mut n_emoncms = 0u32;
    let mut n_era5_corrected = 0u32;

    for date in &all_dates {
        let (temp, source) = if let Some(&t) = emoncms.get(date) {
            n_emoncms += 1;
            (t, "emoncms")
        } else if let Some(&t) = era5.get(date) {
            n_era5_corrected += 1;
            (t + ERA5_BIAS_CORRECTION_C, "ERA5+1.0")
        } else {
            continue;
        };

        let hdd = (config().thresholds.hdd_base_temp_c - temp).max(0.0);
        dates.push(date.clone());
        temps.push(temp);
        hdds.push(hdd);
        sources.push(source.to_string());
    }

    let df = DataFrame::new(vec![
        Series::new("date".into(), &dates).into(),
        Series::new("tmean_c".into(), &temps).into(),
        Series::new("hdd".into(), &hdds).into(),
        Series::new("source".into(), &sources).into(),
    ])
    .context("build weather DataFrame")?;

    eprintln!(
        "Weather: {} days ({} emoncms, {} ERA5 bias-corrected +{:.1}°C)",
        df.height(),
        n_emoncms,
        n_era5_corrected,
        ERA5_BIAS_CORRECTION_C,
    );

    Ok(df)
}

// ── Analysis helpers ─────────────────────────────────────────────────────────

/// Aggregate half-hourly consumption to daily totals, split by fuel.
///
/// Returns DataFrame with columns: date (Utf8), elec_kwh (f64), gas_kwh (f64)
pub fn daily_totals(consumption: &DataFrame) -> Result<DataFrame> {
    let df = consumption.clone().lazy();

    // Extract date string from timestamp
    let daily = df
        .with_column(col("timestamp").dt().strftime("%Y-%m-%d").alias("date"))
        .group_by([col("date"), col("fuel")])
        .agg([col("kwh").sum()])
        .collect()
        .context("daily aggregation")?;

    // Split by fuel and join
    let elec_daily = daily
        .clone()
        .lazy()
        .filter(col("fuel").eq(lit("electricity")))
        .select([col("date"), col("kwh").alias("elec_kwh")])
        .collect()?;

    let gas_daily = daily
        .clone()
        .lazy()
        .filter(col("fuel").eq(lit("gas")))
        .select([col("date"), col("kwh").alias("gas_kwh")])
        .collect()?;

    let result = elec_daily
        .lazy()
        .join(
            gas_daily.lazy(),
            [col("date")],
            [col("date")],
            JoinArgs::new(JoinType::Left),
        )
        .with_column(col("gas_kwh").fill_null(lit(0.0)))
        .sort(["date"], Default::default())
        .collect()
        .context("join fuel columns")?;

    Ok(result)
}

/// Print a summary of Octopus data: coverage, totals, monthly breakdown.
pub fn print_summary(consumption: &DataFrame, weather: &DataFrame) -> Result<()> {
    let daily = daily_totals(consumption)?;
    let n_days = daily.height();

    // Overall totals
    let elec_total: f64 = daily.column("elec_kwh")?.f64()?.sum().unwrap_or(0.0);
    let gas_total: f64 = daily.column("gas_kwh")?.f64()?.sum().unwrap_or(0.0);

    let dates = daily.column("date")?.str()?;
    let first = dates.get(0).unwrap_or("?");
    let last = dates.get(n_days - 1).unwrap_or("?");

    println!("\n═══ Octopus Energy Data Summary ═══\n");
    println!("Coverage:     {} → {} ({} days)", first, last, n_days);
    println!(
        "Electricity:  {:.0} kWh total ({:.1} kWh/day avg)",
        elec_total,
        elec_total / n_days as f64
    );
    println!(
        "Gas:          {:.0} kWh total ({:.1} kWh/day avg over gas-era days)",
        gas_total,
        {
            let gas_days = daily
                .clone()
                .lazy()
                .filter(col("gas_kwh").gt(lit(0.0)))
                .collect()?
                .height();
            if gas_days > 0 {
                gas_total / gas_days as f64
            } else {
                0.0
            }
        }
    );

    // HDD summary by source
    let source_counts = weather
        .clone()
        .lazy()
        .group_by([col("source")])
        .agg([col("hdd").sum(), col("date").count().alias("days")])
        .sort(["source"], Default::default())
        .collect()?;
    let src_names = source_counts.column("source")?.str()?;
    let src_hdds = source_counts.column("hdd")?.f64()?;
    let src_days = source_counts.column("days")?.u32()?;

    let hdd_total: f64 = weather.column("hdd")?.f64()?.sum().unwrap_or(0.0);
    println!(
        "Degree days:  {:.0} HDD (base {:.1}°C) over {} days",
        hdd_total,
        config().thresholds.hdd_base_temp_c,
        weather.height()
    );
    for i in 0..source_counts.height() {
        let name = src_names.get(i).unwrap_or("?");
        let hdd = src_hdds.get(i).unwrap_or(0.0);
        let days = src_days.get(i).unwrap_or(0);
        println!("  {:>12}: {:.0} HDD over {} days", name, hdd, days);
    }

    // Monthly breakdown
    println!("\n── Monthly Breakdown ──\n");
    println!(
        "{:<10} {:>10} {:>10} {:>8} {:>10}",
        "Month", "Elec kWh", "Gas kWh", "HDD", "Elec/HDD"
    );
    println!("{}", "─".repeat(52));

    let monthly = daily
        .clone()
        .lazy()
        .with_column(col("date").str().head(lit(7)).alias("month"))
        .group_by([col("month")])
        .agg([col("elec_kwh").sum(), col("gas_kwh").sum()])
        .sort(["month"], Default::default())
        .collect()?;

    // Join with monthly HDD (drop source column before aggregation)
    let monthly_hdd = weather
        .clone()
        .lazy()
        .with_column(col("date").str().head(lit(7)).alias("month"))
        .group_by([col("month")])
        .agg([col("hdd").sum()])
        .collect()?;

    let joined = monthly
        .lazy()
        .join(
            monthly_hdd.lazy(),
            [col("month")],
            [col("month")],
            JoinArgs::new(JoinType::Left),
        )
        .sort(["month"], Default::default())
        .collect()?;

    let months = joined.column("month")?.str()?;
    let elecs = joined.column("elec_kwh")?.f64()?;
    let gases = joined.column("gas_kwh")?.f64()?;
    let hdds = joined.column("hdd")?.f64()?;

    for i in 0..joined.height() {
        let m = months.get(i).unwrap_or("?");
        let e = elecs.get(i).unwrap_or(0.0);
        let g = gases.get(i).unwrap_or(0.0);
        let h = hdds.get(i).unwrap_or(0.0);
        let e_per_hdd = if h > 0.0 {
            format!("{:.1}", e / h)
        } else {
            "-".to_string()
        };
        println!(
            "{:<10} {:>10.1} {:>10.1} {:>8.1} {:>10}",
            m, e, g, h, e_per_hdd
        );
    }

    Ok(())
}

/// Daily HP stats broken down by operating state from the state machine.
///
/// Returns (date, heating_elec_kwh, heating_heat_kwh, dhw_elec_kwh, dhw_heat_kwh)
pub fn daily_hp_by_state(enriched_df: &DataFrame) -> Result<Vec<(String, f64, f64, f64, f64)>> {
    // The enriched df has ~10s samples with elec_w, heat_w, state, timestamp.
    // Convert power (W) × sample interval to energy (kWh).
    // Samples are ~1 minute apart (60s) in the DB.
    const SAMPLE_HOURS: f64 = 1.0 / 60.0; // 1-minute samples → hours

    let daily = enriched_df
        .clone()
        .lazy()
        .with_column(col("timestamp").dt().strftime("%Y-%m-%d").alias("date"))
        // Heating energy
        .with_columns([
            when(col("state").eq(lit("heating")))
                .then(col("elec_w") * lit(SAMPLE_HOURS / 1000.0))
                .otherwise(lit(0.0))
                .alias("htg_elec_kwh"),
            when(col("state").eq(lit("heating")))
                .then(col("heat_w") * lit(SAMPLE_HOURS / 1000.0))
                .otherwise(lit(0.0))
                .alias("htg_heat_kwh"),
            when(col("state").eq(lit("dhw")))
                .then(col("elec_w") * lit(SAMPLE_HOURS / 1000.0))
                .otherwise(lit(0.0))
                .alias("dhw_elec_kwh"),
            when(col("state").eq(lit("dhw")))
                .then(col("heat_w") * lit(SAMPLE_HOURS / 1000.0))
                .otherwise(lit(0.0))
                .alias("dhw_heat_kwh"),
        ])
        .group_by([col("date")])
        .agg([
            col("htg_elec_kwh").sum(),
            col("htg_heat_kwh").sum(),
            col("dhw_elec_kwh").sum(),
            col("dhw_heat_kwh").sum(),
        ])
        .sort(["date"], Default::default())
        .collect()
        .context("daily HP by state aggregation")?;

    let dates = daily.column("date")?.str()?;
    let he = daily.column("htg_elec_kwh")?.f64()?;
    let hh = daily.column("htg_heat_kwh")?.f64()?;
    let de = daily.column("dhw_elec_kwh")?.f64()?;
    let dh = daily.column("dhw_heat_kwh")?.f64()?;

    let mut result = Vec::with_capacity(daily.height());
    for i in 0..daily.height() {
        result.push((
            dates.get(i).unwrap_or("").to_string(),
            he.get(i).unwrap_or(0.0),
            hh.get(i).unwrap_or(0.0),
            de.get(i).unwrap_or(0.0),
            dh.get(i).unwrap_or(0.0),
        ));
    }
    Ok(result)
}

// Gas-era constants are now in config().gas_era

/// Print a comparison of gas-era vs heat-pump-era energy use,
/// normalised by degree days, with heating and DHW separated.
///
/// HP-era uses the state machine to split heating from DHW (measured).
/// Gas-era uses the reference estimate of 11.82 kWh/day DHW.
///
/// Uses the cutover date of 2024-10-22 (HP monitoring start).
pub fn print_gas_vs_hp(
    consumption: &DataFrame,
    weather: &DataFrame,
    hp_by_state: Option<&[(String, f64, f64, f64, f64)]>, // from daily_hp_by_state
) -> Result<()> {
    let daily = daily_totals(consumption)?;
    let cutover = "2024-10-22";

    println!("\n═══ Gas Era vs Heat Pump Era ═══\n");
    println!("Temperature sources:");
    println!(
        "  Gas era:  ERA5-Land + {:.1}°C bias correction (derived from 507-day overlap)",
        ERA5_BIAS_CORRECTION_C
    );
    println!("  HP era:   emoncms feed 503093 (Met Office hourly) where available");
    println!("  HDD base: {:.1}°C", config().thresholds.hdd_base_temp_c);
    println!("  HP data:  state machine (heating vs DHW vs defrost) from 10s samples");
    println!();

    // Build HP daily lookup: date → (htg_elec, htg_heat, dhw_elec, dhw_heat)
    let hp_map: HashMap<String, (f64, f64, f64, f64)> = hp_by_state
        .map(|data| {
            data.iter()
                .map(|(d, he, hh, de, dh)| (d.clone(), (*he, *hh, *de, *dh)))
                .collect()
        })
        .unwrap_or_default();

    // Join daily totals with weather
    let joined = daily
        .lazy()
        .join(
            weather
                .clone()
                .lazy()
                .select([col("date"), col("hdd"), col("tmean_c"), col("source")]),
            [col("date")],
            [col("date")],
            JoinArgs::new(JoinType::Inner),
        )
        .collect()?;

    for (label, filter_expr) in [
        (
            "Gas era (heating season)",
            col("date")
                .lt(lit(cutover))
                .and(col("gas_kwh").gt(lit(0.0)))
                .and(col("hdd").gt(lit(0.5))),
        ),
        (
            "Heat pump era (heating)",
            col("date").gt_eq(lit(cutover)).and(col("hdd").gt(lit(0.5))),
        ),
    ] {
        let era = joined.clone().lazy().filter(filter_expr).collect()?;

        let n = era.height();
        if n == 0 {
            println!("{}: no data", label);
            continue;
        }

        let octopus_elec: f64 = era.column("elec_kwh")?.f64()?.sum().unwrap_or(0.0);
        let gas: f64 = era.column("gas_kwh")?.f64()?.sum().unwrap_or(0.0);
        let hdd: f64 = era.column("hdd")?.f64()?.sum().unwrap_or(0.0);

        // Count temp sources used
        let src_counts = era
            .clone()
            .lazy()
            .group_by([col("source")])
            .agg([col("date").count().alias("n")])
            .collect()?;

        println!("{}:", label);
        println!("  Days:            {}", n);

        let src_str: Vec<String> = (0..src_counts.height())
            .map(|i| {
                let s = src_counts
                    .column("source")
                    .unwrap()
                    .str()
                    .unwrap()
                    .get(i)
                    .unwrap_or("?");
                let c = src_counts
                    .column("n")
                    .unwrap()
                    .u32()
                    .unwrap()
                    .get(i)
                    .unwrap_or(0);
                format!("{}×{}", c, s)
            })
            .collect();
        println!("  Temp source:     {}", src_str.join(", "));
        println!("  Total HDD:       {:.0} ({:.1}/day)", hdd, hdd / n as f64);

        if gas > 0.0 {
            // Gas era — estimate DHW and subtract
            let total_gas_heat = gas * config().gas_era.boiler_efficiency;
            let dhw_heat = config().gas_era.dhw_kwh_per_day * n as f64;
            let heating_heat = total_gas_heat - dhw_heat;
            let dhw_gas = dhw_heat / config().gas_era.boiler_efficiency;
            let heating_gas = gas - dhw_gas;

            println!(
                "  Whole-house elec:{:.0} kWh ({:.1}/day)",
                octopus_elec,
                octopus_elec / n as f64
            );
            println!(
                "  Total gas:       {:.0} kWh ({:.1}/day)",
                gas,
                gas / n as f64
            );
            println!(
                "  Boiler eff:      {:.0}%%",
                config().gas_era.boiler_efficiency * 100.0
            );
            println!("  ── Heating ──");
            println!(
                "    Gas input:     {:.0} kWh ({:.1}/day)",
                heating_gas,
                heating_gas / n as f64
            );
            println!(
                "    Heat delivered:{:.0} kWh ({:.1}/day)",
                heating_heat,
                heating_heat / n as f64
            );
            println!("    Heat/HDD:      {:.1} kWh/HDD", heating_heat / hdd);
            println!(
                "  ── DHW (est. {:.1} kWh/day) ──",
                config().gas_era.dhw_kwh_per_day
            );
            println!(
                "    Gas input:     {:.0} kWh ({:.1}/day)",
                dhw_gas,
                dhw_gas / n as f64
            );
            println!(
                "    Heat delivered:{:.0} kWh ({:.1}/day)",
                dhw_heat,
                dhw_heat / n as f64
            );
        } else if !hp_map.is_empty() {
            // HP era — use state machine data
            let era_dates = era.column("date")?.str()?;
            let mut htg_elec = 0.0f64;
            let mut htg_heat = 0.0f64;
            let mut dhw_elec = 0.0f64;
            let mut dhw_heat = 0.0f64;
            let mut matched = 0u32;
            for i in 0..era.height() {
                if let Some(d) = era_dates.get(i) {
                    if let Some(&(he, hh, de, dh)) = hp_map.get(d) {
                        htg_elec += he;
                        htg_heat += hh;
                        dhw_elec += de;
                        dhw_heat += dh;
                        matched += 1;
                    }
                }
            }
            if matched > 0 {
                let total_hp_elec = htg_elec + dhw_elec;
                let baseload = octopus_elec - total_hp_elec;
                let htg_cop = if htg_elec > 0.0 {
                    htg_heat / htg_elec
                } else {
                    0.0
                };
                let dhw_cop = if dhw_elec > 0.0 {
                    dhw_heat / dhw_elec
                } else {
                    0.0
                };

                println!(
                    "  Whole-house elec:{:.0} kWh ({:.1}/day) (Octopus meter)",
                    octopus_elec,
                    octopus_elec / n as f64
                );
                println!(
                    "  HP total elec:   {:.0} kWh ({:.1}/day) (SDM120, {} days)",
                    total_hp_elec,
                    total_hp_elec / matched as f64,
                    matched
                );
                println!(
                    "  Baseload:        {:.0} kWh ({:.1}/day) (house − HP)",
                    baseload,
                    baseload / matched as f64
                );
                println!("  ── Heating (state machine) ──");
                println!(
                    "    Elec input:    {:.0} kWh ({:.1}/day)",
                    htg_elec,
                    htg_elec / matched as f64
                );
                println!(
                    "    Heat delivered:{:.0} kWh ({:.1}/day)",
                    htg_heat,
                    htg_heat / matched as f64
                );
                println!("    Heat/HDD:      {:.1} kWh/HDD", htg_heat / hdd);
                println!("    Heating COP:   {:.2}", htg_cop);
                println!("  ── DHW (state machine) ──");
                println!(
                    "    Elec input:    {:.0} kWh ({:.1}/day)",
                    dhw_elec,
                    dhw_elec / matched as f64
                );
                println!(
                    "    Heat delivered:{:.0} kWh ({:.1}/day)",
                    dhw_heat,
                    dhw_heat / matched as f64
                );
                println!("    DHW COP:       {:.2}", dhw_cop);
            }
        } else {
            println!(
                "  Whole-house elec:{:.0} kWh ({:.1}/day)",
                octopus_elec,
                octopus_elec / n as f64
            );
        }
        println!();
    }

    // Summary comparison
    println!("── Like-for-like: heating only, per degree day ──\n");
    println!("  DHW stripped from both eras for fair comparison.");
    println!(
        "  Gas era: DHW estimated at {:.1} kWh/day (reference.rs).",
        config().gas_era.dhw_kwh_per_day
    );
    println!("  HP era:  DHW measured by state machine (flow rate threshold).");
    println!("  Any reduction in heat/HDD between eras reflects insulation");
    println!("  improvements (work overlapped both eras).");
    println!();

    Ok(())
}

/// Print baseload analysis: whole-house electricity minus heat pump electricity.
///
/// Requires both Octopus data and emoncms HP data loaded into the same time range.
pub fn print_baseload(consumption: &DataFrame, hp_daily_elec: &[(String, f64)]) -> Result<()> {
    let daily = daily_totals(consumption)?;

    println!("\n═══ Baseload Analysis (Whole-House − Heat Pump) ═══\n");
    println!(
        "{:<12} {:>12} {:>12} {:>12}",
        "Date", "House kWh", "HP kWh", "Baseload kWh"
    );
    println!("{}", "─".repeat(52));

    let hp_map: HashMap<&str, f64> = hp_daily_elec
        .iter()
        .map(|(d, v)| (d.as_str(), *v))
        .collect();

    let dates = daily.column("date")?.str()?;
    let elecs = daily.column("elec_kwh")?.f64()?;

    let mut total_house = 0.0f64;
    let mut total_hp = 0.0f64;
    let mut count = 0u32;

    for i in 0..daily.height() {
        let date = dates.get(i).unwrap_or("?");
        let house = elecs.get(i).unwrap_or(0.0);
        if let Some(&hp) = hp_map.get(date) {
            let baseload = house - hp;
            println!(
                "{:<12} {:>12.2} {:>12.2} {:>12.2}",
                date, house, hp, baseload
            );
            total_house += house;
            total_hp += hp;
            count += 1;
        }
    }

    if count > 0 {
        let total_base = total_house - total_hp;
        println!("{}", "─".repeat(52));
        println!(
            "{:<12} {:>12.1} {:>12.1} {:>12.1}",
            "TOTAL", total_house, total_hp, total_base
        );
        println!(
            "{:<12} {:>12.1} {:>12.1} {:>12.1}",
            "Daily avg",
            total_house / count as f64,
            total_hp / count as f64,
            total_base / count as f64,
        );
    }

    Ok(())
}
