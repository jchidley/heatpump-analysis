mod analysis;
mod config;
mod db;
mod emoncms;
mod gaps;
mod octopus;
mod octopus_tariff;
mod overnight;
mod thermal;

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use polars::prelude::SerWriter;
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::Value;

#[derive(Parser)]
#[command(name = "heatpump-analysis")]
#[command(about = "Fetch and analyse heat pump data from emoncms.org")]
struct Cli {
    /// Emoncms read API key (required for 'feeds' and 'sync' commands)
    #[arg(long, env = "EMONCMS_APIKEY", default_value = "")]
    apikey: String,

    /// SQLite database path
    #[arg(long, default_value = "heatpump.db")]
    db: PathBuf,

    /// How many days of history to analyse (default 7)
    #[arg(long, default_value = "7")]
    days: u32,

    /// Analyse all available data (overrides --days)
    #[arg(long)]
    all_data: bool,

    /// Start date for analysis (YYYY-MM-DD). Overrides --days.
    #[arg(long)]
    from: Option<String>,

    /// End date for analysis (YYYY-MM-DD). Defaults to now.
    #[arg(long)]
    to: Option<String>,

    /// Include simulated (gap-filled) data in analysis
    #[arg(long)]
    include_simulated: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum ThermalSnapshotCommands {
    /// Export explicit thermal reproducibility snapshot with signoff manifest
    Export {
        /// Path to thermal config TOML included in snapshot manifest
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Mandatory human signoff reason for snapshot creation
        #[arg(long)]
        signoff_reason: String,
        /// Explicit human approval gate (required)
        #[arg(long)]
        approved_by_human: bool,
    },
    /// Import a previously exported thermal snapshot manifest
    Import {
        /// Path to snapshot manifest JSON
        #[arg(long)]
        manifest: String,
        /// Mandatory human signoff reason for snapshot import
        #[arg(long)]
        signoff_reason: String,
        /// Explicit human approval gate (required)
        #[arg(long)]
        approved_by_human: bool,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum HistoryReviewTarget {
    Heating,
    Dhw,
    Both,
}

#[derive(Subcommand)]
enum Commands {
    /// List all available feeds from emoncms API
    Feeds,
    /// Download/update data from emoncms to local SQLite database
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
    /// Degree day analysis — energy normalised by heating demand
    DegreeDays,
    /// Indoor temperature analysis (Leather room sensor)
    IndoorTemp,
    /// DHW analysis vs design expectations
    Dhw,
    /// Compare actual COP against Arotherm manufacturer spec
    CopVsSpec,
    /// Compare actual performance against design calculations and gas-era data
    DesignComparison,
    /// Export data to CSV for the time period
    Export {
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Octopus Energy data summary (consumption + weather + monthly breakdown)
    Octopus,
    /// Compare gas-era vs heat-pump-era energy use (normalised by degree days)
    GasVsHp,
    /// Baseload analysis: whole-house electricity minus heat pump electricity
    Baseload,
    /// Overnight heating strategy optimizer — backtest optimal schedules vs actual
    Overnight,
    /// Run all analyses
    All,
    /// Print room summary table (geometry, thermal mass, radiators)
    ThermalRooms,
    /// Print inter-room connections and doorway exchanges
    ThermalConnections,
    /// Live energy balance from InfluxDB (per-room heat flows)
    ThermalAnalyse {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Analyse moisture: current condensation risk + overnight humidity balance
    ThermalMoisture {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Solve for equilibrium room temperatures at given conditions
    ThermalEquilibrium {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Outside temperature (°C). Defaults to current from InfluxDB.
        #[arg(long)]
        outside: Option<f64>,
        /// Mean water temperature (°C). Defaults to current from InfluxDB.
        #[arg(long)]
        mwt: Option<f64>,
        /// Solar irradiance on SW vertical surfaces (W/m²)
        #[arg(long, default_value = "0")]
        solar_sw: f64,
        /// Solar irradiance on NE vertical surfaces (W/m²)
        #[arg(long, default_value = "0")]
        solar_ne: f64,
    },
    /// Generate control lookup table for adaptive heating controller
    ThermalControlTable {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Calibrate thermal model parameters from InfluxDB using fixed test windows
    ThermalCalibrate {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Validate calibrated thermal model on holdout windows (beyond calibration nights)
    ThermalValidate {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Run period-by-period cooldown fit diagnostics (Rust parity with Python fit)
    ThermalFitDiagnostics {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Operational validation: predict room temps during normal heated operation
    ThermalOperational {
        /// Path to thermal calibration config TOML
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
    },
    /// Explicit thermal snapshot export/import workflow (human-gated)
    ThermalSnapshot {
        #[command(subcommand)]
        action: ThermalSnapshotCommands,
    },
    /// DHW session analysis — draws, charges, inflection detection, HWC tracking
    DhwSessions {
        /// Path to thermal calibration config TOML (for InfluxDB connection)
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Days of history to analyse
        #[arg(long, default_value = "12")]
        days: u32,
        /// Output format: human, verbose, or json
        #[arg(long, default_value = "verbose")]
        format: String,
        /// Don't write results to InfluxDB
        #[arg(long)]
        no_write: bool,
    },
    /// Query live DHW state from z2m-hub (JSON by default; use --human for operator view)
    DhwLiveStatus {
        /// z2m-hub base URL
        #[arg(long, default_value = "http://pi5data:3030")]
        base_url: String,
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
    },
    /// Reconstruct fused high-resolution heating-history evidence (defaults to last 7 days ending now)
    HeatingHistory {
        /// Path to thermal calibration config TOML (for InfluxDB connection)
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Inclusive start of window (RFC3339). Defaults to --days before --until/now.
        #[arg(long)]
        since: Option<String>,
        /// Exclusive end of window (RFC3339). Defaults to now.
        #[arg(long)]
        until: Option<String>,
        /// Rolling window size when --since is omitted
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
        /// Print raw InfluxDB Flux profiler output for key queries to stderr
        #[arg(long)]
        profile_queries: bool,
    },
    /// Reconstruct fused high-resolution DHW-history evidence (defaults to last 7 days ending now)
    DhwHistory {
        /// Path to thermal calibration config TOML (for InfluxDB connection)
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Inclusive start of window (RFC3339). Defaults to --days before --until/now.
        #[arg(long)]
        since: Option<String>,
        /// Exclusive end of window (RFC3339). Defaults to now.
        #[arg(long)]
        until: Option<String>,
        /// Rolling window size when --since is omitted
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
        /// Print raw InfluxDB Flux profiler output for key queries to stderr
        #[arg(long)]
        profile_queries: bool,
    },
    /// Native-cadence DHW drill-down for one bounded event/window
    DhwDrilldown {
        /// Path to thermal calibration config TOML (for InfluxDB connection)
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Inclusive start of drill-down window (RFC3339)
        #[arg(long)]
        since: String,
        /// Exclusive end of drill-down window (RFC3339)
        #[arg(long)]
        until: String,
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
    },
    /// Comprehensive high-resolution historical review (defaults to maximum-detail last 7 days ending now)
    HistoryReview {
        /// Domain to review
        #[arg(value_enum, default_value_t = HistoryReviewTarget::Both)]
        target: HistoryReviewTarget,
        /// Path to thermal calibration config TOML (for InfluxDB connection)
        #[arg(long, default_value = "model/thermal-config.toml")]
        config: String,
        /// Inclusive start of window (RFC3339). Defaults to --days before --until/now.
        #[arg(long)]
        since: Option<String>,
        /// Exclusive end of window (RFC3339). Defaults to now.
        #[arg(long)]
        until: Option<String>,
        /// Rolling window size when --since is omitted
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Human-oriented summary output
        #[arg(long)]
        human: bool,
        /// Skip DHW session analysis when reviewing DHW/both
        #[arg(long)]
        no_sessions: bool,
    },
}

impl Cli {
    /// Get the API client, failing if no key was provided.
    fn require_client(&self) -> Result<emoncms::Client> {
        anyhow::ensure!(
            !self.apikey.is_empty(),
            "This command requires --apikey or EMONCMS_APIKEY"
        );
        Ok(emoncms::Client::new(&self.apikey))
    }

    /// Open the local database, failing if it doesn't exist.
    fn require_db(&self) -> Result<rusqlite::Connection> {
        anyhow::ensure!(
            self.db.exists(),
            "Database not found: {}. Run 'sync' first to download data.",
            self.db.display()
        );
        db::open(&self.db)
    }
}

fn main() -> Result<()> {
    // Load config.toml from next to the executable, or fall back to cwd
    let config_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.toml")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("config.toml"));
    config::load(&config_path)?;

    let cli = Cli::parse();

    let (start, end) = resolve_time_range(&cli)?;

    match cli.command {
        Commands::Feeds => {
            let client = cli.require_client()?;
            let feeds = client.list_feeds()?;
            println!(
                "{:<10} {:<25} {:<15} {:<8} Value",
                "ID", "Name", "Tag", "Unit"
            );
            println!("{}", "-".repeat(70));
            for f in feeds {
                println!(
                    "{:<10} {:<25} {:<15} {:<8} {}",
                    f.id,
                    f.name,
                    f.tag,
                    f.unit,
                    f.value.map_or("-".to_string(), |v| format!("{:.1}", v))
                );
            }
        }

        Commands::Sync => {
            let client = cli.require_client()?;
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
            let conn = cli.require_db()?;

            let total: u64 = conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))?;

            let (min_ts, max_ts): (i64, i64) = conn.query_row(
                "SELECT COALESCE(MIN(timestamp), 0), COALESCE(MAX(timestamp), 0) FROM samples",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;

            println!(
                "{:<15} {:<25} {:>10} {:>22} {:>22}",
                "Feed ID", "Name", "Samples", "First", "Last"
            );
            println!("{}", "-".repeat(100));

            let mut stmt = conn.prepare(
                "SELECT s.feed_id, COALESCE(f.name, '?'), COUNT(*),
                        MIN(s.timestamp), MAX(s.timestamp)
                 FROM samples s
                 LEFT JOIN feeds f ON f.id = s.feed_id
                 GROUP BY s.feed_id
                 ORDER BY s.feed_id",
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
                println!(
                    "{:<15} {:<25} {:>10} {:>22} {:>22}",
                    id, name, count, first_dt, last_dt
                );
            }

            let db_size: u64 = conn
                .query_row(
                    "SELECT page_count * page_size FROM pragma_page_count, pragma_page_size",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            println!(
                "\nTotal: {} samples, {:.1} MB",
                total,
                db_size as f64 / 1_048_576.0
            );

            if min_ts > 0 && max_ts > 0 {
                let min_dt = chrono::DateTime::from_timestamp_millis(min_ts).unwrap();
                let max_dt = chrono::DateTime::from_timestamp_millis(max_ts).unwrap();
                let days = (max_ts - min_ts) as f64 / 86_400_000.0;
                println!(
                    "Range: {} to {} ({:.0} days)",
                    min_dt.format("%Y-%m-%d"),
                    max_dt.format("%Y-%m-%d"),
                    days
                );
            }
        }

        Commands::Gaps => {
            let conn = cli.require_db()?;
            gaps::ensure_schema(&conn)?;
            gaps::print_gap_report(&conn)?;
        }

        Commands::FillGaps => {
            let conn = cli.require_db()?;
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
                let dur = format_duration(gap.duration_min);
                let start_str = chrono::DateTime::from_timestamp_millis(gap.start_ts)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();

                let n = gaps::fill_gap(&conn, gap, &model)?;
                total_samples += n;
                eprintln!("  {} ({}) → {} samples", start_str, dur, n);
            }

            println!(
                "\nFilled {} gaps, generated {} simulated samples",
                gap_list.len(),
                total_samples,
            );
        }

        Commands::IndoorTemp => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::indoor_temp(&df)?;
        }

        Commands::Dhw => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::dhw_analysis(&df)?;
        }

        Commands::CopVsSpec => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::cop_vs_spec(&df)?;
        }

        Commands::DesignComparison => {
            let conn = cli.require_db()?;
            let temps = db::load_daily_outside_temp(&conn, start, end)?;
            let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::cop_vs_spec(&df)?;
            analysis::design_comparison(&temps, &elec, &heat)?;
        }

        Commands::DegreeDays => {
            let conn = cli.require_db()?;
            let temps = db::load_daily_outside_temp(&conn, start, end)?;
            let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
            analysis::degree_days(&temps, &elec, &heat)?;
        }

        Commands::Export { ref output } => {
            let df = load_dataframe(&cli, start, end)?;
            let mut df = analysis::enrich(&df)?;
            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(path)?;
                    polars::prelude::CsvWriter::new(&mut file).finish(&mut df)?;
                    eprintln!("Wrote {} rows to {}", df.height(), path);
                }
                None => {
                    let mut stdout = std::io::stdout();
                    polars::prelude::CsvWriter::new(&mut stdout).finish(&mut df)?;
                }
            }
        }

        Commands::Data => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            println!("{}", df);
        }

        Commands::Summary => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
        }

        Commands::CopByTemp => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::cop_by_outside_temp(&df)?;
        }

        Commands::Hourly => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::hourly_profile(&df)?;
        }

        Commands::Daily => {
            let conn = cli.require_db()?;
            let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
            analysis::daily_energy(&elec, &heat)?;
        }

        Commands::Octopus => {
            let consumption = octopus::load_consumption(None)?;
            let conn = cli.require_db().ok();
            let weather = octopus::load_weather(None, conn.as_ref())?;
            octopus::print_summary(&consumption, &weather)?;
        }

        Commands::GasVsHp => {
            let consumption = octopus::load_consumption(None)?;
            let conn = cli.require_db().ok();
            let weather = octopus::load_weather(None, conn.as_ref())?;

            // Load enriched HP data with state machine classification
            let hp_by_state = if let Some(ref c) = conn {
                let df = if cli.include_simulated {
                    db::load_dataframe_with_simulated(c, start, end)?
                } else {
                    db::load_dataframe(c, start, end)?
                };
                let enriched = analysis::enrich(&df)?;
                Some(octopus::daily_hp_by_state(&enriched)?)
            } else {
                None
            };

            octopus::print_gas_vs_hp(&consumption, &weather, hp_by_state.as_deref())?;
        }

        Commands::Baseload => {
            let consumption = octopus::load_consumption(None)?;
            let conn = cli.require_db()?;
            let (elec_cum, _heat_cum) = db::load_daily_energy(&conn, start, end)?;

            // Convert cumulative to daily deltas
            let mut hp_daily: Vec<(String, f64)> = Vec::new();
            for i in 1..elec_cum.len() {
                if let (Some(v1), Some(v0)) = (elec_cum[i].1, elec_cum[i - 1].1) {
                    let delta = v1 - v0;
                    if (0.0..200.0).contains(&delta) {
                        let date = chrono::DateTime::from_timestamp_millis(elec_cum[i].0)
                            .map(|dt| dt.format("%Y-%m-%d").to_string())
                            .unwrap_or_default();
                        hp_daily.push((date, delta));
                    }
                }
            }

            octopus::print_baseload(&consumption, &hp_daily)?;
        }

        Commands::Overnight => {
            let conn = cli.require_db()?;
            let df = if cli.include_simulated {
                db::load_dataframe_with_simulated(&conn, start, end)?
            } else {
                db::load_dataframe(&conn, start, end)?
            };
            let enriched = analysis::enrich(&df)?;
            overnight::overnight_analysis(&enriched)?;
        }

        Commands::All => {
            let df = load_dataframe(&cli, start, end)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
            analysis::cop_by_outside_temp(&df)?;
            analysis::hourly_profile(&df)?;

            let conn = cli.require_db()?;
            let (elec, heat) = db::load_daily_energy(&conn, start, end)?;
            analysis::daily_energy(&elec, &heat)?;

            let temps = db::load_daily_outside_temp(&conn, start, end)?;
            analysis::degree_days(&temps, &elec, &heat)?;
        }

        Commands::ThermalRooms => {
            thermal::print_rooms()?;
        }

        Commands::ThermalConnections => {
            thermal::print_connections()?;
        }

        Commands::ThermalAnalyse { ref config } => {
            thermal::print_analyse(std::path::Path::new(config))?;
        }

        Commands::ThermalMoisture { ref config } => {
            thermal::print_moisture(std::path::Path::new(config))?;
        }

        Commands::ThermalEquilibrium {
            ref config,
            outside,
            mwt,
            solar_sw,
            solar_ne,
        } => {
            thermal::print_equilibrium(
                std::path::Path::new(config),
                outside,
                mwt,
                solar_sw,
                solar_ne,
            )?;
        }

        Commands::ThermalControlTable { ref config } => {
            thermal::generate_control_table(std::path::Path::new(config))?;
        }

        Commands::ThermalCalibrate { ref config } => {
            thermal::calibrate(std::path::Path::new(config))?;
        }

        Commands::ThermalValidate { ref config } => {
            thermal::validate(std::path::Path::new(config))?;
        }

        Commands::ThermalFitDiagnostics { ref config } => {
            thermal::fit_diagnostics(std::path::Path::new(config))?;
        }

        Commands::ThermalOperational { ref config } => {
            thermal::operational_validate(std::path::Path::new(config))?;
        }

        Commands::ThermalSnapshot { ref action } => match action {
            ThermalSnapshotCommands::Export {
                config,
                signoff_reason,
                approved_by_human,
            } => {
                let manifest_path = thermal::snapshot_export(
                    std::path::Path::new(config),
                    signoff_reason,
                    *approved_by_human,
                )?;
                println!(
                    "Wrote thermal snapshot manifest: {}",
                    manifest_path.display()
                );
            }
            ThermalSnapshotCommands::Import {
                manifest,
                signoff_reason,
                approved_by_human,
            } => {
                thermal::snapshot_import(
                    std::path::Path::new(manifest),
                    signoff_reason,
                    *approved_by_human,
                )?;
                println!("Imported thermal snapshot from manifest: {}", manifest);
            }
        },

        Commands::DhwSessions {
            ref config,
            days,
            ref format,
            no_write,
        } => {
            let output = match format.as_str() {
                "json" => thermal::DhwSessionsOutput::Json,
                "human" => thermal::DhwSessionsOutput::Human,
                _ => thermal::DhwSessionsOutput::Verbose,
            };
            thermal::dhw_sessions(config, days, output, no_write)?;
        }

        Commands::DhwLiveStatus {
            ref base_url,
            human,
        } => {
            print_dhw_live_status(base_url, human)?;
        }

        Commands::HeatingHistory {
            ref config,
            ref since,
            ref until,
            days,
            human,
            profile_queries,
        } => {
            let (since, until) = resolve_history_window(since.as_deref(), until.as_deref(), days)?;
            thermal::heating_history(
                std::path::Path::new(config),
                &since,
                &until,
                human,
                profile_queries,
            )?;
        }

        Commands::DhwHistory {
            ref config,
            ref since,
            ref until,
            days,
            human,
            profile_queries,
        } => {
            let (since, until) = resolve_history_window(since.as_deref(), until.as_deref(), days)?;
            thermal::dhw_history(
                std::path::Path::new(config),
                &since,
                &until,
                human,
                profile_queries,
            )?;
        }

        Commands::DhwDrilldown {
            ref config,
            ref since,
            ref until,
            human,
        } => {
            thermal::dhw_drilldown(std::path::Path::new(config), since, until, human)?;
        }

        Commands::HistoryReview {
            target,
            ref config,
            ref since,
            ref until,
            days,
            human,
            no_sessions,
        } => {
            let explicit_window = since.is_some() || until.is_some();
            let (since, until) = resolve_history_window(since.as_deref(), until.as_deref(), days)?;
            run_history_review(
                std::path::Path::new(config),
                target,
                &since,
                &until,
                human,
                no_sessions,
                explicit_window,
            )?;
        }
    }

    Ok(())
}

/// Load a DataFrame from the local database.
fn load_dataframe(cli: &Cli, start: i64, end: i64) -> Result<polars::prelude::DataFrame> {
    let conn = cli.require_db()?;
    if cli.include_simulated {
        eprintln!("Loading from {} (including simulated)", cli.db.display());
        db::load_dataframe_with_simulated(&conn, start, end)
    } else {
        eprintln!("Loading from {}", cli.db.display());
        db::load_dataframe(&conn, start, end)
    }
}

/// Parse --from/--to dates, --all-data, or fall back to --days.
#[derive(Debug, serde::Serialize)]
struct DhwLiveSummary {
    charge_state: Option<String>,
    crossover_achieved: Option<bool>,
    remaining_litres: Option<f64>,
    full_litres: Option<f64>,
    effective_t1_c: Option<f64>,
    t1_c: Option<f64>,
    hwc_storage_c: Option<f64>,
    target_c: Option<f64>,
    sfmode: Option<String>,
    charging: Option<bool>,
    safe_for_two_showers: Option<bool>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct HistoryReviewSummary {
    generated_at: String,
    target: String,
    window: HistoryReviewWindow,
    heating_verdict: Option<HistoryVerdict>,
    dhw_verdict: Option<HistoryVerdict>,
    heating: Option<thermal::HeatingHistorySummary>,
    dhw: Option<thermal::DhwHistorySummary>,
    dhw_sessions: Option<Value>,
    no_sessions: bool,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct HistoryReviewWindow {
    since: String,
    until: String,
}

#[derive(Serialize)]
struct HistoryVerdict {
    status: &'static str,
    change_under_review: String,
    success_criteria_checked: Vec<String>,
    supporting_evidence: Vec<String>,
    confounders: Vec<String>,
    recommended_next_change: String,
}

fn run_history_review(
    config_path: &std::path::Path,
    target: HistoryReviewTarget,
    since: &str,
    until: &str,
    human: bool,
    no_sessions: bool,
    explicit_window: bool,
) -> Result<()> {
    let days = history_window_days(since, until)?;
    let config_path_str = config_path.to_string_lossy().to_string();
    let heating = match target {
        HistoryReviewTarget::Heating | HistoryReviewTarget::Both => Some(
            thermal::heating_history_summary(config_path, since, until, false)?,
        ),
        HistoryReviewTarget::Dhw => None,
    };
    let dhw =
        match target {
            HistoryReviewTarget::Dhw | HistoryReviewTarget::Both => Some(
                thermal::dhw_history_summary(config_path, since, until, false)?,
            ),
            HistoryReviewTarget::Heating => None,
        };

    let mut review_warnings = Vec::new();
    let dhw_sessions = match target {
        HistoryReviewTarget::Dhw | HistoryReviewTarget::Both if !no_sessions && explicit_window => {
            review_warnings.push(
                "dhw_sessions omitted because the requested history-review window is exact; current session analysis is day-rounded and could include out-of-window evidence".to_string(),
            );
            None
        }
        HistoryReviewTarget::Dhw | HistoryReviewTarget::Both if !no_sessions => Some(
            thermal::dhw_sessions_json_summary(&config_path_str, days, true)?,
        ),
        _ => None,
    };

    let heating_verdict = heating.as_ref().map(heating_history_verdict);
    let dhw_verdict = dhw.as_ref().map(dhw_history_verdict);

    if human {
        let now_utc = Utc::now().to_rfc3339();
        println!("History review");
        println!("==============");
        println!("now_utc: {now_utc}");
        println!("window: {since} → {until}");
        println!("target: {:?}", target);
        println!("mode: compact human summary only");
        println!("note: structured default output is the machine/LLM interface; use heating-history or dhw-history directly for detailed factual views");
        println!();

        if let Some(verdict) = &heating_verdict {
            print_history_verdict_human("Heating verdict", verdict);
            println!();
        }
        if let Some(verdict) = &dhw_verdict {
            print_history_verdict_human("DHW verdict", verdict);
            println!();
        }
        if let Some(session_summary) = &dhw_sessions {
            println!("dhw_sessions_summary:");
            println!("{}", serde_json::to_string_pretty(session_summary)?);
            println!();
        }
        if review_warnings.is_empty() {
            println!("review_warnings: none");
        } else {
            println!("review_warnings:");
            for warning in &review_warnings {
                println!("- {warning}");
            }
        }

        return Ok(());
    }

    let summary = HistoryReviewSummary {
        generated_at: Utc::now().to_rfc3339(),
        target: format!("{:?}", target).to_lowercase(),
        window: HistoryReviewWindow {
            since: since.to_string(),
            until: until.to_string(),
        },
        heating_verdict,
        dhw_verdict,
        heating,
        dhw,
        dhw_sessions,
        no_sessions,
        warnings: review_warnings,
    };

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn dedupe_strings(values: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn print_history_verdict_human(label: &str, verdict: &HistoryVerdict) {
    println!("{label}");
    println!("{}", "-".repeat(label.len()));
    println!("status: {}", verdict.status);
    println!("change_under_review: {}", verdict.change_under_review);
    if verdict.success_criteria_checked.is_empty() {
        println!("success_criteria_checked: none");
    } else {
        println!("success_criteria_checked:");
        for item in &verdict.success_criteria_checked {
            println!("- {item}");
        }
    }
    if verdict.supporting_evidence.is_empty() {
        println!("supporting_evidence: none");
    } else {
        println!("supporting_evidence:");
        for item in &verdict.supporting_evidence {
            println!("- {item}");
        }
    }
    if verdict.confounders.is_empty() {
        println!("confounders: none");
    } else {
        println!("confounders:");
        for item in &verdict.confounders {
            println!("- {item}");
        }
    }
    println!(
        "recommended_next_change: {}",
        verdict.recommended_next_change
    );
}

fn heating_history_verdict(summary: &thermal::HeatingHistorySummary) -> HistoryVerdict {
    let comfort_miss_count = summary.events.comfort_miss_periods.len();
    let dhw_overlap_count = summary.events.dhw_overlap_periods.len();
    let has_core_comfort = summary.leather_c.is_some();
    let has_controller_intent =
        !summary.controller_events.is_empty() || summary.target_flow_c.is_some();

    let mut success_criteria_checked = vec![
        "waking-hours comfort misses are acceptably rare".to_string(),
        "controller intent and actuator response are present in the review window".to_string(),
        "DHW overlap is not materially undermining comfort".to_string(),
    ];
    if summary.events.likely_preheat_start.is_some() {
        success_criteria_checked
            .push("overnight preheat start is visible in controller evidence".to_string());
    }

    let mut supporting_evidence = Vec::new();
    if let Some(leather) = &summary.leather_c {
        if let Some(latest) = &leather.latest {
            supporting_evidence.push(format!(
                "Leather latest {:.2}°C at {}",
                latest.value, latest.ts
            ));
        }
    }
    supporting_evidence.push(format!(
        "comfort miss periods detected: {comfort_miss_count}"
    ));
    supporting_evidence.push(format!("DHW overlap periods detected: {dhw_overlap_count}"));
    supporting_evidence.push(format!(
        "controller events recorded: {}",
        summary.controller_events.len()
    ));
    if let Some(preheat) = &summary.events.likely_preheat_start {
        supporting_evidence.push(format!(
            "likely preheat start at {} via {}",
            preheat.ts, preheat.action
        ));
    }
    if summary.events.likely_sawtooth {
        supporting_evidence.push(format!(
            "sawtooth candidate detected (alternations={})",
            summary.events.sawtooth_alternations
        ));
    }

    let mut confounders = Vec::new();
    if dhw_overlap_count > 0 {
        confounders.push("DHW overlap was present in the review window".to_string());
    }
    if summary.events.likely_sawtooth {
        confounders.push("controller behaviour showed a sawtooth candidate".to_string());
    }
    confounders.extend(summary.warnings.iter().cloned());
    dedupe_strings(&mut confounders);

    let (status, recommended_next_change) = if !has_core_comfort || !has_controller_intent {
        (
            "inconclusive",
            "Restore missing heating evidence inputs before changing control logic.".to_string(),
        )
    } else if comfort_miss_count == 0 && !summary.events.likely_sawtooth {
        (
            "working",
            "Hold the current heating plan and gather more clean overnight windows before retuning.".to_string(),
        )
    } else if comfort_miss_count > 0 && dhw_overlap_count == 0 && !summary.events.likely_sawtooth {
        (
            "failing",
            "Investigate overnight planner timing and actuator follow-through on the next clean overnight anchor.".to_string(),
        )
    } else {
        let next = if dhw_overlap_count > 0 {
            "Review matched heating/DHW windows and consider DHW timing or pre-DHW banking before retuning heating."
        } else if summary.events.likely_sawtooth {
            "Validate the sawtooth pattern on a clean disturbance-free window before changing control gains."
        } else {
            "Review another clean overnight window before deciding on the next heating change."
        };
        ("mixed", next.to_string())
    };

    HistoryVerdict {
        status,
        change_under_review: "adaptive heating overnight planner and coupled heating control"
            .to_string(),
        success_criteria_checked,
        supporting_evidence,
        confounders,
        recommended_next_change,
    }
}

fn dhw_history_verdict(summary: &thermal::DhwHistorySummary) -> HistoryVerdict {
    let full_count = summary
        .charges_detected
        .iter()
        .filter(|c| c.crossover == Some(true))
        .count();
    let partial_count = summary.charges_detected.len().saturating_sub(full_count);
    let has_core_dhw = summary.t1_c.is_some() && summary.hwc_storage_c.is_some();

    let success_criteria_checked = vec![
        "charge timing keeps hot-water supply practical".to_string(),
        "full-charge fraction is acceptable".to_string(),
        "partial or no-crossover charges are rare or explained".to_string(),
        "T1 and HwcStorageTemp behaviour are consistent with the current plan".to_string(),
    ];

    let mut supporting_evidence = vec![
        format!("charges detected: {}", summary.charges_detected.len()),
        format!("full charges: {full_count}"),
        format!("partial charges: {partial_count}"),
    ];
    if let Some(t1) = &summary.t1_c {
        if let Some(latest) = &t1.latest {
            supporting_evidence.push(format!("T1 latest {:.1}°C at {}", latest.value, latest.ts));
        }
    }
    if let Some(remaining) = &summary.remaining_litres {
        if let Some(latest) = &remaining.latest {
            supporting_evidence.push(format!(
                "remaining litres latest {:.0}L at {}",
                latest.value, latest.ts
            ));
        }
    }
    if let Some(max_div) = summary.events.max_t1_hwc_divergence_c {
        supporting_evidence.push(format!("max T1/HWC divergence {:.1}°C", max_div));
    }

    let mut confounders = Vec::new();
    if summary.events.large_t1_hwc_divergence {
        confounders.push("large T1/HwcStorageTemp divergence detected".to_string());
    }
    if summary.events.hwc_sfmode_load_stuck {
        confounders.push("HwcSFMode may be stuck on load".to_string());
    }
    confounders.extend(summary.warnings.iter().cloned());
    dedupe_strings(&mut confounders);

    let (status, recommended_next_change) = if !has_core_dhw {
        (
            "inconclusive",
            "Restore missing DHW evidence inputs before changing DHW control logic.".to_string(),
        )
    } else if summary.charges_detected.is_empty() {
        (
            "inconclusive",
            "Wait for a representative DHW charge window or replay a named anchor before changing DHW logic.".to_string(),
        )
    } else if !summary.events.no_crossover && !summary.events.low_t1 && partial_count == 0 {
        (
            "working",
            "Hold the current DHW plan and keep collecting representative charge windows before retuning.".to_string(),
        )
    } else if summary.events.no_crossover && summary.events.low_t1 {
        (
            "failing",
            "Revisit DHW trigger/completion logic on the next representative charge because non-crossover behaviour is now coinciding with low T1.".to_string(),
        )
    } else {
        let next = if summary.events.no_crossover {
            "Inspect the next partial-charge anchor and validate whether T1-based completion logic should change."
        } else if summary.events.large_t1_hwc_divergence {
            "Keep using T1 as the comfort truth and review whether lower-cylinder-trigger behaviour is still operationally sensible."
        } else {
            "Review another representative DHW charge before changing the plan."
        };
        ("mixed", next.to_string())
    };

    HistoryVerdict {
        status,
        change_under_review: "T1-informed DHW timing and charge-completion interpretation"
            .to_string(),
        success_criteria_checked,
        supporting_evidence,
        confounders,
        recommended_next_change,
    }
}

fn history_window_days(since: &str, until: &str) -> Result<u32> {
    let since_dt = chrono::DateTime::parse_from_rfc3339(since)
        .with_context(|| format!("invalid since timestamp: {since}"))?;
    let until_dt = chrono::DateTime::parse_from_rfc3339(until)
        .with_context(|| format!("invalid until timestamp: {until}"))?;
    anyhow::ensure!(
        until_dt > since_dt,
        "history review window must have since < until"
    );
    let seconds = (until_dt - since_dt).num_seconds();
    let days = ((seconds + 86_399) / 86_400).max(1) as u32;
    Ok(days)
}

fn resolve_history_window(
    since: Option<&str>,
    until: Option<&str>,
    days: u32,
) -> Result<(String, String)> {
    let until_dt = match until {
        Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
            .with_context(|| format!("invalid --until timestamp: {ts}"))?
            .with_timezone(&Utc),
        None => Utc::now(),
    };

    let since_dt = match since {
        Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
            .with_context(|| format!("invalid --since timestamp: {ts}"))?
            .with_timezone(&Utc),
        None => until_dt - chrono::TimeDelta::days(days as i64),
    };

    anyhow::ensure!(
        since_dt < until_dt,
        "history window must have since < until (got {since_dt} >= {until_dt})"
    );

    Ok((since_dt.to_rfc3339(), until_dt.to_rfc3339()))
}

fn get_json(client: &Client, url: &str) -> Result<Value> {
    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?;
    let resp = resp
        .error_for_status()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp
        .json()
        .with_context(|| format!("parse JSON from {url}"))?)
}

fn as_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

fn as_bool(v: &Value, key: &str) -> Option<bool> {
    v.get(key)?.as_bool()
}

fn as_string(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(ToString::to_string)
}

fn print_dhw_live_status(base_url: &str, human: bool) -> Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?;

    let hot_water = get_json(&client, &format!("{base_url}/api/hot-water"))?;
    let dhw_status = get_json(&client, &format!("{base_url}/api/dhw/status"))?;

    let remaining_litres = as_f64(&hot_water, "remaining_litres");
    let full_litres = as_f64(&hot_water, "full_litres");
    let effective_t1_c = as_f64(&hot_water, "effective_t1");
    let t1_c = as_f64(&hot_water, "t1").or_else(|| as_f64(&dhw_status, "t1_hot"));
    let hwc_storage_c =
        as_f64(&hot_water, "hwc_storage").or_else(|| as_f64(&dhw_status, "cylinder_temp"));
    let target_c = as_f64(&dhw_status, "target_temp");
    let charge_state = as_string(&hot_water, "charge_state");
    let crossover_achieved = as_bool(&hot_water, "crossover_achieved");
    let sfmode = as_string(&dhw_status, "sfmode");
    let charging = as_bool(&dhw_status, "charging");

    let safe_for_two_showers = remaining_litres.map(|l| l >= 140.0);
    let mut warnings = Vec::new();
    if t1_c.is_none() {
        warnings.push("T1 unavailable".to_string());
    }
    if remaining_litres.is_none() {
        warnings.push("remaining litres unavailable".to_string());
    }
    if matches!(sfmode.as_deref(), Some("load")) && charging != Some(true) {
        warnings.push("HwcSFMode is load while charging=false".to_string());
    }
    if let Some(t1) = t1_c {
        if t1 < 42.0 {
            warnings.push(format!("T1 is low at {:.1}°C", t1));
        }
    }

    let summary = DhwLiveSummary {
        charge_state,
        crossover_achieved,
        remaining_litres,
        full_litres,
        effective_t1_c,
        t1_c,
        hwc_storage_c,
        target_c,
        sfmode,
        charging,
        safe_for_two_showers,
        warnings,
    };

    if !human {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("DHW live status");
        println!("--------------");
        if let Some(ref s) = summary.charge_state {
            println!("charge_state: {s}");
        }
        if let Some(v) = summary.crossover_achieved {
            println!("crossover_achieved: {v}");
        }
        if let Some(v) = summary.remaining_litres {
            println!("remaining_litres: {:.1}", v);
        }
        if let Some(v) = summary.full_litres {
            println!("full_litres: {:.1}", v);
        }
        if let Some(v) = summary.effective_t1_c {
            println!("effective_t1_c: {:.1}", v);
        }
        if let Some(v) = summary.t1_c {
            println!("t1_c: {:.1}", v);
        }
        if let Some(v) = summary.hwc_storage_c {
            println!("hwc_storage_c: {:.1}", v);
        }
        if let Some(v) = summary.target_c {
            println!("target_c: {:.1}", v);
        }
        if let Some(ref v) = summary.sfmode {
            println!("sfmode: {v}");
        }
        if let Some(v) = summary.charging {
            println!("charging: {v}");
        }
        if let Some(v) = summary.safe_for_two_showers {
            println!("safe_for_two_showers: {v}");
        }
        if summary.warnings.is_empty() {
            println!("warnings: none");
        } else {
            println!("warnings:");
            for warning in &summary.warnings {
                println!("- {warning}");
            }
        }
    }

    Ok(())
}

fn resolve_time_range(cli: &Cli) -> Result<(i64, i64)> {
    let end = match &cli.to {
        Some(s) => {
            let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| anyhow::anyhow!("Invalid --to date '{}': {}", s, e))?;
            d.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp()
        }
        None => Utc::now().timestamp(),
    };
    let start = if cli.all_data {
        // Oct 22 2024 — earliest data
        1_729_555_200
    } else {
        match &cli.from {
            Some(s) => {
                let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|e| anyhow::anyhow!("Invalid --from date '{}': {}", s, e))?;
                d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
            }
            None => end - (cli.days as i64 * 86400),
        }
    };
    Ok((start, end))
}

fn format_duration(minutes: f64) -> String {
    if minutes > 1440.0 {
        format!("{:.1}d", minutes / 1440.0)
    } else if minutes > 60.0 {
        format!("{:.1}h", minutes / 60.0)
    } else {
        format!("{:.0}m", minutes)
    }
}
