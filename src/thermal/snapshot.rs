use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::artifact::{config_sha256, git_meta, GitMeta};
use super::error::{ThermalError, ThermalResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotFileEntry {
    source_rel_path: String,
    snapshot_rel_path: String,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ThermalSnapshotManifest {
    schema_version: u32,
    generated_at_utc: String,
    command: String,
    signoff_reason: String,
    git: GitMeta,
    config_path: String,
    config_sha256: String,
    files: Vec<SnapshotFileEntry>,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

pub fn snapshot_export(
    config_path: &Path,
    signoff_reason: &str,
    approved_by_human: bool,
) -> ThermalResult<PathBuf> {
    if !approved_by_human {
        return Err(ThermalError::HumanApprovalRequired);
    }
    if signoff_reason.trim().is_empty() {
        return Err(ThermalError::EmptySignoffReason);
    }

    let cfg_txt = fs::read_to_string(config_path).map_err(|source| ThermalError::ConfigRead {
        path: config_path.display().to_string(),
        source,
    })?;

    let snapshot_root = Path::new("artifacts").join("thermal").join("snapshots");
    fs::create_dir_all(&snapshot_root).map_err(|source| ThermalError::ArtifactWrite {
        path: snapshot_root.display().to_string(),
        source,
    })?;

    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let out_dir = snapshot_root.join(format!("thermal-snapshot-{}", ts));
    let files_dir = out_dir.join("files");
    fs::create_dir_all(&files_dir).map_err(|source| ThermalError::ArtifactWrite {
        path: files_dir.display().to_string(),
        source,
    })?;

    let required_paths = [
        "artifacts/thermal/baselines/thermal-calibrate-baseline.json",
        "artifacts/thermal/baselines/thermal-validate-baseline.json",
        "artifacts/thermal/baselines/thermal-fit-diagnostics-baseline.json",
        "artifacts/thermal/regression-thresholds.toml",
    ];

    let mut entries = Vec::new();
    for src_rel in required_paths {
        let src = Path::new(src_rel);
        let file_name = src
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or_else(|| ThermalError::InvalidSnapshotPath(src_rel.to_string()))?;
        let dst_rel = format!("files/{file_name}");
        let dst = out_dir.join(&dst_rel);

        fs::copy(src, &dst).map_err(|source| ThermalError::SnapshotCopy {
            from: src.display().to_string(),
            to: dst.display().to_string(),
            source,
        })?;

        let sha = sha256_file(&dst)?;
        entries.push(SnapshotFileEntry {
            source_rel_path: src_rel.to_string(),
            snapshot_rel_path: dst_rel,
            sha256: sha,
        });
    }

    let cfg_copy_name = config_path
        .file_name()
        .and_then(|x| x.to_str())
        .ok_or_else(|| ThermalError::InvalidSnapshotPath(config_path.display().to_string()))?;
    let cfg_copy_rel = format!("files/{cfg_copy_name}");
    let cfg_copy_path = out_dir.join(&cfg_copy_rel);
    fs::copy(config_path, &cfg_copy_path).map_err(|source| ThermalError::SnapshotCopy {
        from: config_path.display().to_string(),
        to: cfg_copy_path.display().to_string(),
        source,
    })?;
    entries.push(SnapshotFileEntry {
        source_rel_path: config_path.display().to_string(),
        snapshot_rel_path: cfg_copy_rel,
        sha256: sha256_file(&cfg_copy_path)?,
    });

    let manifest = ThermalSnapshotManifest {
        schema_version: 1,
        generated_at_utc: Utc::now().to_rfc3339(),
        command: "thermal-snapshot-export".to_string(),
        signoff_reason: signoff_reason.trim().to_string(),
        git: git_meta(),
        config_path: config_path.display().to_string(),
        config_sha256: config_sha256(&cfg_txt),
        files: entries,
    };

    let manifest_path = out_dir.join("manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(ThermalError::ArtifactSerialize)?;
    fs::write(&manifest_path, manifest_json).map_err(|source| ThermalError::ArtifactWrite {
        path: manifest_path.display().to_string(),
        source,
    })?;

    Ok(manifest_path)
}

pub fn snapshot_import(
    manifest_path: &Path,
    signoff_reason: &str,
    approved_by_human: bool,
) -> ThermalResult<()> {
    if !approved_by_human {
        return Err(ThermalError::HumanApprovalRequired);
    }
    if signoff_reason.trim().is_empty() {
        return Err(ThermalError::EmptySignoffReason);
    }

    let manifest_txt =
        fs::read_to_string(manifest_path).map_err(|source| ThermalError::SnapshotManifestRead {
            path: manifest_path.display().to_string(),
            source,
        })?;
    let manifest: ThermalSnapshotManifest =
        serde_json::from_str(&manifest_txt).map_err(|source| {
            ThermalError::SnapshotManifestParse {
                path: manifest_path.display().to_string(),
                source,
            }
        })?;

    let root = manifest_path
        .parent()
        .ok_or_else(|| ThermalError::InvalidSnapshotPath(manifest_path.display().to_string()))?;

    for entry in manifest.files {
        let src_rel = sanitize_relative_path(&entry.snapshot_rel_path)?;
        let dst_rel = sanitize_relative_path(&entry.source_rel_path)?;

        let src = root.join(src_rel);
        let dst = Path::new(".").join(dst_rel);

        let src_sha = sha256_file(&src)?;
        if src_sha != entry.sha256 {
            return Err(ThermalError::InvalidSnapshotPath(format!(
                "sha256 mismatch for {}",
                src.display()
            )));
        }

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|source| ThermalError::ArtifactWrite {
                path: parent.display().to_string(),
                source,
            })?;
        }

        fs::copy(&src, &dst).map_err(|source| ThermalError::SnapshotCopy {
            from: src.display().to_string(),
            to: dst.display().to_string(),
            source,
        })?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_file(path: &Path) -> ThermalResult<String> {
    let bytes = fs::read(path).map_err(|source| ThermalError::ConfigRead {
        path: path.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn sanitize_relative_path(input: &str) -> ThermalResult<PathBuf> {
    let p = Path::new(input);
    if p.is_absolute() {
        return Err(ThermalError::InvalidSnapshotPath(input.to_string()));
    }

    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return Err(ThermalError::InvalidSnapshotPath(input.to_string())),
        }
    }

    if out.as_os_str().is_empty() {
        return Err(ThermalError::InvalidSnapshotPath(input.to_string()));
    }

    Ok(out)
}
