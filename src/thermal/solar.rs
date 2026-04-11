use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, TimeZone, Timelike, Utc};
use reqwest::blocking::Client;
use serde::Deserialize;

/// Solar position (altitude and azimuth) from date/time and location.
/// Uses simplified Spencer (1971) equations — accurate to ~1° for our purposes.
/// Returns (altitude_rad, azimuth_rad) where azimuth is clockwise from north.
pub(crate) fn solar_position(dt: DateTime<FixedOffset>, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
    let lat = lat_deg.to_radians();

    let doy = dt.ordinal() as f64;
    let hour_utc = dt.hour() as f64 + dt.minute() as f64 / 60.0;

    let b = (360.0_f64 / 365.0 * (doy - 81.0)).to_radians();
    let eot = 9.87 * (2.0 * b).sin() - 7.53 * b.cos() - 1.5 * b.sin();

    let decl = 23.45_f64.to_radians() * (360.0_f64 / 365.0 * (doy + 284.0)).to_radians().sin();

    let solar_time = hour_utc + (lon_deg / 15.0) + (eot / 60.0);
    let hour_angle = ((solar_time - 12.0) * 15.0).to_radians();

    let sin_alt = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos();
    let altitude = sin_alt.asin();

    let cos_az = if altitude.cos().abs() > 1e-10 {
        (decl.sin() - altitude.sin() * lat.sin()) / (altitude.cos() * lat.cos())
    } else {
        0.0
    };
    let mut azimuth = cos_az.clamp(-1.0, 1.0).acos();
    if hour_angle > 0.0 {
        azimuth = std::f64::consts::TAU - azimuth;
    }

    (altitude, azimuth)
}

/// Irradiance on a surface with given tilt and azimuth from DNI + DHI.
pub(crate) fn surface_irradiance(
    dni: f64,
    dhi: f64,
    solar_altitude: f64,
    solar_azimuth: f64,
    surface_tilt: f64,
    surface_azimuth: f64,
) -> f64 {
    if solar_altitude <= 0.0 {
        return 0.0;
    }

    let cos_aoi = solar_altitude.sin() * surface_tilt.cos()
        + solar_altitude.cos() * surface_tilt.sin() * (solar_azimuth - surface_azimuth).cos();
    let cos_aoi = cos_aoi.max(0.0);

    let direct = dni * cos_aoi;
    let svf = (1.0 + surface_tilt.cos()) / 2.0;
    let diffuse = dhi * svf;

    direct + diffuse
}

/// Surface azimuth constants (clockwise from north, radians).
const AZ_SW: f64 = 225.0 * std::f64::consts::PI / 180.0;
const AZ_NE: f64 = 45.0 * std::f64::consts::PI / 180.0;
const AZ_SE: f64 = 135.0 * std::f64::consts::PI / 180.0;
const TILT_VERTICAL: f64 = std::f64::consts::FRAC_PI_2;
const TILT_HORIZONTAL: f64 = 0.0;
#[allow(dead_code)]
const TILT_SLOPING_45: f64 = std::f64::consts::FRAC_PI_4;

pub(crate) struct HourlySolarIrradiance {
    pub time: DateTime<FixedOffset>,
    pub sw_vertical: f64,
    pub ne_vertical: f64,
    pub ne_horizontal: f64,
    pub se_vertical: f64,
}

/// Fetch hourly DNI + DHI from Open-Meteo and compute irradiance on SW/NE/SE surfaces.
pub(crate) fn fetch_surface_irradiance(
    latitude: f64,
    longitude: f64,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<HourlySolarIrradiance> {
    let start_date = start.date_naive().to_string();
    let end_date = end.date_naive().to_string();
    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={latitude}&longitude={longitude}&start_date={start_date}&end_date={end_date}&hourly=direct_normal_irradiance,diffuse_radiation&timezone=UTC"
    );

    let resp = match Client::new().get(&url).send() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    #[derive(Debug, Deserialize)]
    struct SolarHourly {
        time: Vec<String>,
        direct_normal_irradiance: Vec<f64>,
        diffuse_radiation: Vec<f64>,
    }
    #[derive(Debug, Deserialize)]
    struct SolarResponse {
        hourly: SolarHourly,
    }

    let parsed: SolarResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    parsed
        .hourly
        .time
        .iter()
        .zip(
            parsed
                .hourly
                .direct_normal_irradiance
                .iter()
                .zip(parsed.hourly.diffuse_radiation.iter()),
        )
        .filter_map(|(t, (dni, dhi))| {
            let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()?;
            let utc = Utc.from_utc_datetime(&naive);
            let fixed: DateTime<FixedOffset> = utc.with_timezone(&FixedOffset::east_opt(0)?);

            let (alt, az) = solar_position(fixed, latitude, longitude);

            let sw_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_SW);
            let ne_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_NE);
            let ne_h = surface_irradiance(*dni, *dhi, alt, az, TILT_HORIZONTAL, AZ_NE);
            let se_v = surface_irradiance(*dni, *dhi, alt, az, TILT_VERTICAL, AZ_SE);

            Some(HourlySolarIrradiance {
                time: fixed,
                sw_vertical: sw_v,
                ne_vertical: ne_v,
                ne_horizontal: ne_h,
                se_vertical: se_v,
            })
        })
        .collect()
}

/// Interpolate surface irradiance for a time window from hourly data.
pub(crate) fn avg_irradiance_in_window(
    solar: &[HourlySolarIrradiance],
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> (f64, f64, f64, f64) {
    let in_window: Vec<&HourlySolarIrradiance> = solar
        .iter()
        .filter(|s| s.time >= start && s.time <= end)
        .collect();

    if in_window.is_empty() {
        if let Some(nearest) = solar.iter().min_by_key(|s| {
            let mid = start + (end - start) / 2;
            (s.time - mid).num_seconds().unsigned_abs()
        }) {
            return (
                nearest.sw_vertical,
                nearest.ne_vertical,
                nearest.ne_horizontal,
                nearest.se_vertical,
            );
        }
        return (0.0, 0.0, 0.0, 0.0);
    }

    let n = in_window.len() as f64;
    (
        in_window.iter().map(|s| s.sw_vertical).sum::<f64>() / n,
        in_window.iter().map(|s| s.ne_vertical).sum::<f64>() / n,
        in_window.iter().map(|s| s.ne_horizontal).sum::<f64>() / n,
        in_window.iter().map(|s| s.se_vertical).sum::<f64>() / n,
    )
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

    // --- solar_position tests ---

    // @lat: [[tests#Solar position and irradiance helpers#Solar position varies with time and season]]
    #[test]
    fn midday_summer_gives_high_altitude() {
        // London, 21 June, solar noon (~12:00 UTC)
        let pos = solar_position(dt(2024, 6, 21, 12, 0), 51.5, 0.0);
        let alt_deg = pos.0.to_degrees();
        // Summer midday in London: altitude should be roughly 55-65 degrees
        assert!(alt_deg > 45.0, "altitude {alt_deg} should be > 45 deg at summer midday");
        assert!(alt_deg < 75.0, "altitude {alt_deg} should be < 75 deg at London latitude");
    }

    #[test]
    fn midnight_gives_negative_altitude() {
        // London, 21 June, midnight
        let pos = solar_position(dt(2024, 6, 21, 0, 0), 51.5, 0.0);
        let alt_deg = pos.0.to_degrees();
        assert!(alt_deg < 0.0, "altitude {alt_deg} should be negative at midnight");
    }

    #[test]
    fn winter_midday_lower_than_summer() {
        let summer = solar_position(dt(2024, 6, 21, 12, 0), 51.5, 0.0);
        let winter = solar_position(dt(2024, 12, 21, 12, 0), 51.5, 0.0);
        assert!(
            summer.0 > winter.0,
            "summer altitude {} should exceed winter {}",
            summer.0.to_degrees(),
            winter.0.to_degrees()
        );
    }

    #[test]
    fn azimuth_in_valid_range() {
        // Azimuth should always be in [0, 2*PI)
        for hour in [6, 9, 12, 15, 18] {
            let (_, az) = solar_position(dt(2024, 6, 21, hour, 0), 51.5, 0.0);
            assert!(az >= 0.0 && az < std::f64::consts::TAU, "azimuth {az} out of range");
        }
    }

    // --- surface_irradiance tests ---

    // @lat: [[tests#Solar position and irradiance helpers#Surface irradiance is non-negative and respects geometry]]
    #[test]
    fn zero_irradiance_gives_zero() {
        let result = surface_irradiance(0.0, 0.0, 0.5, 3.0, TILT_VERTICAL, AZ_SW);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn negative_altitude_gives_zero() {
        // Sun below horizon
        let result = surface_irradiance(500.0, 100.0, -0.1, 3.0, TILT_VERTICAL, AZ_SW);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn positive_inputs_give_positive_result() {
        let result = surface_irradiance(
            500.0,
            100.0,
            0.8,                              // ~46 deg altitude
            std::f64::consts::PI,              // south azimuth
            TILT_VERTICAL,
            AZ_SW,
        );
        assert!(result > 0.0, "irradiance {result} should be positive with positive inputs");
    }

    #[test]
    fn horizontal_surface_gets_full_diffuse() {
        // On a horizontal surface (tilt=0), SVF = 1.0, so diffuse component = dhi
        let dhi = 200.0;
        let result = surface_irradiance(0.0, dhi, 0.5, 3.0, TILT_HORIZONTAL, AZ_SW);
        assert!((result - dhi).abs() < 1e-10, "horizontal surface should get full DHI");
    }

    // --- avg_irradiance_in_window tests ---

    fn make_solar_data() -> Vec<HourlySolarIrradiance> {
        (0..6)
            .map(|h| HourlySolarIrradiance {
                time: dt(2024, 6, 21, 9 + h, 0),
                sw_vertical: 100.0 * (h as f64 + 1.0),
                ne_vertical: 50.0 * (h as f64 + 1.0),
                ne_horizontal: 200.0,
                se_vertical: 75.0,
            })
            .collect()
    }

    // @lat: [[tests#Solar position and irradiance helpers#Window irradiance averaging handles partial and empty windows]]
    #[test]
    fn avg_irradiance_full_window() {
        let data = make_solar_data();
        let (sw, ne, ne_h, se) = avg_irradiance_in_window(
            &data,
            dt(2024, 6, 21, 9, 0),
            dt(2024, 6, 21, 14, 0),
        );
        // sw: 100,200,300,400,500,600 -> mean 350
        assert!((sw - 350.0).abs() < 1e-10, "sw_vertical avg should be 350, got {sw}");
        assert!((ne - 175.0).abs() < 1e-10, "ne_vertical avg should be 175, got {ne}");
        assert!((ne_h - 200.0).abs() < 1e-10);
        assert!((se - 75.0).abs() < 1e-10);
    }

    #[test]
    fn avg_irradiance_partial_window() {
        let data = make_solar_data();
        // Window covers hours 11 and 12 (indices 2 and 3)
        let (sw, _, _, _) = avg_irradiance_in_window(
            &data,
            dt(2024, 6, 21, 11, 0),
            dt(2024, 6, 21, 12, 0),
        );
        // sw at h=11: 300, h=12: 400 -> mean 350
        assert!((sw - 350.0).abs() < 1e-10, "partial window sw avg should be 350, got {sw}");
    }

    #[test]
    fn avg_irradiance_empty_window_uses_nearest() {
        let data = make_solar_data();
        // Window entirely outside data range; nearest to midpoint should be used
        let (sw, _, _, _) = avg_irradiance_in_window(
            &data,
            dt(2024, 6, 21, 20, 0),
            dt(2024, 6, 21, 22, 0),
        );
        // Nearest to midpoint (21:00) is hour 14 (last point) -> sw = 600
        assert!((sw - 600.0).abs() < 1e-10, "should use nearest point, got {sw}");
    }

    #[test]
    fn avg_irradiance_no_data_returns_zeros() {
        let (sw, ne, ne_h, se) = avg_irradiance_in_window(
            &[],
            dt(2024, 6, 21, 9, 0),
            dt(2024, 6, 21, 14, 0),
        );
        assert_eq!(sw, 0.0);
        assert_eq!(ne, 0.0);
        assert_eq!(ne_h, 0.0);
        assert_eq!(se, 0.0);
    }
}
