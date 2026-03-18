mod analysis;
mod db;
mod emoncms;
mod gaps;

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "heatpump-analysis")]
#[command(about = "Fetch and analyse heat pump data from emoncms.org")]
struct Cli {
    /// Emoncms read API key
    #[arg(long, env = "EMONCMS_APIKEY")]
    apikey: String,

    /// SQLite database path (default: heatpump.db in current directory)
    #[arg(long, default_value = "heatpump.db")]
    db: PathBuf,

    /// How many days of history to analyse (default 7)
    #[arg(long, default_value = "7")]
    days: u32,

    /// Data interval in seconds — only used for API mode, ignored with local DB
    /// (local DB always stores at 1-minute resolution)
    #[arg(long, default_value = "300")]
    interval: u32,

    /// Force fetching from API instead of local database
    #[arg(long)]
    api: bool,

    /// Include simulated (gap-filled) data in analysis
    #[arg(long)]
    include_simulated: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all available feeds
    Feeds,
    /// Download all data from emoncms to local SQLite database
    Sync,
    /// Show database status (row counts, date range, size)
    DbStatus,
    /// Show raw data table for the time period
    Data,
    /// Summary statistics (overall, by state)
    Summary,
    /// COP broken down by outside temperature bands
    CopByTemp,
    /// Average profile by hour of day
    Hourly,
    /// Daily energy totals and COP from cumulative meters
    Daily,
    /// Show gaps in the data and their fill status
    Gaps,
    /// Fill gaps with synthetic data (modelled from outside temp + real patterns)
    FillGaps,
    /// Run all analyses
    All,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = emoncms::Client::new(&cli.apikey);

    let end = Utc::now().timestamp();
    let start = end - (cli.days as i64 * 86400);

    match cli.command {
        Commands::Feeds => {
            let feeds = client.list_feeds()?;
            println!(
                "{:<10} {:<25} {:<15} {:<8} {}",
                "ID", "Name", "Tag", "Unit", "Value"
            );
            println!("{}", "-".repeat(70));
            for f in feeds {
                println!(
                    "{:<10} {:<25} {:<15} {:<8} {}",
                    f.id,
                    f.name,
                    f.tag,
                    f.unit,
                    f.value
                        .map_or("-".to_string(), |v| format!("{:.1}", v))
                );
            }
        }

        Commands::Sync => {
            eprintln!("Opening database: {}", cli.db.display());
            let conn = db::open(&cli.db)?;

            eprintln!("Syncing feed metadata...");
            let feeds = db::sync_feeds(&conn, &client)?;
            eprintln!("  {} feeds registered", feeds.len());

            eprintln!("Downloading data (1-minute resolution)...");
            let stats = db::sync_all(&conn, &client)?;
            println!("\n{}", stats);
        }

        Commands::DbStatus => {
            let conn = db::open(&cli.db)?;

            // Total samples
            let total: u64 =
                conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))?;

            // Date range
            let (min_ts, max_ts): (i64, i64) = conn.query_row(
                "SELECT COALESCE(MIN(timestamp), 0), COALESCE(MAX(timestamp), 0) FROM samples",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;

            // Per-feed counts
            println!("{:<15} {:<25} {:>10} {:>22} {:>22}",
                "Feed ID", "Name", "Samples", "First", "Last");
            println!("{}", "-".repeat(100));

            let mut stmt = conn.prepare(
                "SELECT s.feed_id, COALESCE(f.name, '?'), COUNT(*),
                        MIN(s.timestamp), MAX(s.timestamp)
                 FROM samples s
                 LEFT JOIN feeds f ON f.id = s.feed_id
                 GROUP BY s.feed_id
                 ORDER BY s.feed_id"
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;

            for row in rows {
                let (id, name, count, first, last) = row?;
                let first_dt = chrono::DateTime::from_timestamp_millis(first)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let last_dt = chrono::DateTime::from_timestamp_millis(last)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                println!("{:<15} {:<25} {:>10} {:>22} {:>22}",
                    id, name, count, first_dt, last_dt);
            }

            // DB file size
            let db_size: u64 = conn
                .query_row(
                    "SELECT page_count * page_size FROM pragma_page_count, pragma_page_size",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            println!("\nTotal: {} samples, {:.1} MB",
                total, db_size as f64 / 1_048_576.0);

            if min_ts > 0 && max_ts > 0 {
                let min_dt = chrono::DateTime::from_timestamp_millis(min_ts).unwrap();
                let max_dt = chrono::DateTime::from_timestamp_millis(max_ts).unwrap();
                let days = (max_ts - min_ts) as f64 / 86_400_000.0;
                println!("Range: {} to {} ({:.0} days)",
                    min_dt.format("%Y-%m-%d"), max_dt.format("%Y-%m-%d"), days);
            }
        }

        Commands::Gaps => {
            let conn = db::open(&cli.db)?;
            gaps::ensure_schema(&conn)?;
            gaps::print_gap_report(&conn)?;
        }

        Commands::FillGaps => {
            let conn = db::open(&cli.db)?;
            gaps::ensure_schema(&conn)?;

            eprintln!("Building model from real data...");
            let model = gaps::TempBinModel::from_db(&conn)?;
            eprintln!(
                "  Heating: {} temp bins, DHW: {} temp bins",
                model.heating_bins.len(),
                model.dhw_bins.len(),
            );

            let gap_list = gaps::find_gaps(&conn, 3.0)?;
            eprintln!("Found {} gaps to fill", gap_list.len());

            let mut total_samples = 0u64;
            for gap in &gap_list {
                let dur = if gap.duration_min > 1440.0 {
                    format!("{:.1}d", gap.duration_min / 1440.0)
                } else if gap.duration_min > 60.0 {
                    format!("{:.1}h", gap.duration_min / 60.0)
                } else {
                    format!("{:.0}m", gap.duration_min)
                };

                let start = chrono::DateTime::from_timestamp_millis(gap.start_ts)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();

                let n = gaps::fill_gap(&conn, gap, &model)?;
                total_samples += n;
                eprintln!("  {} ({}) → {} samples", start, dur, n);
            }

            println!(
                "\nFilled {} gaps, generated {} simulated samples",
                gap_list.len(),
                total_samples,
            );
        }

        Commands::Data => {
            let df = load_or_fetch(&cli, &client, start, end)?;
            let df = analysis::enrich(&df)?;
            println!("{}", df);
        }

        Commands::Summary => {
            let df = load_or_fetch(&cli, &client, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
        }

        Commands::CopByTemp => {
            let df = load_or_fetch(&cli, &client, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::cop_by_outside_temp(&df)?;
        }

        Commands::Hourly => {
            let df = load_or_fetch(&cli, &client, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::hourly_profile(&df)?;
        }

        Commands::Daily => {
            if !cli.api && cli.db.exists() {
                let conn = db::open(&cli.db)?;
                let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
                analysis::daily_energy_from_data(&elec, &heat)?;
            } else {
                analysis::daily_energy(&client, start, end)?;
            }
        }

        Commands::All => {
            let df = load_or_fetch(&cli, &client, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
            analysis::cop_by_outside_temp(&df)?;
            analysis::hourly_profile(&df)?;

            if !cli.api && cli.db.exists() {
                let conn = db::open(&cli.db)?;
                let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
                analysis::daily_energy_from_data(&elec, &heat)?;
            } else {
                analysis::daily_energy(&client, start, end)?;
            }
        }
    }

    Ok(())
}

/// Load data from local DB if available, otherwise fetch from API.
fn load_or_fetch(
    cli: &Cli,
    client: &emoncms::Client,
    start: i64,
    end: i64,
) -> Result<polars::prelude::DataFrame> {
    if !cli.api && cli.db.exists() {
        let conn = db::open(&cli.db)?;
        if cli.include_simulated {
            eprintln!("Loading from local database (including simulated): {}", cli.db.display());
            db::load_dataframe_with_simulated(&conn, start, end)
        } else {
            eprintln!("Loading from local database: {}", cli.db.display());
            db::load_dataframe(&conn, start, end)
        }
    } else {
        if !cli.api {
            eprintln!("No local database found, fetching from API (run 'sync' first for faster queries)");
        }
        analysis::fetch_dataframe(client, start, end, cli.interval)
    }
}
