//! Build Polars DataFrames from emoncms data and run analyses.
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
//! | DHW        | ≥ 16.0 l/m      | > 0               | > 0    | Diverter to cylinder, peak ~20.7 |
//! | Defrost    | any             | < 0 or heat ≤ 0   | ≤ 0    | Reverse cycle, extracts from water|
//! | Transition | 14.5–16.0 l/m   | any               | any    | Diverter valve moving            |
//!
//! ## Hysteresis
//!
//! - Enter DHW when flow_rate rises above **16.0** l/m AND heat > 0
//! - Exit DHW when flow_rate drops below **15.0** l/m AND heat > 0
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

use crate::emoncms::Client;

/// Known feed IDs for this heat pump installation.
pub struct FeedIds {
    pub outside_temp: &'static str,
    pub electric_power: &'static str,
    pub electric_energy: &'static str,
    pub heat_power: &'static str,
    pub heat_energy: &'static str,
    pub flow_temp: &'static str,
    pub return_temp: &'static str,
    pub flow_rate: &'static str,
    pub indoor_temp: &'static str,
}

pub const FEEDS: FeedIds = FeedIds {
    outside_temp: "503093",
    electric_power: "503094",
    electric_energy: "503095",
    heat_power: "503096",
    heat_energy: "503097",
    flow_temp: "503098",
    return_temp: "503099",
    flow_rate: "503100",
    indoor_temp: "503101",
};

// --- Arotherm 5kW operating thresholds ---

/// Minimum electrical power to consider the compressor running.
const ELEC_RUNNING_W: f64 = 50.0;

/// Flow rate above which we enter DHW state (diverter valve to cylinder).
/// The gap between heating (14.3–14.5) and steady DHW (16.5+) is near-empty.
const DHW_ENTER_FLOW_RATE: f64 = 16.0;

/// Flow rate below which we exit DHW state back to heating.
/// Provides hysteresis across the 14.5–16.0 transition zone (diverter moving).
const DHW_EXIT_FLOW_RATE: f64 = 15.0;

/// DT threshold below which we consider the system to be in defrost.
/// During defrost the return is warmer than the flow (heat extracted from water).
const DEFROST_DT_THRESHOLD: f64 = -0.5;

/// Operating state of the heat pump at a given moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpState {
    Idle,
    Heating,
    Dhw,
    Defrost,
}

impl HpState {
    pub fn as_str(&self) -> &'static str {
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
    let n = elec.len();
    let mut states = Vec::with_capacity(n);
    let mut current = HpState::Idle;
    // Track which productive state to return to after defrost ends
    let mut pre_defrost = HpState::Heating;

    for i in 0..n {
        let e = elec[i].unwrap_or(0.0);
        let h = heat[i].unwrap_or(0.0);
        let fr = flow_rate[i].unwrap_or(0.0);
        let dt = delta_t[i];

        let next = if e <= ELEC_RUNNING_W {
            // Compressor not running
            HpState::Idle
        } else if h <= 0.0 || dt < DEFROST_DT_THRESHOLD {
            // Defrost: heat being extracted from water to melt outdoor coil ice.
            // Can happen at ANY flow rate (depends on diverter valve position).
            if current != HpState::Defrost {
                pre_defrost = match current {
                    HpState::Dhw => HpState::Dhw,
                    _ => HpState::Heating,
                };
            }
            HpState::Defrost
        } else {
            // Productive operation — use flow rate + hysteresis
            match current {
                HpState::Idle | HpState::Heating => {
                    if fr >= DHW_ENTER_FLOW_RATE {
                        HpState::Dhw
                    } else {
                        HpState::Heating
                    }
                }
                HpState::Dhw => {
                    if fr < DHW_EXIT_FLOW_RATE {
                        HpState::Heating
                    } else {
                        HpState::Dhw
                    }
                }
                HpState::Defrost => {
                    // Returning from defrost — go back to previous productive state,
                    // but re-check flow rate in case diverter moved during defrost.
                    if fr >= DHW_ENTER_FLOW_RATE {
                        HpState::Dhw
                    } else if fr < DHW_EXIT_FLOW_RATE {
                        HpState::Heating
                    } else {
                        // In the transition zone — use pre-defrost state
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

/// Fetch all key feeds for a time range and merge into a single DataFrame.
pub fn fetch_dataframe(client: &Client, start: i64, end: i64, interval: u32) -> Result<DataFrame> {
    let feeds: Vec<(&str, &str)> = vec![
        (FEEDS.electric_power, "elec_w"),
        (FEEDS.heat_power, "heat_w"),
        (FEEDS.flow_temp, "flow_t"),
        (FEEDS.return_temp, "return_t"),
        (FEEDS.flow_rate, "flow_rate"),
        (FEEDS.outside_temp, "outside_t"),
        (FEEDS.indoor_temp, "indoor_t"),
    ];

    // Fetch first feed to get timestamps
    let first_data = client.feed_data(feeds[0].0, start, end, interval)?;
    let timestamps: Vec<i64> = first_data.iter().map(|(ts, _)| *ts).collect();
    let first_values: Vec<Option<f64>> = first_data.iter().map(|(_, v)| *v).collect();

    // Build datetime column
    let dt_series = Series::new("timestamp".into(), &timestamps)
        .cast(&DataType::Datetime(TimeUnit::Milliseconds, Some("UTC".into())))
        .context("Failed to create datetime column")?;

    let mut columns: Vec<Column> = vec![dt_series.into()];

    // Add first feed
    columns.push(Series::new(feeds[0].1.into(), &first_values).into());

    // Fetch remaining feeds
    for (id, name) in &feeds[1..] {
        let data = client.feed_data(id, start, end, interval)?;
        let values: Vec<Option<f64>> = data.iter().map(|(_, v)| *v).collect();
        let mut aligned = values;
        aligned.resize(timestamps.len(), None);
        columns.push(Series::new((*name).into(), &aligned).into());
    }

    let df = DataFrame::new(columns).context("Failed to build DataFrame")?;
    Ok(df)
}

/// Add computed columns: COP, delta_T, and operating state.
pub fn enrich(df: &DataFrame) -> Result<DataFrame> {
    // First pass: compute delta_t eagerly (needed for state machine)
    let flow_t = df.column("flow_t")?.f64()?;
    let return_t = df.column("return_t")?.f64()?;
    let delta_t: Vec<f64> = flow_t
        .into_iter()
        .zip(return_t.into_iter())
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

    // Build enriched DataFrame with lazy API for COP
    let lf = df.clone().lazy();
    let enriched = lf
        .with_columns([
            // COP (only meaningful when actively heating/DHW, not during defrost)
            when(col("elec_w").gt(lit(ELEC_RUNNING_W)))
                .then(col("heat_w") / col("elec_w"))
                .otherwise(lit(NULL))
                .alias("cop"),
            // Delta T
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
    // Overall stats (heating + DHW, excluding defrost and idle)
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

    // Breakdown by state
    let by_state = df
        .clone()
        .lazy()
        .filter(col("elec_w").gt(lit(ELEC_RUNNING_W)))
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

    // State distribution (including idle)
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

/// COP by outside temperature bands, for heating only (excluding DHW and defrost).
pub fn cop_by_outside_temp(df: &DataFrame) -> Result<()> {
    let result = df
        .clone()
        .lazy()
        .filter(col("state").eq(lit("heating")))
        .with_column(
            ((col("outside_t") / lit(2.0))
                .floor()
                .cast(DataType::Int32)
                * lit(2))
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

/// Daily totals using cumulative energy feeds (from API).
pub fn daily_energy(client: &Client, start: i64, end: i64) -> Result<()> {
    let elec_data = client.feed_data(FEEDS.electric_energy, start, end, 86400)?;
    let heat_data = client.feed_data(FEEDS.heat_energy, start, end, 86400)?;
    daily_energy_from_data(&elec_data, &heat_data)
}

/// Daily totals from pre-loaded cumulative energy data.
pub fn daily_energy_from_data(
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
