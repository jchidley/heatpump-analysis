//! Octopus Energy tariff rate lookup and window discovery.
//!
//! Re-exports the shared `octopus-tariff` crate (`~/github/octopus-tariff`).
//! All implementation lives in that crate; this module exists so that
//! `crate::octopus_tariff::*` paths remain stable inside heatpump-analysis.

#[allow(unused_imports)]
pub use octopus_tariff::{
    format_windows, naive_time_to_night_offset, AgreementMinRate, CachedTariffWindows,
    OctopusCredentials, RateInterval, TariffBook, TariffTimeWindow,
};
