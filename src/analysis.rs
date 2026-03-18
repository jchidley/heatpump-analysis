//! Build Polars DataFrames from emoncms data and run analyses.

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
        // Pad/truncate to match timestamp length
        let mut aligned = values;
        aligned.resize(timestamps.len(), None);
        columns.push(Series::new((*name).into(), &aligned).into());
    }

    let df = DataFrame::new(columns).context("Failed to build DataFrame")?;
    Ok(df)
}

/// Add computed columns: COP, delta_T, hour, date, month, is_dhw flag.
pub fn enrich(df: &DataFrame) -> Result<DataFrame> {
    let lf = df.clone().lazy();

    let enriched = lf
        .with_columns([
            // COP = heat / electric (only when electric > 50W to avoid divide-by-tiny-number)
            when(col("elec_w").gt(lit(50.0)))
                .then(col("heat_w") / col("elec_w"))
                .otherwise(lit(NULL))
                .alias("cop"),
            // Delta T
            (col("flow_t") - col("return_t")).alias("delta_t"),
            // DHW flag: flow rate > 14.5 l/m indicates DHW mode
            // Heating clusters at 14.3 l/m, DHW jumps to 16.5-17.0 l/m
            // The gap between 14.5 and 16.0 is near-empty (transitions only)
            when(col("flow_rate").gt(lit(14.5)))
                .then(lit(true))
                .otherwise(lit(false))
                .alias("is_dhw"),
        ])
        .collect()
        .context("Failed to enrich DataFrame")?;

    Ok(enriched)
}

/// Summary statistics for a time period.
pub fn summary(df: &DataFrame) -> Result<()> {
    let lf = df.clone().lazy();

    // Overall stats
    let stats = lf
        .clone()
        .filter(col("elec_w").gt(lit(50.0)))
        .select([
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("cop").mean().alias("avg_cop"),
            col("flow_t").mean().alias("avg_flow_t"),
            col("return_t").mean().alias("avg_return_t"),
            col("outside_t").mean().alias("avg_outside_t"),
            col("cop").min().alias("min_cop"),
            col("cop").max().alias("max_cop"),
            len().alias("running_samples"),
        ])
        .collect()?;

    println!("\n=== Summary (when running, elec > 50W) ===");
    println!("{}", stats);

    // DHW vs Space Heating breakdown
    let dhw_stats = df
        .clone()
        .lazy()
        .filter(col("elec_w").gt(lit(50.0)))
        .group_by([col("is_dhw")])
        .agg([
            col("cop").mean().alias("avg_cop"),
            col("elec_w").mean().alias("avg_elec_w"),
            col("heat_w").mean().alias("avg_heat_w"),
            col("flow_t").mean().alias("avg_flow_t"),
            len().alias("samples"),
        ])
        .sort(["is_dhw"], Default::default())
        .collect()?;

    println!("\n=== DHW vs Space Heating ===");
    println!("{}", dhw_stats);

    Ok(())
}

/// COP by outside temperature bands (e.g. 0-2, 2-4, 4-6, ...).
pub fn cop_by_outside_temp(df: &DataFrame) -> Result<()> {
    let result = df
        .clone()
        .lazy()
        .filter(col("elec_w").gt(lit(50.0)))
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

    println!("\n=== COP by Outside Temperature Band (°C) ===");
    println!("{}", result);

    Ok(())
}

/// Hourly profile: average COP, power, temps by hour of day.
pub fn hourly_profile(df: &DataFrame) -> Result<()> {
    let result = df
        .clone()
        .lazy()
        .filter(col("elec_w").gt(lit(50.0)))
        .with_column(
            col("timestamp")
                .dt()
                .hour()
                .alias("hour"),
        )
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

    println!("\n=== Hourly Profile ===");
    println!("{}", result);

    Ok(())
}

/// Daily totals using cumulative energy feeds.
pub fn daily_energy(client: &Client, start: i64, end: i64) -> Result<()> {
    // Use daily interval for cumulative feeds
    let elec_data = client.feed_data(FEEDS.electric_energy, start, end, 86400)?;
    let heat_data = client.feed_data(FEEDS.heat_energy, start, end, 86400)?;

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
