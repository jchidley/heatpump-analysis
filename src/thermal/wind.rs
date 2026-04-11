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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn dt(year: i32, month: u32, day: u32, hour: u32, min: u32) -> DateTime<FixedOffset> {
        let naive = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            NaiveTime::from_hms_opt(hour, min, 0).unwrap(),
        );
        utc().from_utc_datetime(&naive)
    }

    fn enabled_cfg(ach_per_ms: f64) -> WindCfg {
        WindCfg {
            enabled: true,
            latitude: 51.5,
            longitude: 0.0,
            ach_per_ms,
            max_multiplier: 2.5,
        }
    }

    fn disabled_cfg() -> WindCfg {
        WindCfg {
            enabled: false,
            latitude: 51.5,
            longitude: 0.0,
            ach_per_ms: 0.1,
            max_multiplier: 2.5,
        }
    }

    fn wind_points(speeds: &[f64]) -> Vec<(DateTime<FixedOffset>, f64)> {
        speeds
            .iter()
            .enumerate()
            .map(|(i, &v)| (dt(2024, 1, 15, 10 + i as u32, 0), v))
            .collect()
    }

    // --- wind_multiplier_for_window tests ---

    #[test]
    fn disabled_returns_one() {
        let points = wind_points(&[5.0, 10.0]);
        let (mult, avg) = wind_multiplier_for_window(
            &disabled_cfg(),
            &points,
            dt(2024, 1, 15, 10, 0),
            dt(2024, 1, 15, 11, 0),
        );
        assert_eq!(mult, 1.0);
        assert_eq!(avg, 0.0);
    }

    #[test]
    fn no_wind_gives_multiplier_one() {
        let points = wind_points(&[0.0, 0.0, 0.0]);
        let (mult, avg) = wind_multiplier_for_window(
            &enabled_cfg(0.1),
            &points,
            dt(2024, 1, 15, 10, 0),
            dt(2024, 1, 15, 12, 0),
        );
        assert!((mult - 1.0).abs() < 1e-10, "zero wind should give mult=1.0, got {mult}");
        assert!((avg).abs() < 1e-10);
    }

    #[test]
    fn high_wind_gives_higher_multiplier() {
        let low_wind = wind_points(&[2.0, 2.0]);
        let high_wind = wind_points(&[10.0, 10.0]);
        let cfg = enabled_cfg(0.1);
        let window = (dt(2024, 1, 15, 10, 0), dt(2024, 1, 15, 11, 0));

        let (mult_low, _) = wind_multiplier_for_window(&cfg, &low_wind, window.0, window.1);
        let (mult_high, _) = wind_multiplier_for_window(&cfg, &high_wind, window.0, window.1);

        assert!(
            mult_high > mult_low,
            "high wind mult {mult_high} should exceed low wind mult {mult_low}"
        );
    }

    #[test]
    fn multiplier_clamped_to_max() {
        // Extreme wind: 100 m/s * 0.1 ach_per_ms -> raw mult = 11.0, clamped to 2.5
        let points = wind_points(&[100.0, 100.0]);
        let cfg = enabled_cfg(0.1);
        let (mult, _) = wind_multiplier_for_window(
            &cfg,
            &points,
            dt(2024, 1, 15, 10, 0),
            dt(2024, 1, 15, 11, 0),
        );
        assert!((mult - 2.5).abs() < 1e-10, "mult should be clamped to 2.5, got {mult}");
    }

    #[test]
    fn empty_window_returns_one() {
        let points = wind_points(&[5.0, 5.0]);
        // Window entirely outside data range
        let (mult, avg) = wind_multiplier_for_window(
            &enabled_cfg(0.1),
            &points,
            dt(2024, 1, 15, 20, 0),
            dt(2024, 1, 15, 22, 0),
        );
        assert_eq!(mult, 1.0);
        assert_eq!(avg, 0.0);
    }

    #[test]
    fn correct_avg_wind_speed() {
        let points = wind_points(&[4.0, 6.0, 8.0]);
        let (_, avg) = wind_multiplier_for_window(
            &enabled_cfg(0.1),
            &points,
            dt(2024, 1, 15, 10, 0),
            dt(2024, 1, 15, 12, 0),
        );
        assert!((avg - 6.0).abs() < 1e-10, "avg should be 6.0, got {avg}");
    }

    // @lat: [[tests#Thermal physics primitives#Wind multiplier is monotonic in wind speed]]
    #[test]
    fn wind_multiplier_is_monotonic_in_wind_speed() {
        let cfg = enabled_cfg(0.1);
        let window = (dt(2024, 1, 15, 10, 0), dt(2024, 1, 15, 11, 0));
        let mut prev_mult = 0.0;
        for speed in [0.0, 1.0, 2.0, 5.0, 10.0, 15.0, 20.0] {
            let points = wind_points(&[speed, speed]);
            let (mult, _) = wind_multiplier_for_window(&cfg, &points, window.0, window.1);
            assert!(
                mult >= prev_mult,
                "multiplier should be monotonic: speed={speed}, mult={mult}, prev={prev_mult}"
            );
            prev_mult = mult;
        }
    }
}
