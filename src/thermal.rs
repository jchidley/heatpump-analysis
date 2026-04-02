#![forbid(unsafe_code)]

mod artifact;
mod calibration;
mod config;
mod dhw_sessions;
mod diagnostics;
mod display;
mod error;
mod geometry;
mod influx;
mod operational;
mod physics;
mod report;
mod snapshot;
mod solar;
mod validation;
mod wind;

// Re-export the public entry points consumed by main.rs
pub use calibration::calibrate;
pub use diagnostics::fit_diagnostics;
pub use display::{
    generate_control_table, print_analyse, print_connections, print_equilibrium,
    print_moisture, print_rooms,
};
pub use operational::operational_validate;
pub use snapshot::{snapshot_export, snapshot_import};
pub use dhw_sessions::{dhw_sessions, DhwSessionsOutput};
pub use validation::validate;
