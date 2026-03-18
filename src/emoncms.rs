//! Emoncms API client for fetching feed data.

use anyhow::{Context, Result};
use serde::Deserialize;

const BASE_URL: &str = "https://emoncms.org";

/// Metadata for a single emoncms feed.
#[derive(Debug, Deserialize, Clone)]
pub struct Feed {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub unit: String,
    #[serde(deserialize_with = "de_option_f64")]
    pub value: Option<f64>,
}

fn de_option_f64<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<f64>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    match s {
        Some(v) => Ok(v.parse::<f64>().ok()),
        None => Ok(None),
    }
}

/// A single data point: (timestamp_ms, value).
pub type DataPoint = (i64, Option<f64>);

/// Client wrapping the emoncms read API.
pub struct Client {
    apikey: String,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(apikey: &str) -> Self {
        Self {
            apikey: apikey.to_string(),
            http: reqwest::blocking::Client::new(),
        }
    }

    /// List all feeds visible to this API key.
    pub fn list_feeds(&self) -> Result<Vec<Feed>> {
        let url = format!("{}/feed/list.json?apikey={}", BASE_URL, self.apikey);
        let resp = self.http.get(&url).send()?.text()?;
        let feeds: Vec<Feed> =
            serde_json::from_str(&resp).context("Failed to parse feed list")?;
        Ok(feeds)
    }

    /// Fetch time-series data for a feed.
    ///
    /// - `id`: feed ID
    /// - `start` / `end`: unix timestamps in **seconds**
    /// - `interval`: seconds between data points
    pub fn feed_data(
        &self,
        id: &str,
        start: i64,
        end: i64,
        interval: u32,
    ) -> Result<Vec<DataPoint>> {
        let url = format!(
            "{}/feed/data.json?apikey={}&id={}&start={}&end={}&interval={}",
            BASE_URL,
            self.apikey,
            id,
            start * 1000,
            end * 1000,
            interval,
        );
        let resp = self.http.get(&url).send()?.text()?;
        let data: Vec<DataPoint> =
            serde_json::from_str(&resp).context("Failed to parse feed data")?;
        Ok(data)
    }
}
