//! Fetch Octopus import tariff agreements and unit rates from the account API.
//!
//! This removes hardcoded tariff prices and window times from analysis code:
//! historical and current rates, and Cosy/peak window times, are all derived
//! from the account's own agreements via the Octopus API at runtime.
//!
//! Window structure is stable for months; use `CachedTariffWindows::load_or_fetch`
//! with a long max-age (e.g. 12 hours) to avoid redundant API calls.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Timelike, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.octopus.energy/v1";
const DEFAULT_ENVRC: &str = "~/github/octopus/.envrc";

#[derive(Debug, Clone)]
pub struct TariffBook {
    intervals: Vec<RateInterval>,
    agreements: Vec<AgreementMinRate>,
}

#[derive(Debug, Clone)]
struct RateInterval {
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
    value_inc_vat: f64,
}

#[derive(Debug, Clone)]
struct AgreementMinRate {
    tariff_code: String,
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
    min_rate_inc_vat: f64,
}

// ---------------------------------------------------------------------------
// Window caching types (stable for months — refresh at most daily)
// ---------------------------------------------------------------------------

/// A tariff time window expressed in UK local time (HH:MM strings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TariffTimeWindow {
    pub start: String, // "HH:MM" UK local
    pub end: String,   // "HH:MM" UK local ("00:00" = midnight end-of-window)
}

impl TariffTimeWindow {
    pub fn start_time(&self) -> Option<NaiveTime> {
        parse_hhmm(&self.start)
    }
    pub fn end_time(&self) -> Option<NaiveTime> {
        parse_hhmm(&self.end)
    }
}

/// Cached tariff window structure derived from the Octopus account API.
///
/// Valid for months; check staleness via `fetched_at`.
/// On-disk format is JSON for easy inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTariffWindows {
    pub fetched_at: DateTime<Utc>,
    pub tariff_code: String,
    /// Cheapest-rate (Cosy) windows, sorted by start time, UK local.
    pub cosy_windows: Vec<TariffTimeWindow>,
    /// Peak-rate (highest, distinct) windows, UK local.
    /// Empty on single-rate or flat tariffs.
    pub peak_windows: Vec<TariffTimeWindow>,
}

impl CachedTariffWindows {
    /// Load from JSON cache file.  Returns `None` if missing, unparseable, or older than `max_age`.
    pub fn load(cache_path: &Path, max_age: Duration) -> Option<Self> {
        let text = std::fs::read_to_string(cache_path).ok()?;
        let cached: Self = serde_json::from_str(&text).ok()?;
        let age_secs = Utc::now()
            .signed_duration_since(cached.fetched_at)
            .num_seconds();
        if age_secs < 0 || age_secs as u64 > max_age.as_secs() {
            return None;
        }
        Some(cached)
    }

    /// Fetch fresh windows from the Octopus API.
    pub fn fetch() -> Result<Self> {
        let now = Utc::now();
        // Load a 3-day window to cover DST transitions and time-of-day edge cases.
        let book = TariffBook::load(now - chrono::Duration::days(1), now + chrono::Duration::days(2))?;
        let today = chrono::Local::now().date_naive();
        book.windows_for_local_date(today)
    }

    /// Load from cache or fetch from API if missing / stale.  Writes a fresh cache on refresh.
    pub fn load_or_fetch(cache_path: &Path, max_age: Duration) -> Result<Self> {
        if let Some(cached) = Self::load(cache_path, max_age) {
            return Ok(cached);
        }
        let fresh = Self::fetch()?;
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&fresh)
            .context("serialise CachedTariffWindows")?;
        std::fs::write(cache_path, json)
            .with_context(|| format!("write tariff window cache {}", cache_path.display()))?;
        Ok(fresh)
    }
}

#[derive(Debug, Clone)]
struct OctopusCredentials {
    api_key: String,
    account_number: String,
}

#[derive(Debug, Clone)]
struct ImportAgreement {
    tariff_code: String,
    product_code: String,
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    #[serde(default)]
    properties: Vec<AccountProperty>,
}

#[derive(Debug, Deserialize)]
struct AccountProperty {
    #[serde(default)]
    electricity_meter_points: Vec<ElectricityMeterPoint>,
}

#[derive(Debug, Deserialize)]
struct ElectricityMeterPoint {
    #[serde(default)]
    is_export: bool,
    #[serde(default)]
    agreements: Vec<AccountAgreement>,
}

#[derive(Debug, Deserialize)]
struct AccountAgreement {
    tariff_code: String,
    valid_from: String,
    valid_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RatesResponse {
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    results: Vec<UnitRate>,
}

#[derive(Debug, Deserialize)]
struct UnitRate {
    valid_from: String,
    valid_to: String,
    value_inc_vat: f64,
}

impl TariffBook {
    pub fn load(period_start: DateTime<Utc>, period_end: DateTime<Utc>) -> Result<Self> {
        if period_end <= period_start {
            bail!("invalid tariff period: {}..{}", period_start, period_end);
        }

        let creds = OctopusCredentials::load()?;
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("build Octopus HTTP client")?;

        let agreements = fetch_import_agreements(&client, &creds, period_start, period_end)?;
        if agreements.is_empty() {
            bail!(
                "no Octopus import tariff agreements overlap analysis period {}..{}",
                period_start,
                period_end
            );
        }

        let mut intervals = Vec::new();
        let mut agreement_mins = Vec::new();

        for agreement in agreements {
            let rates = fetch_unit_rates(&client, &creds, &agreement, period_start, period_end)?;
            if rates.is_empty() {
                continue;
            }
            let min_rate = rates
                .iter()
                .map(|r| r.value_inc_vat)
                .fold(f64::INFINITY, f64::min);
            agreement_mins.push(AgreementMinRate {
                tariff_code: agreement.tariff_code.clone(),
                valid_from: agreement.valid_from,
                valid_to: agreement.valid_to,
                min_rate_inc_vat: min_rate,
            });
            intervals.extend(rates);
        }

        if intervals.is_empty() {
            bail!(
                "Octopus account API returned no unit rates for analysis period {}..{}",
                period_start,
                period_end
            );
        }

        intervals.sort_by_key(|r| r.valid_from);
        intervals.dedup_by(|a, b| {
            a.valid_from == b.valid_from
                && a.valid_to == b.valid_to
                && (a.value_inc_vat - b.value_inc_vat).abs() < 1e-9
        });
        agreement_mins.sort_by_key(|a| a.valid_from);

        Ok(Self {
            intervals,
            agreements: agreement_mins,
        })
    }

    pub fn rate_at(&self, ts: DateTime<Utc>) -> Result<f64> {
        let idx = self
            .intervals
            .partition_point(|interval| interval.valid_from <= ts);
        if idx == 0 {
            bail!("no Octopus rate covering {}", ts);
        }
        let interval = &self.intervals[idx - 1];
        if ts < interval.valid_to {
            Ok(interval.value_inc_vat)
        } else {
            bail!("no Octopus rate covering {}", ts)
        }
    }

    pub fn cheapest_rate_for(&self, ts: DateTime<Utc>) -> Result<f64> {
        let idx = self
            .agreements
            .partition_point(|agreement| agreement.valid_from <= ts);
        if idx == 0 {
            bail!("no Octopus agreement covering {}", ts);
        }
        let agreement = &self.agreements[idx - 1];
        if ts < agreement.valid_to {
            Ok(agreement.min_rate_inc_vat)
        } else {
            bail!("no Octopus agreement covering {}", ts)
        }
    }

    pub fn effective_rate(&self, ts: DateTime<Utc>, battery_coverage: f64) -> Result<f64> {
        let grid_rate = self.rate_at(ts)?;
        let battery_rate = self.cheapest_rate_for(ts)?;
        Ok(battery_coverage * battery_rate + (1.0 - battery_coverage) * grid_rate)
    }

    pub fn is_lowest_rate(&self, ts: DateTime<Utc>) -> Result<bool> {
        let rate = self.rate_at(ts)?;
        let min_rate = self.cheapest_rate_for(ts)?;
        Ok((rate - min_rate).abs() < 1e-9)
    }

    pub fn min_rate(&self) -> f64 {
        self.intervals
            .iter()
            .map(|interval| interval.value_inc_vat)
            .fold(f64::INFINITY, f64::min)
    }

    pub fn max_rate(&self) -> f64 {
        self.intervals
            .iter()
            .map(|interval| interval.value_inc_vat)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    // -----------------------------------------------------------------------
    // Window extraction
    // -----------------------------------------------------------------------

    /// Derive cheap (Cosy) and peak window times for a given UK local date.
    ///
    /// Iterates over rate intervals that start on `date_local`, converts UTC
    /// boundaries to UK local time, then classifies each by rate tier:
    /// minimum rate = Cosy, maximum rate = Peak (if >1p above standard).
    pub fn windows_for_local_date(&self, date_local: NaiveDate) -> Result<CachedTariffWindows> {
        use chrono::TimeZone;

        // UTC window that generously covers the local date (allow ±25h for DST)
        let day_start_utc = chrono::Local
            .from_local_datetime(&date_local.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .ok_or_else(|| anyhow!("DST gap at start of {date_local}"))?
            .with_timezone(&Utc);
        let day_end_utc = day_start_utc + chrono::Duration::hours(25);

        // Collect (local_start, local_end, rate) for intervals whose local start falls on date_local
        let mut day_intervals: Vec<(NaiveTime, NaiveTime, f64)> = Vec::new();
        for interval in &self.intervals {
            if interval.valid_from >= day_end_utc || interval.valid_to <= day_start_utc {
                continue;
            }
            let local_from = interval.valid_from.with_timezone(&chrono::Local);
            if local_from.date_naive() != date_local {
                continue;
            }
            let local_to = interval.valid_to.with_timezone(&chrono::Local);
            day_intervals.push((
                local_from.time(),
                local_to.time(),
                interval.value_inc_vat,
            ));
        }

        if day_intervals.is_empty() {
            bail!("no tariff intervals found for local date {date_local}");
        }

        let min_rate = day_intervals.iter().map(|r| r.2).fold(f64::INFINITY, f64::min);
        let max_rate = day_intervals.iter().map(|r| r.2).fold(f64::NEG_INFINITY, f64::max);
        // Only treat max as a distinct "peak" tier when it is notably above the others
        let has_peak = max_rate - min_rate > 2.0;

        let mut cosy_windows: Vec<TariffTimeWindow> = day_intervals
            .iter()
            .filter(|r| (r.2 - min_rate).abs() < 0.01)
            .map(|(s, e, _)| TariffTimeWindow {
                start: fmt_naive_time(s),
                end: fmt_naive_time(e),
            })
            .collect();
        cosy_windows.sort_by_key(|w| w.start.clone());

        let mut peak_windows: Vec<TariffTimeWindow> = if has_peak {
            day_intervals
                .iter()
                .filter(|r| (r.2 - max_rate).abs() < 0.01)
                .map(|(s, e, _)| TariffTimeWindow {
                    start: fmt_naive_time(s),
                    end: fmt_naive_time(e),
                })
                .collect()
        } else {
            Vec::new()
        };
        peak_windows.sort_by_key(|w| w.start.clone());

        // Tariff code: find the agreement active on this date
        let noon_utc = day_start_utc + chrono::Duration::hours(12);
        let tariff_code = self
            .agreements
            .iter()
            .find(|a| a.valid_from <= noon_utc && noon_utc < a.valid_to)
            .map(|a| a.tariff_code.clone())
            .unwrap_or_default();

        Ok(CachedTariffWindows {
            fetched_at: Utc::now(),
            tariff_code,
            cosy_windows,
            peak_windows,
        })
    }

    /// Return the morning cheapest-rate window as `(start_offset, end_offset)` in minutes
    /// since 20:00 on `evening_date`.  Used by overnight.rs for schedule generation.
    ///
    /// "Morning" means the cheapest window that starts before 12:00 local on
    /// `evening_date + 1 day`.  Falls back to the classic 04:00–07:00 if none is found.
    pub fn morning_cheapest_offsets(&self, evening_date: NaiveDate) -> Result<(u32, u32)> {
        let morning_date = evening_date
            .succ_opt()
            .ok_or_else(|| anyhow!("date overflow from {evening_date}"))?;
        let windows = self.windows_for_local_date(morning_date)?;
        let noon = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let first_morning = windows
            .cosy_windows
            .iter()
            .find(|w| w.start_time().map_or(false, |t| t < noon));
        match first_morning {
            Some(w) => {
                let s = w.start_time().context("bad start time in cosy window")?;
                let e = w.end_time().context("bad end time in cosy window")?;
                Ok((naive_time_to_night_offset(s), naive_time_to_night_offset(e)))
            }
            None => {
                // Fallback: classic 04:00–07:00
                Ok((naive_time_to_night_offset(
                    NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
                ), naive_time_to_night_offset(
                    NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
                )))
            }
        }
    }
}

impl OctopusCredentials {
    fn load() -> Result<Self> {
        let envrc = expand_tilde(DEFAULT_ENVRC);
        let api_key = std::env::var("OCTOPUS_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| read_env_from_envrc(&envrc, "OCTOPUS_API_KEY").ok())
            .ok_or_else(|| {
                anyhow!(
                    "missing OCTOPUS_API_KEY; set env var or expose it via {}",
                    envrc.display()
                )
            })?;
        let account_number = std::env::var("OCTOPUS_ACCOUNT_NUMBER")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| read_env_from_envrc(&envrc, "OCTOPUS_ACCOUNT_NUMBER").ok())
            .ok_or_else(|| {
                anyhow!(
                    "missing OCTOPUS_ACCOUNT_NUMBER; set env var or expose it via {}",
                    envrc.display()
                )
            })?;
        Ok(Self {
            api_key,
            account_number,
        })
    }
}

fn fetch_import_agreements(
    client: &Client,
    creds: &OctopusCredentials,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Result<Vec<ImportAgreement>> {
    let url = format!("{API_BASE}/accounts/{}/", creds.account_number);
    let body: AccountResponse = client
        .get(&url)
        .basic_auth(&creds.api_key, Some(""))
        .send()
        .context("request Octopus account")?
        .error_for_status()
        .context("Octopus account API returned error")?
        .json()
        .context("parse Octopus account response")?;

    let mut agreements = Vec::new();
    for property in body.properties {
        for meter_point in property.electricity_meter_points {
            if meter_point.is_export {
                continue;
            }
            for agreement in meter_point.agreements {
                let valid_from = parse_api_dt(&agreement.valid_from)?;
                let valid_to = agreement
                    .valid_to
                    .as_deref()
                    .map(parse_api_dt)
                    .transpose()?
                    .unwrap_or(DateTime::<Utc>::MAX_UTC);
                if valid_to <= period_start || valid_from >= period_end {
                    continue;
                }
                let product_code = product_code_from_tariff(&agreement.tariff_code)?;
                agreements.push(ImportAgreement {
                    tariff_code: agreement.tariff_code,
                    product_code,
                    valid_from,
                    valid_to,
                });
            }
        }
    }

    agreements.sort_by_key(|agreement| agreement.valid_from);
    agreements.dedup_by(|a, b| {
        a.tariff_code == b.tariff_code
            && a.valid_from == b.valid_from
            && a.valid_to == b.valid_to
    });
    Ok(agreements)
}

fn fetch_unit_rates(
    client: &Client,
    creds: &OctopusCredentials,
    agreement: &ImportAgreement,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Result<Vec<RateInterval>> {
    let fetch_start = agreement.valid_from.max(period_start);
    let fetch_end = agreement.valid_to.min(period_end);
    if fetch_end <= fetch_start {
        return Ok(Vec::new());
    }

    let mut url = format!(
        "{API_BASE}/products/{}/electricity-tariffs/{}/standard-unit-rates/?page_size=1500&period_from={}&period_to={}",
        agreement.product_code,
        agreement.tariff_code,
        iso_utc(fetch_start),
        iso_utc(fetch_end),
    );

    let mut intervals = Vec::new();
    loop {
        let page: RatesResponse = client
            .get(&url)
            .basic_auth(&creds.api_key, Some(""))
            .send()
            .with_context(|| format!("request Octopus unit rates {url}"))?
            .error_for_status()
            .with_context(|| format!("Octopus unit-rates API returned error for {url}"))?
            .json()
            .context("parse Octopus unit-rates response")?;

        for rate in page.results {
            let valid_from = parse_api_dt(&rate.valid_from)?;
            let valid_to = parse_api_dt(&rate.valid_to)?;
            if valid_to <= fetch_start || valid_from >= fetch_end {
                continue;
            }
            intervals.push(RateInterval {
                valid_from,
                valid_to,
                value_inc_vat: rate.value_inc_vat,
            });
        }

        match page.next {
            Some(next) => url = next,
            None => break,
        }
    }

    Ok(intervals)
}

fn parse_api_dt(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("bad Octopus datetime: {value}"))
        .map(|dt| dt.with_timezone(&Utc))
}

fn product_code_from_tariff(tariff_code: &str) -> Result<String> {
    let parts: Vec<&str> = tariff_code.split('-').collect();
    if parts.len() < 4 {
        bail!("unexpected Octopus tariff code: {tariff_code}");
    }
    Ok(parts[2..parts.len() - 1].join("-"))
}

fn iso_utc(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/jack".to_string());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Format a NaiveTime as "HH:MM" (used for window start/end strings).
fn fmt_naive_time(t: &NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// Parse "HH:MM" into NaiveTime.
fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Convert a local NaiveTime (expressed on the morning after the evening date)
/// to a minute offset from 20:00 on the evening date.
/// e.g. 04:00 → 480, 07:00 → 660, 00:00 → 240.
pub fn naive_time_to_night_offset(t: NaiveTime) -> u32 {
    const NIGHT_START: u32 = 20;
    let h = t.hour();
    let m = t.minute();
    if h >= NIGHT_START {
        (h - NIGHT_START) * 60 + m
    } else {
        (24 - NIGHT_START + h) * 60 + m
    }
}

fn read_env_from_envrc(envrc: &PathBuf, name: &str) -> Result<String> {
    let command = format!(
        "set -a; . '{}' >/dev/null 2>&1; printf '%s' \"${{{name}:-}}\"",
        envrc.display()
    );
    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .output()
        .with_context(|| format!("source {}", envrc.display()))?;
    if !output.status.success() {
        bail!("failed to source {}", envrc.display());
    }
    let value = String::from_utf8(output.stdout).context("envrc value was not UTF-8")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{} not set in {}", name, envrc.display());
    }
    Ok(trimmed.to_string())
}
