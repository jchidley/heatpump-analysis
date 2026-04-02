mod analysis;
mod config;
mod db;
mod emoncms;
mod gaps;
mod octopus;
mod overnight;
mod thermal;

use std::path::PathBuf;

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand};
use polars::prelude::SerWriter;

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
