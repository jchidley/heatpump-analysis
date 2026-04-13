//! Polars-based analysis of heat pump data.
//!
//! # Vaillant Arotherm Plus 5kW Operating Model
//!
//! The Arotherm 5kW has a **fixed pump speed** of ~860 L/h (14.3 L/min).
//! The flow rate changes when the diverter valve switches between the
//! heating circuit and the DHW cylinder coil (lower flow resistance → higher rate).
//!
//! ## Operating States
//!
//! | State      | Flow Rate       | DT (flow−return) | Heat   | Notes                            |
//! |------------|-----------------|-------------------|--------|----------------------------------|
//! | Idle       | —               | —                 | —      | elec ≤ 50W                       |
//! | Heating    | 14.0–14.5 l/m   | > 0               | > 0    | Fixed pump, weather-compensated  |
//! | DHW        | ≥ 15.0 l/m      | > 0               | > 0    | Diverter to cylinder, peak ~21 |
//! | Defrost    | any             | < 0 or heat ≤ 0   | ≤ 0    | Reverse cycle, extracts from water|
//! | Transition | 14.5–15.0 l/m   | any               | any    | Diverter valve moving            |
//!
//! ## Hysteresis
//!
//! - Enter DHW when flow_rate rises above **15.0** l/m AND heat > 0
//! - Exit DHW when flow_rate drops below **14.7** l/m AND heat > 0
//! - Enter Defrost when heat ≤ 0 or DT < −0.5 (regardless of flow rate)
//! - Exit Defrost when heat > 0 AND DT > 0 (return to previous state)
//!
//! ## Evidence (full dataset, Oct 2024 – Mar 2026, 447k running samples)
//!
//! - Heating: 363k samples at 14.3–14.4 l/m
//! - DHW: 76k samples, main peak at 20.0–21.0 l/m (77% of DHW)
//! - Defrost: 7k samples (1.6%), split between heating and DHW flow rates
//! - Transition: 1.2k samples (0.3%), 67% are DHW→heating ramp-down

use anyhow::{Context, Result};
use chrono::DateTime;
use polars::prelude::*;

use crate::config::{self, config};

/// Operating state of the heat pump at a given moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HpState {
    Idle,
    Heating,
    Dhw,
    Defrost,
}

impl HpState {
    fn as_str(&self) -> &'static str {
        match self {
            HpState::Idle => "idle",
            HpState::Heating => "heating",
            HpState::Dhw => "dhw",
            HpState::Defrost => "defrost",
        }
    }
}

/// Classify each row into an operating state using a hysteresis state machine.
///
/// Processes rows in time order, tracking the previous state to handle
/// the transition zone (14.5–16.0 l/m) and defrost recovery correctly.
fn classify_states(
    elec: &[Option<f64>],
    heat: &[Option<f64>],
    flow_rate: &[Option<f64>],
    delta_t: &[f64],
) -> Vec<&'static str> {
    let cfg = config();
    let thresholds = &cfg.thresholds;

    let n = elec.len();
    let mut states = Vec::with_capacity(n);
    let mut current = HpState::Idle;
    let mut pre_defrost = HpState::Heating;

    for i in 0..n {
        let e = elec[i].unwrap_or(0.0);
        let h = heat[i].unwrap_or(0.0);
        let fr = flow_rate[i].unwrap_or(0.0);
        let dt = delta_t[i];

        let next = if e <= thresholds.elec_running_w {
            HpState::Idle
        } else if h <= 0.0 || dt < thresholds.defrost_dt_threshold {
            if current != HpState::Defrost {
                pre_defrost = match current {
                    HpState::Dhw => HpState::Dhw,
                    _ => HpState::Heating,
                };
            }
            HpState::Defrost
        } else {
            match current {
                HpState::Idle | HpState::Heating => {
                    if fr >= thresholds.dhw_enter_flow_rate {
                        HpState::Dhw
                    } else {
                        HpState::Heating
                    }
                }
                HpState::Dhw => {
                    if fr < thresholds.dhw_exit_flow_rate {
                        HpState::Heating
                    } else {
                        HpState::Dhw
                    }
                }
                HpState::Defrost => {
                    if fr >= thresholds.dhw_enter_flow_rate {
                        HpState::Dhw
                    } else if fr < thresholds.dhw_exit_flow_rate {
                        HpState::Heating
                    } else {
                        pre_defrost
                    }
                }
            }
        };

        current = next;
        states.push(current.as_str());
    }

    states
}

/// Add computed columns: COP, delta_T, and operating state.
pub fn enrich(df: &DataFrame) -> Result<DataFrame> {
    // Compute delta_t eagerly (needed for state machine)
    let flow_t = df.column("flow_t")?.f64()?;
    let return_t = df.column("return_t")?.f64()?;
    let delta_t: Vec<f64> = flow_t
        .into_iter()
        .zip(return_t)
        .map(|(f, r)| match (f, r) {
            (Some(f), Some(r)) => f - r,
            _ => 0.0,
        })
        .collect();

    // Extract arrays for state machine
    let elec: Vec<Option<f64>> = df.column("elec_w")?.f64()?.into_iter().collect();
    let heat: Vec<Option<f64>> = df.column("heat_w")?.f64()?.into_iter().collect();
    let flow_rate: Vec<Option<f64>> = df.column("flow_rate")?.f64()?.into_iter().collect();

    // Run state machine
    let states = classify_states(&elec, &heat, &flow_rate, &delta_t);

    // Add COP and delta_t columns
    let enriched = df
        .clone()
        .lazy()
        .with_columns([
            when(col("elec_w").gt(lit(config().thresholds.elec_running_w)))
                .then(col("heat_w") / col("elec_w"))
                .otherwise(lit(NULL))
                .alias("cop"),
            (col("flow_t") - col("return_t")).alias("delta_t"),
        ])
        .collect()
        .context("Failed to compute COP/DT")?;

    // Add the state column
    let state_series = Series::new("state".into(), states);
    let enriched = enriched
        .hstack(&[state_series.into()])
        .context("Failed to add state column")?;

    Ok(enriched)
}

/// Summary statistics for a time period, broken down by operating state.
pub fn summary(df: &DataFrame) -> Result<()> {
    let stats = df
        .clone()
        .lazy()
        .filter(
            col("state")
                .eq(lit("heating"))
                .or(col("state").eq(lit("dhw"))),
        )
        .select([
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("cop").mean().alias("avg_cop"),
            col("flow_t").mean().alias("avg_flow_t"),
            col("return_t").mean().alias("avg_return_t"),
            col("outside_t").mean().alias("avg_outside_t"),
            len().alias("productive_samples"),
        ])
        .collect()?;

    println!("\n=== Summary (heating + DHW, excluding defrost) ===");
    println!("{}", stats);

    let by_state = df
        .clone()
        .lazy()
        .filter(col("elec_w").gt(lit(config().thresholds.elec_running_w)))
        .group_by([col("state")])
        .agg([
            col("cop").mean().alias("avg_cop"),
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("flow_t").mean().alias("avg_flow_t"),
            col("flow_rate").mean().alias("avg_flow_rate"),
            col("delta_t").mean().alias("avg_dt"),
            len().alias("samples"),
        ])
        .sort(["state"], Default::default())
        .collect()?;

    println!("\n=== Breakdown by Operating State ===");
    println!("{}", by_state);

    let dist = df
        .clone()
        .lazy()
        .group_by([col("state")])
        .agg([len().alias("samples")])
        .sort(["state"], Default::default())
        .collect()?;

    println!("\n=== State Distribution (all samples) ===");
    println!("{}", dist);

    Ok(())
}

/// COP by outside temperature bands, for heating only.
pub fn cop_by_outside_temp(df: &DataFrame) -> Result<()> {
    let result = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("heating")))
        .with_column(
            ((col("outside_t") / lit(2.0)).floor().cast(DataType::Int32) * lit(2))
                .alias("temp_band"),
        )
        .group_by([col("temp_band")])
        .agg([
            col("cop").mean().alias("avg_cop"),
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            len().alias("samples"),
        ])
        .sort(["temp_band"], Default::default())
        .collect()?;

    println!("\n=== COP by Outside Temperature Band — Heating Only ===");
    println!("{}", result);

    Ok(())
}

/// Hourly profile: average COP, power, temps by hour of day.
pub fn hourly_profile(df: &DataFrame) -> Result<()> {
    let result = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("heating")))
        .with_column(col("timestamp").dt().hour().alias("hour"))
        .group_by([col("hour")])
        .agg([
            col("cop").mean().alias("avg_cop"),
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("flow_t").mean().alias("avg_flow_t"),
            len().alias("samples"),
        ])
        .sort(["hour"], Default::default())
        .collect()?;

    println!("\n=== Hourly Profile — Heating Only ===");
    println!("{}", result);

    Ok(())
}

/// Daily totals from cumulative energy data.
pub fn daily_energy(
    elec_data: &[(i64, Option<f64>)],
    heat_data: &[(i64, Option<f64>)],
) -> Result<()> {
    let n = elec_data.len().min(heat_data.len());
    if n < 2 {
        println!("Not enough daily data points");
        return Ok(());
    }

    let mut dates: Vec<String> = Vec::new();
    let mut elec_kwh: Vec<Option<f64>> = Vec::new();
    let mut heat_kwh: Vec<Option<f64>> = Vec::new();
    let mut daily_cop: Vec<Option<f64>> = Vec::new();

    for i in 1..n {
        let ts = elec_data[i].0 / 1000;
        let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
        dates.push(dt.format("%Y-%m-%d").to_string());

        let de = match (elec_data[i].1, elec_data[i - 1].1) {
            (Some(a), Some(b)) if a >= b => Some(a - b),
            _ => None,
        };
        let dh = match (heat_data[i].1, heat_data[i - 1].1) {
            (Some(a), Some(b)) if a >= b => Some(a - b),
            _ => None,
        };

        let cop = match (de, dh) {
            (Some(e), Some(h)) if e > 0.1 => Some(h / e),
            _ => None,
        };

        elec_kwh.push(de);
        heat_kwh.push(dh);
        daily_cop.push(cop);
    }

    let df = DataFrame::new(vec![
        Series::new("date".into(), &dates).into(),
        Series::new("elec_kwh".into(), &elec_kwh).into(),
        Series::new("heat_kwh".into(), &heat_kwh).into(),
        Series::new("daily_cop".into(), &daily_cop).into(),
    ])?;

    println!("\n=== Daily Energy & COP ===");
    println!("{}", df);

    Ok(())
}

/// Degree day analysis combining outside temperature with energy consumption.
///
/// Uses UK standard base temperature of 15.5°C.
/// HDD = max(0, base_temp − mean_daily_outside_temp)
pub fn degree_days(
    daily_temps: &[(String, f64, f64, f64)], // (date, mean, min, max)
    elec_data: &[(i64, Option<f64>)],
    heat_data: &[(i64, Option<f64>)],
) -> Result<()> {
    // Build energy lookup by date
    let mut elec_by_date: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut heat_by_date: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for i in 1..elec_data.len().min(heat_data.len()) {
        let ts = elec_data[i].0 / 1000;
        let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
        let date = dt.format("%Y-%m-%d").to_string();

        if let (Some(a), Some(b)) = (elec_data[i].1, elec_data[i - 1].1) {
            if a >= b {
                elec_by_date.insert(date.clone(), a - b);
            }
        }
        if let (Some(a), Some(b)) = (heat_data[i].1, heat_data[i - 1].1) {
            if a >= b {
                heat_by_date.insert(date, a - b);
            }
        }
    }

    // Build daily degree day table
    let mut dates: Vec<String> = Vec::new();
    let mut hdd_vals: Vec<f64> = Vec::new();
    let mut mean_temps: Vec<f64> = Vec::new();
    let mut elec_kwh_vals: Vec<Option<f64>> = Vec::new();
    let mut heat_kwh_vals: Vec<Option<f64>> = Vec::new();
    let mut kwh_per_hdd: Vec<Option<f64>> = Vec::new();
    let mut heat_per_hdd: Vec<Option<f64>> = Vec::new();
    let mut cop_vals: Vec<Option<f64>> = Vec::new();

    for (date, mean_t, _min_t, _max_t) in daily_temps {
        let hdd = (config().thresholds.hdd_base_temp_c - mean_t).max(0.0);
        let elec = elec_by_date.get(date).copied();
        let heat = heat_by_date.get(date).copied();

        let cop = match (elec, heat) {
            (Some(e), Some(h)) if e > 0.1 => Some(h / e),
            _ => None,
        };

        let e_per_hdd = match elec {
            Some(e) if hdd > 0.1 => Some(e / hdd),
            _ => None,
        };
        let h_per_hdd = match heat {
            Some(h) if hdd > 0.1 => Some(h / hdd),
            _ => None,
        };

        dates.push(date.clone());
        hdd_vals.push(hdd);
        mean_temps.push(*mean_t);
        elec_kwh_vals.push(elec);
        heat_kwh_vals.push(heat);
        kwh_per_hdd.push(e_per_hdd);
        heat_per_hdd.push(h_per_hdd);
        cop_vals.push(cop);
    }

    let df = DataFrame::new(vec![
        Series::new("date".into(), &dates).into(),
        Series::new("mean_°C".into(), &mean_temps).into(),
        Series::new("HDD".into(), &hdd_vals).into(),
        Series::new("elec_kWh".into(), &elec_kwh_vals).into(),
        Series::new("heat_kWh".into(), &heat_kwh_vals).into(),
        Series::new("COP".into(), &cop_vals).into(),
        Series::new("elec/HDD".into(), &kwh_per_hdd).into(),
        Series::new("heat/HDD".into(), &heat_per_hdd).into(),
    ])?;

    println!(
        "\n=== Daily Degree Days (base {:.1}°C) ===",
        config().thresholds.hdd_base_temp_c
    );
    println!("{}", df);

    // Weekly aggregates
    let n = dates.len();
    if n >= 7 {
        let mut week_labels: Vec<String> = Vec::new();
        let mut week_hdd: Vec<f64> = Vec::new();
        let mut week_elec: Vec<Option<f64>> = Vec::new();
        let mut week_heat: Vec<Option<f64>> = Vec::new();
        let mut week_cop: Vec<Option<f64>> = Vec::new();
        let mut week_elec_per_hdd: Vec<Option<f64>> = Vec::new();
        let mut week_heat_per_hdd: Vec<Option<f64>> = Vec::new();

        let chunks = n / 7;
        for c in 0..chunks {
            let start_idx = c * 7;
            let end_idx = start_idx + 7;

            let label = format!("{} → {}", &dates[start_idx], &dates[end_idx - 1]);
            let sum_hdd: f64 = hdd_vals[start_idx..end_idx].iter().sum();
            let sum_elec: f64 = elec_kwh_vals[start_idx..end_idx]
                .iter()
                .filter_map(|v| *v)
                .sum();
            let sum_heat: f64 = heat_kwh_vals[start_idx..end_idx]
                .iter()
                .filter_map(|v| *v)
                .sum();

            let has_elec = elec_kwh_vals[start_idx..end_idx]
                .iter()
                .any(|v| v.is_some());
            let has_heat = heat_kwh_vals[start_idx..end_idx]
                .iter()
                .any(|v| v.is_some());

            week_labels.push(label);
            week_hdd.push(sum_hdd);
            week_elec.push(if has_elec { Some(sum_elec) } else { None });
            week_heat.push(if has_heat { Some(sum_heat) } else { None });
            week_cop.push(if has_elec && sum_elec > 0.1 {
                Some(sum_heat / sum_elec)
            } else {
                None
            });
            week_elec_per_hdd.push(if has_elec && sum_hdd > 0.5 {
                Some(sum_elec / sum_hdd)
            } else {
                None
            });
            week_heat_per_hdd.push(if has_heat && sum_hdd > 0.5 {
                Some(sum_heat / sum_hdd)
            } else {
                None
            });
        }

        let wdf = DataFrame::new(vec![
            Series::new("week".into(), &week_labels).into(),
            Series::new("HDD".into(), &week_hdd).into(),
            Series::new("elec_kWh".into(), &week_elec).into(),
            Series::new("heat_kWh".into(), &week_heat).into(),
            Series::new("COP".into(), &week_cop).into(),
            Series::new("elec/HDD".into(), &week_elec_per_hdd).into(),
            Series::new("heat/HDD".into(), &week_heat_per_hdd).into(),
        ])?;

        println!("\n=== Weekly Degree Day Summary ===");
        println!("{}", wdf);
    }

    // Period totals
    let total_hdd: f64 = hdd_vals.iter().sum();
    let total_elec: f64 = elec_kwh_vals.iter().filter_map(|v| *v).sum();
    let total_heat: f64 = heat_kwh_vals.iter().filter_map(|v| *v).sum();
    let zero_hdd_days = hdd_vals.iter().filter(|h| **h < 0.01).count();

    println!("\n=== Period Summary ===");
    println!("Days:              {}", dates.len());
    println!("Total HDD:         {:.1}", total_hdd);
    println!("Zero-HDD days:     {} (no heating needed)", zero_hdd_days);
    println!("Total elec:        {:.1} kWh", total_elec);
    println!("Total heat:        {:.1} kWh", total_heat);
    if total_elec > 0.1 {
        println!("Period COP:        {:.2}", total_heat / total_elec);
    }
    if total_hdd > 0.5 {
        println!("Elec per HDD:      {:.2} kWh/HDD", total_elec / total_hdd);
        println!("Heat per HDD:      {:.2} kWh/HDD", total_heat / total_hdd);
    }

    // Estimate base temperature from data: find the outside temp above which
    // daily electricity consumption drops to DHW-only levels
    let mut temp_elec: Vec<(f64, f64)> = mean_temps
        .iter()
        .zip(elec_kwh_vals.iter())
        .filter_map(|(t, e)| e.map(|e| (*t, e)))
        .collect();
    temp_elec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    if temp_elec.len() >= 10 {
        // Find the "elbow" — the temperature above which consumption plateaus
        // Use the warmest 20% of days as the baseline (DHW-only)
        let warm_count = (temp_elec.len() as f64 * 0.2).max(5.0) as usize;
        let warm_start = temp_elec.len() - warm_count;
        let baseline_elec: f64 =
            temp_elec[warm_start..].iter().map(|(_, e)| e).sum::<f64>() / warm_count as f64;

        // Find the coldest temperature where consumption is within 20% of baseline
        let threshold = baseline_elec * 1.2;
        let mut estimated_base = config().thresholds.hdd_base_temp_c;
        for (t, e) in &temp_elec {
            if *e <= threshold {
                estimated_base = *t;
                break;
            }
        }

        println!(
            "\n[ESTIMATED] Base temperature: ~{:.0}°C (consumption plateaus at {:.1} kWh/day above this)",
            estimated_base, baseline_elec,
        );
    }

    // Monthly aggregation for comparison with gas-era data
    println!("\n=== Monthly Degree Day Summary ===");
    println!(
        "{:>8} {:>7} {:>10} {:>10} {:>7} {:>10} {:>10}",
        "Month", "HDD", "Elec kWh", "Heat kWh", "COP", "Elec/HDD", "Heat/HDD"
    );
    println!("{}", "-".repeat(70));

    let mut month_data: std::collections::BTreeMap<
        String,
        (f64, f64, f64), // (hdd, elec, heat)
    > = std::collections::BTreeMap::new();

    for (date, mean_t, _, _) in daily_temps {
        let month = &date[..7]; // "YYYY-MM"
        let hdd = (config().thresholds.hdd_base_temp_c - mean_t).max(0.0);
        let elec = elec_by_date.get(date.as_str()).copied().unwrap_or(0.0);
        let heat = heat_by_date.get(date.as_str()).copied().unwrap_or(0.0);

        let entry = month_data.entry(month.to_string()).or_default();
        entry.0 += hdd;
        entry.1 += elec;
        entry.2 += heat;
    }

    for (month, (hdd, elec, heat)) in &month_data {
        let cop = if *elec > 0.1 { heat / elec } else { 0.0 };
        let e_per_hdd = if *hdd > 0.5 {
            format!("{:.2}", elec / hdd)
        } else {
            "-".to_string()
        };
        let h_per_hdd = if *hdd > 0.5 {
            format!("{:.1}", heat / hdd)
        } else {
            "-".to_string()
        };
        println!(
            "{:>8} {:>7.1} {:>10.1} {:>10.0} {:>7.2} {:>10} {:>10}",
            month, hdd, elec, heat, cop, e_per_hdd, h_per_hdd
        );
    }

    // Compare with gas-era monthly data
    println!("\n=== Month-on-Month: Gas Era vs Heat Pump ===");
    println!(
        "{:>8} {:>8} {:>10} {:>10} {:>8} {:>10} {:>10}",
        "Month", "Gas HDD", "Gas kWh", "Gas Heat*", "HP HDD", "HP Elec", "HP Heat"
    );
    println!("{}", "-".repeat(72));

    let gas_era = &config().gas_era;
    for gm in &gas_era.monthly {
        let _month_key = gm.month.to_string();
        // Find matching HP month (any year)
        let mm = &gm.month[5..7]; // extract "01", "02", etc.
        let hp_matches: Vec<_> = month_data.iter().filter(|(k, _)| &k[5..7] == mm).collect();

        let gas_heating = (gm.gas_kwh - gm.hot_water_kwh) * gas_era.boiler_efficiency;

        if let Some((_hp_month, (hp_hdd, hp_elec, hp_heat))) = hp_matches.first() {
            println!(
                "{:>8} {:>8.0} {:>10.0} {:>10.0} {:>8.1} {:>10.1} {:>10.0}",
                mm, gm.hdd_17c, gm.gas_kwh, gas_heating, hp_hdd, hp_elec, hp_heat
            );
        } else {
            println!(
                "{:>8} {:>8.0} {:>10.0} {:>10.0} {:>8} {:>10} {:>10}",
                mm, gm.hdd_17c, gm.gas_kwh, gas_heating, "-", "-", "-"
            );
        }
    }
    println!(
        "  * Gas Heat = (Gas kWh - Hot Water) × {:.0}% boiler efficiency",
        gas_era.boiler_efficiency * 100.0
    );

    Ok(())
}

/// Indoor temperature analysis (Leather room — emonth2 sensor).
pub fn indoor_temp(df: &DataFrame) -> Result<()> {
    let design_temp = config().house.design_indoor_temp_c;

    // Overall indoor temp stats
    let stats = df
        .clone()
        .lazy()
        .filter(col("indoor_t").is_not_null())
        .select([
            col("indoor_t").mean().alias("mean"),
            col("indoor_t").min().alias("min"),
            col("indoor_t").max().alias("max"),
            col("indoor_t").std(1).alias("std_dev"),
            len().alias("samples"),
        ])
        .collect()?;

    println!("\n=== Indoor Temperature — Leather Room (emonth2) ===");
    println!("Design target: {:.1}°C", design_temp);
    println!("{}", stats);

    // Indoor temp by hour of day
    let hourly = df
        .clone()
        .lazy()
        .filter(col("indoor_t").is_not_null())
        .with_column(col("timestamp").dt().hour().alias("hour"))
        .group_by([col("hour")])
        .agg([
            col("indoor_t").mean().alias("mean_indoor"),
            col("outside_t").mean().alias("mean_outside"),
        ])
        .sort(["hour"], Default::default())
        .collect()?;

    println!("\n=== Indoor/Outdoor Temperature by Hour ===");
    println!("{}", hourly);

    // Indoor temp vs outside temp — comfort correlation
    let comfort = df
        .clone()
        .lazy()
        .filter(
            col("indoor_t")
                .is_not_null()
                .and(col("state").eq(lit("heating"))),
        )
        .with_column(
            ((col("outside_t") / lit(2.0)).floor().cast(DataType::Int32) * lit(2))
                .alias("outside_band"),
        )
        .group_by([col("outside_band")])
        .agg([
            col("indoor_t").mean().alias("mean_indoor"),
            col("indoor_t").min().alias("min_indoor"),
            len().alias("samples"),
        ])
        .sort(["outside_band"], Default::default())
        .collect()?;

    println!("\n=== Indoor Temp vs Outside Temp (during heating) ===");
    println!("{}", comfort);

    Ok(())
}

/// DHW analysis — compare actual against design expectations.
pub fn dhw_analysis(df: &DataFrame) -> Result<()> {
    let design_dhw_kwh_per_day = config().gas_era.dhw_kwh_per_day;

    // Filter to DHW state only
    let dhw_stats = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("dhw")))
        .select([
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("cop").mean().alias("avg_cop"),
            col("flow_t").mean().alias("avg_flow_t"),
            col("flow_rate").mean().alias("avg_flow_rate"),
            col("delta_t").mean().alias("avg_dt"),
            len().alias("total_minutes"),
        ])
        .collect()?;

    println!("\n=== DHW Analysis ===");
    println!(
        "Design hot water: {:.1} kWh/day (from workbook, gas era)",
        design_dhw_kwh_per_day
    );
    println!("{}", dhw_stats);

    // Estimate daily DHW energy from total minutes and avg power
    let total_mins = dhw_stats
        .column("total_minutes")?
        .u32()?
        .get(0)
        .unwrap_or(0) as f64;
    let avg_heat = dhw_stats.column("avg_heat_w")?.f64()?.get(0).unwrap_or(0.0);

    // Count distinct days with DHW
    let dhw_days = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("dhw")))
        .with_column(col("timestamp").dt().date().alias("date"))
        .select([col("date").n_unique().alias("days")])
        .collect()?;

    let n_days = dhw_days.column("days")?.u32()?.get(0).unwrap_or(1) as f64;
    let total_dhw_kwh = avg_heat * total_mins / 60.0 / 1000.0;
    let dhw_kwh_per_day = total_dhw_kwh / n_days;

    println!("\nDHW days in period:     {:.0}", n_days);
    println!("Total DHW heat:         {:.0} kWh", total_dhw_kwh);
    println!("Actual DHW per day:     {:.1} kWh/day", dhw_kwh_per_day);
    println!(
        "Design DHW per day:     {:.1} kWh/day (gas era estimate)",
        design_dhw_kwh_per_day
    );
    println!(
        "Ratio actual/design:    {:.0}%",
        dhw_kwh_per_day / design_dhw_kwh_per_day * 100.0
    );

    Ok(())
}

/// Compare actual COP against Arotherm manufacturer spec at different flow temps.
pub fn cop_vs_spec(df: &DataFrame) -> Result<()> {
    let arotherm = &config().arotherm;

    // Print the manufacturer reference curve
    println!("\n=== Arotherm 5kW Manufacturer Spec (at -3°C outside) ===");
    println!("{:>10} {:>12} {:>8}", "Flow °C", "Heat Output", "COP");
    println!("{}", "-".repeat(32));
    for sp in &arotherm.spec_at_minus3 {
        println!(
            "{:>10.0} {:>10.0}W {:>8.2}",
            sp.flow_temp_c, sp.heat_output_w, sp.cop
        );
    }

    // Group actual data by 5°C flow temp bands, heating only
    let result = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("heating")))
        .with_column(
            ((col("flow_t") / lit(5.0)).floor().cast(DataType::Int32) * lit(5)).alias("flow_band"),
        )
        .group_by([col("flow_band")])
        .agg([
            col("cop").mean().alias("actual_cop"),
            col("elec_w").mean().alias("avg_elec"),
            col("heat_w").mean().alias("avg_heat"),
            col("outside_t").mean().alias("avg_outside_t"),
            len().alias("samples"),
        ])
        .sort(["flow_band"], Default::default())
        .collect()?;

    println!("\n=== Actual COP by Flow Temperature Band (Heating Only) ===");
    println!("{}", result);

    // Compare at each spec flow temp
    println!("\n=== Actual vs Manufacturer COP ===");
    println!(
        "{:>10} {:>10} {:>10} {:>10} {:>10}",
        "Flow °C", "Spec COP", "Actual COP", "Ratio", "Samples"
    );
    println!("{}", "-".repeat(55));

    let heating = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("heating")))
        .collect()?;

    let flow_t_arr = heating.column("flow_t")?.f64()?;
    let cop_arr = heating.column("cop")?.f64()?;

    for sp in &arotherm.spec_at_minus3 {
        // Find samples within ±2.5°C of this flow temp
        let mut cops: Vec<f64> = Vec::new();
        for i in 0..flow_t_arr.len() {
            if let (Some(ft), Some(cop)) = (flow_t_arr.get(i), cop_arr.get(i)) {
                if (ft - sp.flow_temp_c).abs() < 2.5 && cop > 0.0 {
                    cops.push(cop);
                }
            }
        }
        if !cops.is_empty() {
            let avg_cop = cops.iter().sum::<f64>() / cops.len() as f64;
            let ratio = avg_cop / sp.cop;
            println!(
                "{:>10.0} {:>10.2} {:>10.2} {:>9.0}% {:>10}",
                sp.flow_temp_c,
                sp.cop,
                avg_cop,
                ratio * 100.0,
                cops.len()
            );
        } else {
            println!(
                "{:>10.0} {:>10.2} {:>10} {:>10} {:>10}",
                sp.flow_temp_c, sp.cop, "-", "-", "0"
            );
        }
    }

    Ok(())
}

/// Compare actual heat demand against design calculations and gas-era data.
pub fn design_comparison(
    daily_temps: &[(String, f64, f64, f64)],
    elec_data: &[(i64, Option<f64>)],
    heat_data: &[(i64, Option<f64>)],
) -> Result<()> {
    let cfg = config();
    let house = &cfg.house;
    let gas_era = &cfg.gas_era;
    let radiators = &cfg.radiators;

    println!("\n=== House Design Reference ===");
    println!("Construction:     {}", house.construction);
    println!("Floor area:       {} m²", house.floor_area_m2);
    println!("HTC:              {} W/°C", house.htc_w_per_c);
    println!(
        "Design heat loss: {} W (at {}°C outside, {}°C inside)",
        house.design_heat_loss_w, house.design_outdoor_temp_c, house.design_indoor_temp_c
    );

    // Radiator capacity at different flow temps
    println!("\n=== Radiator Output vs Flow Temperature ===");
    println!(
        "{:>10} {:>15} {:>15}",
        "Flow °C", "Total Output W", "vs Design Loss"
    );
    println!("{}", "-".repeat(45));
    for ft in &[30.0, 32.0, 35.0, 38.0, 40.0, 45.0, 50.0] {
        let output = config::total_radiator_output_at_flow_temp(radiators, *ft);
        let ratio = output / house.design_heat_loss_w * 100.0;
        println!("{:>10.0} {:>13.0}W {:>13.0}%", ft, output, ratio);
    }
    println!(
        "\nTotal T50 rating:  {} W ({} radiators)",
        radiators.iter().map(|r| r.t50_watts as u32).sum::<u32>(),
        radiators.len()
    );

    // Gas era comparison
    println!("\n=== Gas Era vs Heat Pump Comparison ===");

    // Calculate HP annual energy from the data we have
    let mut hp_elec_total = 0.0f64;
    let mut hp_heat_total = 0.0f64;
    let mut hp_days = 0u32;

    let mut elec_by_date: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut heat_by_date: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for i in 1..elec_data.len().min(heat_data.len()) {
        let ts = elec_data[i].0 / 1000;
        let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
        let date = dt.format("%Y-%m-%d").to_string();

        if let (Some(a), Some(b)) = (elec_data[i].1, elec_data[i - 1].1) {
            if a >= b {
                let de = a - b;
                elec_by_date.insert(date.clone(), de);
                hp_elec_total += de;
            }
        }
        if let (Some(a), Some(b)) = (heat_data[i].1, heat_data[i - 1].1) {
            if a >= b {
                let dh = a - b;
                heat_by_date.insert(date.clone(), dh);
                hp_heat_total += dh;
                hp_days += 1;
            }
        }
    }

    // Calculate HDD for the HP period (base 17°C for comparison with gas)
    let mut hp_hdd_17 = 0.0f64;
    for (_date, mean_t, _, _) in daily_temps {
        let hdd = (house.base_temp_gas_era_c - mean_t).max(0.0);
        hp_hdd_17 += hdd;
    }

    let hp_cop = if hp_elec_total > 0.1 {
        hp_heat_total / hp_elec_total
    } else {
        0.0
    };

    // Estimate what gas would have cost for the same period
    let gas_equiv_heating_kwh = hp_hdd_17 * house.kwh_per_hdd;
    let gas_equiv_total_kwh = gas_equiv_heating_kwh / gas_era.boiler_efficiency;

    println!("{:<35} {:>15} {:>15}", "", "Gas Boiler", "Heat Pump");
    println!("{}", "-".repeat(67));
    println!(
        "{:<35} {:>15} {:>15}",
        "Period",
        "Annual (est)",
        format!("{} days", hp_days)
    );
    println!(
        "{:<35} {:>14.0} {:>14.0}",
        "HDD (base 17°C)",
        gas_era.monthly.iter().map(|m| m.hdd_17c).sum::<f64>(),
        hp_hdd_17
    );
    println!(
        "{:<35} {:>13.0}* {:>14.0}",
        "Heat delivered (kWh)", gas_era.annual_heating_delivered_kwh, hp_heat_total
    );
    println!(
        "{:<35} {:>14.0} {:>14.0}",
        "Energy consumed (kWh)", gas_era.annual_gas_kwh, hp_elec_total
    );
    println!(
        "{:<35} {:>14.0}% {:>13.1}x",
        "Efficiency / COP",
        gas_era.boiler_efficiency * 100.0,
        hp_cop
    );
    println!(
        "{:<35} {:>14.1} {:>14.1}",
        "kWh consumed per HDD",
        gas_era.annual_heating_gas_kwh / gas_era.monthly.iter().map(|m| m.hdd_17c).sum::<f64>(),
        hp_elec_total / hp_hdd_17
    );
    println!("\n  * Gas heating estimated: annual {:.0} kWh gas × {:.0}% efficiency = {:.0} kWh delivered",
        gas_era.annual_gas_kwh, gas_era.boiler_efficiency * 100.0, gas_era.annual_heating_delivered_kwh);
    println!(
        "  * Gas figure includes hot water ({:.1} kWh/day), HP figure includes DHW too",
        gas_era.dhw_kwh_per_day
    );

    // For same HDD, how much gas vs electricity would be used?
    if hp_hdd_17 > 10.0 {
        println!("\n=== Same-Weather Comparison (HDD-normalised) ===");
        println!("If this HP period's weather had been heated by gas boiler:");
        println!(
            "  Gas consumed:    {:.0} kWh (at {:.1} kWh/HDD × {:.0} HDD)",
            gas_equiv_total_kwh,
            house.kwh_per_hdd / gas_era.boiler_efficiency,
            hp_hdd_17
        );
        println!("  HP consumed:     {:.0} kWh electricity", hp_elec_total);
        println!(
            "  Energy saving:   {:.0} kWh ({:.0}%)",
            gas_equiv_total_kwh - hp_elec_total,
            (1.0 - hp_elec_total / gas_equiv_total_kwh) * 100.0
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use polars::df;
    use std::path::Path;
    use std::sync::Once;

    static INIT_CONFIG: Once = Once::new();

    fn ensure_config_loaded() {
        INIT_CONFIG.call_once(|| {
            crate::config::load(Path::new("config.toml"))
                .expect("config.toml should load for tests");
        });
    }

    // @lat: [[tests#CLI state classification#DHW classification holds through the transition band]]
    #[test]
    fn dhw_classification_holds_through_transition_band() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let transition = (thresholds.dhw_enter_flow_rate + thresholds.dhw_exit_flow_rate) / 2.0;

        let states = classify_states(
            &[Some(thresholds.elec_running_w + 100.0); 4],
            &[Some(2000.0); 4],
            &[
                Some(thresholds.dhw_enter_flow_rate + 0.1),
                Some(transition),
                Some(thresholds.dhw_exit_flow_rate + 0.01),
                Some(thresholds.dhw_exit_flow_rate - 0.01),
            ],
            &[1.0; 4],
        );

        assert_eq!(states, vec!["dhw", "dhw", "dhw", "heating"]);
    }

    // @lat: [[tests#CLI state classification#Defrost recovery preserves the pre-defrost circuit state]]
    #[test]
    fn defrost_recovery_preserves_pre_defrost_state() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let transition = (thresholds.dhw_enter_flow_rate + thresholds.dhw_exit_flow_rate) / 2.0;

        let states = classify_states(
            &[Some(thresholds.elec_running_w + 100.0); 3],
            &[Some(2000.0), Some(-200.0), Some(2000.0)],
            &[
                Some(thresholds.dhw_enter_flow_rate + 0.2),
                Some(transition),
                Some(transition),
            ],
            &[1.0, thresholds.defrost_dt_threshold - 0.1, 1.0],
        );

        assert_eq!(states, vec!["dhw", "defrost", "dhw"]);
    }

    // @lat: [[tests#CLI state classification#Idle precedence wins over defrost-like noise]]
    #[test]
    fn idle_precedence_wins_over_defrost_like_noise() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;

        let states = classify_states(
            &[
                Some(thresholds.elec_running_w - 1.0),
                Some(thresholds.elec_running_w + 100.0),
            ],
            &[Some(-200.0), Some(2000.0)],
            &[
                Some(thresholds.dhw_enter_flow_rate + 0.2),
                Some(thresholds.dhw_exit_flow_rate - 0.1),
            ],
            &[thresholds.defrost_dt_threshold - 0.1, 1.0],
        );

        assert_eq!(states, vec!["idle", "heating"]);
    }

    // @lat: [[tests#CLI state classification#Heating to DHW to heating cycle transitions cleanly]]
    #[test]
    fn heating_dhw_heating_cycle() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let e = Some(thresholds.elec_running_w + 100.0);

        let states = classify_states(
            &[e; 5],
            &[Some(2000.0); 5],
            &[
                Some(thresholds.dhw_enter_flow_rate - 0.5), // heating
                Some(thresholds.dhw_enter_flow_rate + 0.5), // enter DHW
                Some(thresholds.dhw_enter_flow_rate + 1.0), // stay DHW
                Some(thresholds.dhw_exit_flow_rate - 0.1),  // exit DHW → heating
                Some(thresholds.dhw_exit_flow_rate - 0.5),  // stay heating
            ],
            &[1.0; 5],
        );

        assert_eq!(states, vec!["heating", "dhw", "dhw", "heating", "heating"]);
    }

    // @lat: [[tests#CLI state classification#Defrost entry from heating preserves heating as pre-defrost]]
    #[test]
    fn defrost_from_heating_recovers_to_heating() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let e = Some(thresholds.elec_running_w + 100.0);
        let transition = (thresholds.dhw_enter_flow_rate + thresholds.dhw_exit_flow_rate) / 2.0;

        let states = classify_states(
            &[e; 3],
            &[Some(2000.0), Some(-200.0), Some(2000.0)],
            &[
                Some(14.0),       // heating
                Some(14.0),       // defrost (heat <= 0)
                Some(transition), // recovery → pre_defrost=heating
            ],
            &[1.0, -1.0, 1.0],
        );

        assert_eq!(states, vec!["heating", "defrost", "heating"]);
    }

    // @lat: [[tests#CLI state classification#Defrost DT boundary is exclusive]]
    #[test]
    fn defrost_dt_boundary_is_exclusive() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let e = Some(thresholds.elec_running_w + 100.0);

        // At exactly the threshold, should NOT enter defrost (< is strict)
        let at_boundary = classify_states(
            &[e],
            &[Some(2000.0)],
            &[Some(14.0)],
            &[thresholds.defrost_dt_threshold],
        );
        assert_eq!(
            at_boundary,
            vec!["heating"],
            "exact threshold should be heating, not defrost"
        );

        // Just below threshold → defrost
        let below = classify_states(
            &[e],
            &[Some(2000.0)],
            &[Some(14.0)],
            &[thresholds.defrost_dt_threshold - 0.001],
        );
        assert_eq!(below, vec!["defrost"]);
    }

    // @lat: [[tests#CLI state classification#Missing flow rate defaults to heating not DHW]]
    #[test]
    fn none_flow_rate_defaults_to_heating() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;
        let e = Some(thresholds.elec_running_w + 100.0);

        let states = classify_states(
            &[e; 2],
            &[Some(2000.0); 2],
            &[None, None], // flow_rate missing → 0.0
            &[1.0; 2],
        );

        assert_eq!(states, vec!["heating", "heating"]);
    }

    // @lat: [[tests#CLI state classification#Enrich derives state delta-T and running-only COP]]
    #[test]
    fn enrich_derives_state_delta_t_and_running_only_cop() {
        ensure_config_loaded();
        let thresholds = &config().thresholds;

        let running_elec = thresholds.elec_running_w + 100.0;
        let running_heat = 2400.0;
        let df = df!(
            "elec_w" => &[thresholds.elec_running_w - 1.0, running_elec],
            "heat_w" => &[0.0, running_heat],
            "flow_rate" => &[14.0, 14.0],
            "flow_t" => &[35.0, 40.0],
            "return_t" => &[30.0, 32.0],
            "outside_t" => &[8.0, 8.0],
        )
        .expect("test dataframe");

        let enriched = enrich(&df).expect("enrich should succeed");

        let states: Vec<Option<&str>> = enriched.column("state").unwrap().str().unwrap().into_iter().collect();
        assert_eq!(states, vec![Some("idle"), Some("heating")]);

        let delta_t: Vec<Option<f64>> = enriched.column("delta_t").unwrap().f64().unwrap().into_iter().collect();
        assert_eq!(delta_t, vec![Some(5.0), Some(8.0)]);

        let cop: Vec<Option<f64>> = enriched.column("cop").unwrap().f64().unwrap().into_iter().collect();
        assert_eq!(cop[0], None);
        assert_eq!(cop[1], Some(running_heat / running_elec));
    }
}
