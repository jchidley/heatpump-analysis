use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ThermalError {
    #[error("failed to read thermal config {path}: {source}")]
    ConfigRead { path: String, source: std::io::Error },
    #[error("failed to parse thermal config {path}: {source}")]
    ConfigParse { path: String, source: toml::de::Error },
    #[error("failed to parse thermal geometry {path}: {source}")]
    GeometryParse { path: String, source: serde_json::Error },
    #[error("missing environment variable {0}")]
    MissingEnv(String),
    #[error("failed to parse datetime '{value}': {source}")]
    DateTimeParse {
        value: String,
        source: chrono::ParseError,
    },
    #[error("influx request failed: {0}")]
    InfluxRequest(reqwest::Error),
    #[error("failed to read influx response body: {0}")]
    InfluxResponseRead(reqwest::Error),
    #[error("influx query failed ({status}): {body}")]
    InfluxQueryFailed {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("csv parse error: {0}")]
    CsvParse(#[from] csv::Error),
    #[error("missing column '{column}' in row for {context}")]
    MissingColumn {
        column: &'static str,
        context: &'static str,
    },
    #[error("failed to parse float in {context} from '{value}'")]
    FloatParse {
        context: &'static str,
        value: String,
    },
    #[error("no outside temperature data in calibration window")]
    NoOutsideData,
    #[error("missing room '{0}'")]
    MissingRoom(&'static str),
    #[error("no calibration candidates evaluated")]
    NoCalibrationCandidates,
}

pub type ThermalResult<T> = std::result::Result<T, ThermalError>;
pub type TempSeries = HashMap<String, Vec<(DateTime<FixedOffset>, f64)>>;
pub type ScalarMap = HashMap<String, f64>;
pub type MeasuredRates = (ScalarMap, ScalarMap, f64);

pub type FitState = (
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    ScalarMap,
    ScalarMap,
);
