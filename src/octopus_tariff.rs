//! Fetch Octopus import tariff agreements and unit rates from the account API.
//!
//! This removes hardcoded tariff prices from analysis code: historical and
//! current rates are derived from the account's own agreements and the Octopus
//! standard-unit-rates endpoint at runtime.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::Deserialize;

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
    valid_from: DateTime<Utc>,
    valid_to: DateTime<Utc>,
    min_rate_inc_vat: f64,
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
