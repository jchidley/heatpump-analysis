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
