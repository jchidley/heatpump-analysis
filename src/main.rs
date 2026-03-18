mod analysis;
mod emoncms;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "heatpump-analysis")]
#[command(about = "Fetch and analyse heat pump data from emoncms.org")]
struct Cli {
    /// Emoncms read API key
    #[arg(long, env = "EMONCMS_APIKEY")]
    apikey: String,

    /// How many days of history to fetch (default 7)
    #[arg(long, default_value = "7")]
    days: u32,

    /// Data interval in seconds (default 300 = 5 minutes)
    #[arg(long, default_value = "300")]
    interval: u32,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all available feeds
    Feeds,
    /// Show raw data table for the time period
    Data,
    /// Summary statistics (overall, DHW vs SH)
    Summary,
    /// COP broken down by outside temperature bands
    CopByTemp,
    /// Average profile by hour of day
    Hourly,
    /// Daily energy totals and COP from cumulative meters
    Daily,
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
            println!("{:<10} {:<25} {:<15} {:<8} {}", "ID", "Name", "Tag", "Unit", "Value");
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
        Commands::Data => {
            let df = analysis::fetch_dataframe(&client, start, end, cli.interval)?;
            let df = analysis::enrich(&df)?;
            // Show with full width
            std::env::set_var("POLARS_FMT_TABLE_ROUNDED_CORNERS", "1");
            std::env::set_var("POLARS_FMT_MAX_ROWS", "100");
            println!("{}", df);
        }
        Commands::Summary => {
            let df = analysis::fetch_dataframe(&client, start, end, cli.interval)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
        }
        Commands::CopByTemp => {
            let df = analysis::fetch_dataframe(&client, start, end, cli.interval)?;
            let df = analysis::enrich(&df)?;
            analysis::cop_by_outside_temp(&df)?;
        }
        Commands::Hourly => {
            let df = analysis::fetch_dataframe(&client, start, end, cli.interval)?;
            let df = analysis::enrich(&df)?;
            analysis::hourly_profile(&df)?;
        }
        Commands::Daily => {
            analysis::daily_energy(&client, start, end)?;
        }
        Commands::All => {
            let df = analysis::fetch_dataframe(&client, start, end, cli.interval)?;
            let df = analysis::enrich(&df)?;
            analysis::summary(&df)?;
            analysis::cop_by_outside_temp(&df)?;
            analysis::hourly_profile(&df)?;
            analysis::daily_energy(&client, start, end)?;
        }
    }

    Ok(())
}
