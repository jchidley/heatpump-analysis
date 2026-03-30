use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::calibration::{CalibrationResult, CalibrationSetup};
use super::config::ThermalConfig;
use super::diagnostics::{FitPeriod, FitRecord, FitSummary, PerRoomFitSummary};
use super::error::{ThermalError, ThermalResult};
use super::physics::estimate_thermal_mass;
use super::validation::{RoomResidual, ValidationSummary};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GitMeta {
    pub sha: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct CalibrationArtifact {
    pub schema_version: u32,
    pub generated_at_utc: String,
    pub command: String,
    pub config_path: String,
    pub config_sha256: String,
    pub git: GitMeta,
    pub calibration_windows: Vec<ArtifactWindow>,
    pub calibration: ArtifactCalibration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArtifactWindow {
    pub name: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArtifactCalibration {
    pub leather_ach: f64,
    pub landing_ach: f64,
    pub conservatory_ach: f64,
    pub office_ach: f64,
    pub doorway_cd: f64,
    pub rmse_night1: f64,
    pub rmse_night2: f64,
    pub base_score: f64,
    pub final_score: f64,
    pub night1: Vec<RoomResidual>,
    pub night2: Vec<RoomResidual>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FitDiagnosticsArtifact {
    pub schema_version: u32,
    pub generated_at_utc: String,
    pub command: String,
    pub config_path: String,
    pub config_sha256: String,
    pub git: GitMeta,
    pub range_start: String,
    pub range_end: String,
    pub door_state: String,
    pub cooldown_periods: Vec<FitPeriod>,
    pub records: Vec<FitRecord>,
    pub summary_all: FitSummary,
    pub summary_true_cooling: FitSummary,
    pub per_room_true_cooling: Vec<PerRoomFitSummary>,
    pub calibrated_params: ArtifactCalibrationParams,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArtifactCalibrationParams {
    pub leather_ach: f64,
    pub landing_ach: f64,
    pub conservatory_ach: f64,
    pub office_ach: f64,
    pub doorway_cd: f64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn config_sha256(cfg_txt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cfg_txt.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

pub(crate) fn git_meta() -> GitMeta {
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    GitMeta { sha, dirty }
}

pub(crate) fn build_artifact(
    command: &str,
    config_path: &Path,
    cfg_txt: &str,
    cfg: &ThermalConfig,
    setup: &CalibrationSetup,
    result: &CalibrationResult,
    validation: Option<ValidationSummary>,
) -> ThermalResult<CalibrationArtifact> {
    let thermal_masses: HashMap<String, f64> = setup
        .rooms
        .iter()
        .map(|(name, room)| {
            (
                name.clone(),
                estimate_thermal_mass(room, &setup.connections),
            )
        })
        .collect();

    let calibration = ArtifactCalibration {
        leather_ach: result.leather_ach,
        landing_ach: result.landing_ach,
        conservatory_ach: result.conservatory_ach,
        office_ach: result.office_ach,
        doorway_cd: result.doorway_cd,
        rmse_night1: result.r1,
        rmse_night2: result.r2,
        base_score: result.base_score,
        final_score: result.final_score,
        night1: super::validation::residuals_for_rooms(
            &setup.meas1,
            &result.pred1,
            None,
            &thermal_masses,
        ),
        night2: super::validation::residuals_for_rooms(
            &setup.meas2,
            &result.pred2,
            None,
            &thermal_masses,
        ),
    };

    Ok(CalibrationArtifact {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: command.to_string(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(cfg_txt),
        git: git_meta(),
        calibration_windows: vec![
            ArtifactWindow {
                name: "night1".to_string(),
                start: cfg.test_nights.night1_start.clone(),
                end: cfg.test_nights.night1_end.clone(),
            },
            ArtifactWindow {
                name: "night2".to_string(),
                start: cfg.test_nights.night2_start.clone(),
                end: cfg.test_nights.night2_end.clone(),
            },
        ],
        calibration,
        validation,
    })
}

pub(crate) fn write_artifact(
    prefix: &str,
    artifact: &CalibrationArtifact,
) -> ThermalResult<PathBuf> {
    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{}.json", prefix, ts));
    let json = serde_json::to_string_pretty(artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

pub(crate) fn write_fit_artifact(
    prefix: &str,
    artifact: &FitDiagnosticsArtifact,
) -> ThermalResult<PathBuf> {
    let dir = Path::new("artifacts").join("thermal");
    fs::create_dir_all(&dir).map_err(|source| ThermalError::ArtifactWrite {
        path: dir.display().to_string(),
        source,
    })?;
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{}.json", prefix, ts));
    let json = serde_json::to_string_pretty(artifact).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&path, json).map_err(|source| ThermalError::ArtifactWrite {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}
