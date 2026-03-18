//! SQLite storage for heat pump data.
//!
//! Stores 1-minute resolution data locally so analysis doesn't hit the API.
//! Supports incremental sync — only fetches data newer than what's already stored.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::emoncms::{Client, Feed};

/// All feed IDs we want to download.
const FEED_IDS: &[(&str, &str)] = &[
    ("503093", "outside_temp"),
    ("503094", "elec_power"),
    ("503095", "elec_energy"),
    ("503096", "heat_power"),
    ("503097", "heat_energy"),
    ("503098", "flow_temp"),
    ("503099", "return_temp"),
    ("503100", "flow_rate"),
    ("503101", "indoor_temp"),
    ("503102", "humidity"),
    ("503103", "battery"),
    ("512889", "dhw_flag"),
];

/// Open (or create) the SQLite database and ensure schema exists.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;

    // WAL mode for better concurrent read performance
    conn.pragma_update(None, "journal_mode", "WAL")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS feeds (
            id        TEXT PRIMARY KEY,
            name      TEXT NOT NULL,
            tag       TEXT NOT NULL DEFAULT '',
            unit      TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS samples (
            feed_id   TEXT NOT NULL,
            timestamp INTEGER NOT NULL,  -- unix ms
            value     REAL,
            PRIMARY KEY (feed_id, timestamp)
        ) WITHOUT ROWID;

        CREATE TABLE IF NOT EXISTS sync_state (
            feed_id       TEXT PRIMARY KEY,
            last_timestamp INTEGER NOT NULL DEFAULT 0  -- unix ms
        );
        ",
    )?;

    Ok(conn)
}

/// Sync feed metadata from the API.
pub fn sync_feeds(conn: &Connection, client: &Client) -> Result<Vec<Feed>> {
    let feeds = client.list_feeds()?;

    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO feeds (id, name, tag, unit) VALUES (?1, ?2, ?3, ?4)",
    )?;

    for f in &feeds {
        stmt.execute(params![f.id, f.name, f.tag, f.unit])?;
    }

    Ok(feeds)
}

/// Get the last synced timestamp (unix ms) for a feed, or 0 if never synced.
fn last_synced(conn: &Connection, feed_id: &str) -> Result<i64> {
    let ts: i64 = conn
        .query_row(
            "SELECT COALESCE(
                (SELECT last_timestamp FROM sync_state WHERE feed_id = ?1),
                0
            )",
            params![feed_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(ts)
}

/// Update the sync state for a feed.
fn update_sync_state(conn: &Connection, feed_id: &str, last_ts: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_state (feed_id, last_timestamp) VALUES (?1, ?2)",
        params![feed_id, last_ts],
    )?;
    Ok(())
}

/// Default sync start: 2024-10-22 00:00 UTC (when this installation began logging).
const DEFAULT_SYNC_START_MS: i64 = 1_729_555_200_000;

/// Download all data for all feeds, storing in SQLite.
///
/// - Fetches in 7-day chunks at 1-minute (60s) resolution.
/// - Only fetches data newer than the last sync point.
/// - Skips null values to save space.
pub fn sync_all(conn: &Connection, client: &Client) -> Result<SyncStats> {
    let now_ms = chrono::Utc::now().timestamp() * 1000;
    let mut stats = SyncStats::default();

    for (feed_id, feed_name) in FEED_IDS {
        let last_ts = last_synced(conn, feed_id)?;

        // Start from last synced point (or from the beginning if never synced).
        let start_ms = if last_ts > 0 {
            last_ts
        } else {
            DEFAULT_SYNC_START_MS
        };

        if start_ms >= now_ms {
            eprintln!("  {:<15} up to date", feed_name);
            continue;
        }

        let total_days = (now_ms - start_ms) as f64 / 86_400_000.0;
        eprintln!(
            "  {:<15} syncing {:.0} days from {}...",
            feed_name,
            total_days,
            format_ts(start_ms),
        );

        let chunk_ms: i64 = 7 * 86_400 * 1000; // 7 days
        let mut chunk_start = start_ms;
        let mut feed_inserted = 0u64;
        let mut max_ts = last_ts;

        while chunk_start < now_ms {
            let chunk_end = (chunk_start + chunk_ms).min(now_ms);

            let data = client.feed_data(
                feed_id,
                chunk_start / 1000,
                chunk_end / 1000,
                60, // 1-minute resolution
            )?;

            // Batch insert
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR IGNORE INTO samples (feed_id, timestamp, value)
                     VALUES (?1, ?2, ?3)",
                )?;

                for (ts, val) in &data {
                    if let Some(v) = val {
                        stmt.execute(params![feed_id, ts, v])?;
                        feed_inserted += 1;
                        if *ts > max_ts {
                            max_ts = *ts;
                        }
                    }
                }
            }
            tx.commit()?;

            chunk_start = chunk_end;

            // Brief pause to be polite to the API
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if max_ts > last_ts {
            update_sync_state(conn, feed_id, max_ts)?;
        }

        eprintln!(
            "  {:<15} inserted {} samples (up to {})",
            feed_name,
            feed_inserted,
            format_ts(max_ts),
        );

        stats.feeds_synced += 1;
        stats.samples_inserted += feed_inserted;
    }

    // Report database size
    stats.total_samples = conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))?;
    stats.db_size_bytes = conn
        .query_row("SELECT page_count * page_size FROM pragma_page_count, pragma_page_size", [], |r| r.get(0))
        .unwrap_or(0);

    Ok(stats)
}

/// Build a Polars DataFrame from the local database for the given time range.
///
/// This replaces `fetch_dataframe` when data is available locally.
/// If `include_simulated` is true, gap-filled data is merged in and an
/// `is_simulated` boolean column is added.
pub fn load_dataframe(
    conn: &Connection,
    start: i64,
    end: i64,
) -> Result<polars::prelude::DataFrame> {
    load_dataframe_inner(conn, start, end, false)
}

/// Load data, optionally including simulated gap-fill samples.
pub fn load_dataframe_with_simulated(
    conn: &Connection,
    start: i64,
    end: i64,
) -> Result<polars::prelude::DataFrame> {
    load_dataframe_inner(conn, start, end, true)
}

fn load_dataframe_inner(
    conn: &Connection,
    start: i64,
    end: i64,
    include_simulated: bool,
) -> Result<polars::prelude::DataFrame> {
    use polars::prelude::*;

    let start_ms = start * 1000;
    let end_ms = end * 1000;

    // The feeds we need for analysis, mapped to column names
    let feed_cols: &[(&str, &str)] = &[
        ("503094", "elec_w"),
        ("503096", "heat_w"),
        ("503098", "flow_t"),
        ("503099", "return_t"),
        ("503100", "flow_rate"),
        ("503093", "outside_t"),
        ("503101", "indoor_t"),
    ];

    // Get all timestamps in range from real data
    let mut stmt = conn.prepare(
        "SELECT DISTINCT timestamp FROM samples
         WHERE timestamp >= ?1 AND timestamp < ?2
         ORDER BY timestamp",
    )?;
    let mut timestamps: Vec<i64> = stmt
        .query_map(params![start_ms, end_ms], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Track which timestamps are simulated
    let mut simulated_ts: std::collections::HashSet<i64> = std::collections::HashSet::new();

    if include_simulated {
        // Check if simulated_samples table exists
        let has_sim: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='simulated_samples'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if has_sim {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT timestamp FROM simulated_samples
                 WHERE timestamp >= ?1 AND timestamp < ?2
                   AND timestamp NOT IN (
                       SELECT timestamp FROM samples
                       WHERE feed_id = '503094' AND timestamp >= ?1 AND timestamp < ?2
                   )",
            )?;
            let sim_ts: Vec<i64> = stmt
                .query_map(params![start_ms, end_ms], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            for ts in &sim_ts {
                simulated_ts.insert(*ts);
            }
            timestamps.extend(sim_ts);
            timestamps.sort_unstable();
            timestamps.dedup();
        }
    }

    if timestamps.is_empty() {
        anyhow::bail!("No data found in database for the requested time range");
    }

    eprintln!(
        "Loading {} timestamps from {} to {}",
        timestamps.len(),
        format_ts(timestamps[0]),
        format_ts(*timestamps.last().unwrap()),
    );

    // Build timestamp -> index map for fast lookups
    let ts_index: std::collections::HashMap<i64, usize> = timestamps
        .iter()
        .enumerate()
        .map(|(i, &ts)| (ts, i))
        .collect();

    let n = timestamps.len();

    // Create timestamp column
    let dt_series = Series::new("timestamp".into(), &timestamps)
        .cast(&DataType::Datetime(TimeUnit::Milliseconds, Some("UTC".into())))
        .context("Failed to create datetime column")?;

    let mut columns: Vec<Column> = vec![dt_series.into()];

    // Load each feed
    for (feed_id, col_name) in feed_cols {
        let mut values: Vec<Option<f64>> = vec![None; n];

        // Real data
        let mut stmt = conn.prepare(
            "SELECT timestamp, value FROM samples
             WHERE feed_id = ?1 AND timestamp >= ?2 AND timestamp < ?3",
        )?;

        let rows = stmt.query_map(params![feed_id, start_ms, end_ms], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?;

        for row in rows {
            let (ts, val) = row?;
            if let Some(&idx) = ts_index.get(&ts) {
                values[idx] = Some(val);
            }
        }

        // Simulated data (only fills gaps — doesn't overwrite real data)
        if include_simulated {
            let has_sim: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name='simulated_samples'",
                    [],
                    |row| row.get::<_, u32>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);

            if has_sim {
                let mut stmt = conn.prepare(
                    "SELECT timestamp, value FROM simulated_samples
                     WHERE feed_id = ?1 AND timestamp >= ?2 AND timestamp < ?3",
                )?;

                let rows = stmt.query_map(params![feed_id, start_ms, end_ms], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                })?;

                for row in rows {
                    let (ts, val) = row?;
                    if let Some(&idx) = ts_index.get(&ts) {
                        // Only fill if no real data exists
                        if values[idx].is_none() {
                            values[idx] = Some(val);
                        }
                    }
                }
            }
        }

        columns.push(Series::new((*col_name).into(), &values).into());
    }

    // Add is_simulated column
    if include_simulated {
        let sim_flags: Vec<bool> = timestamps
            .iter()
            .map(|ts| simulated_ts.contains(ts))
            .collect();
        columns.push(Series::new("is_simulated".into(), &sim_flags).into());
    }

    let df = polars::prelude::DataFrame::new(columns).context("Failed to build DataFrame")?;
    Ok(df)
}

/// Load daily cumulative energy data from the database.
pub fn load_daily_energy(
    conn: &Connection,
    start: i64,
    end: i64,
) -> Result<(Vec<(i64, Option<f64>)>, Vec<(i64, Option<f64>)>)> {
    let start_ms = start * 1000;
    let end_ms = end * 1000;

    // For cumulative feeds we want one sample per day.
    // Pick the last sample of each day.
    let load_daily = |feed_id: &str| -> Result<Vec<(i64, Option<f64>)>> {
        let mut stmt = conn.prepare(
            "SELECT timestamp, value FROM samples
             WHERE feed_id = ?1 AND timestamp >= ?2 AND timestamp < ?3
             ORDER BY timestamp",
        )?;

        let all: Vec<(i64, f64)> = stmt
            .query_map(params![feed_id, start_ms, end_ms], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Group by day, take last value
        let mut daily: Vec<(i64, Option<f64>)> = Vec::new();
        let mut current_day: Option<i64> = None;
        let mut last_val: Option<f64> = None;
        let mut last_ts: i64 = 0;

        for (ts, val) in &all {
            let day = ts / 86_400_000;
            match current_day {
                Some(cd) if cd == day => {
                    last_val = Some(*val);
                    last_ts = *ts;
                }
                _ => {
                    if let Some(_cd) = current_day {
                        daily.push((last_ts, last_val));
                    }
                    current_day = Some(day);
                    last_val = Some(*val);
                    last_ts = *ts;
                }
            }
        }
        if current_day.is_some() {
            daily.push((last_ts, last_val));
        }

        Ok(daily)
    };

    let elec = load_daily("503095")?;
    let heat = load_daily("503097")?;

    Ok((elec, heat))
}

/// Load daily outside temperature statistics from the database.
///
/// Returns (date_string, mean_temp, min_temp, max_temp) for each day.
pub fn load_daily_outside_temp(
    conn: &Connection,
    start: i64,
    end: i64,
) -> Result<Vec<(String, f64, f64, f64)>> {
    let start_ms = start * 1000;
    let end_ms = end * 1000;

    let mut stmt = conn.prepare(
        "SELECT date(timestamp/1000, 'unixepoch') AS day,
                AVG(value) AS avg_t,
                MIN(value) AS min_t,
                MAX(value) AS max_t
         FROM samples
         WHERE feed_id = '503093'
           AND timestamp >= ?1 AND timestamp < ?2
         GROUP BY day
         ORDER BY day",
    )?;

    let rows = stmt
        .query_map(params![start_ms, end_ms], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

fn format_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[derive(Default)]
pub struct SyncStats {
    pub feeds_synced: u32,
    pub samples_inserted: u64,
    pub total_samples: u64,
    pub db_size_bytes: u64,
}

impl std::fmt::Display for SyncStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Synced {} feeds, inserted {} new samples\n\
             Database: {} total samples, {:.1} MB",
            self.feeds_synced,
            self.samples_inserted,
            self.total_samples,
            self.db_size_bytes as f64 / 1_048_576.0,
        )
    }
}
