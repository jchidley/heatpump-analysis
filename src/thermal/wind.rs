use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use reqwest::blocking::Client;
use serde::Deserialize;

use super::config::WindCfg;

#[derive(Debug, Deserialize)]
struct OpenMeteoHourly {
    time: Vec<String>,
    wind_speed_10m: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    hourly: OpenMeteoHourly,
}

pub(crate) fn fetch_open_meteo_wind(
    latitude: f64,
    longitude: f64,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<(DateTime<FixedOffset>, f64)> {
    let start_date = start.date_naive().to_string();
    let end_date = end.date_naive().to_string();
    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={latitude}&longitude={longitude}&start_date={start_date}&end_date={end_date}&hourly=wind_speed_10m&wind_speed_unit=ms&timezone=UTC"
    );

    let resp = match Client::new().get(&url).send() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let parsed: OpenMeteoResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    parsed
        .hourly
        .time
        .iter()
        .zip(parsed.hourly.wind_speed_10m.iter())
        .filter_map(|(t, v)| {
            let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()?;
            let utc = Utc.from_utc_datetime(&naive);
            let fixed: DateTime<FixedOffset> = utc.with_timezone(&FixedOffset::east_opt(0)?);
            Some((fixed, *v))
        })
        .collect()
}

pub(crate) fn wind_multiplier_for_window(
    wind_cfg: &WindCfg,
    wind_points: &[(DateTime<FixedOffset>, f64)],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> (f64, f64) {
    if !wind_cfg.enabled {
        return (1.0, 0.0);
    }
    let vals: Vec<f64> = wind_points
        .iter()
        .filter(|(t, _)| *t >= start && *t <= end)
        .map(|(_, v)| *v)
        .collect();

    if vals.is_empty() {
        return (1.0, 0.0);
    }

    let avg = vals.iter().sum::<f64>() / vals.len() as f64;
    let mult = (1.0 + wind_cfg.ach_per_ms * avg).clamp(0.1, wind_cfg.max_multiplier);
    (mult, avg)
}
