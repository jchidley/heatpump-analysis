use std::fs;
use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(name = "thermal-regression-check")]
#[command(about = "Compare thermal JSON artifacts against baseline with tolerance gates")]
struct Cli {
    /// Baseline artifact JSON path
    #[arg(long)]
    baseline: PathBuf,

    /// Candidate artifact JSON path
    #[arg(long)]
    candidate: PathBuf,

    /// TOML file with comparison thresholds
    #[arg(long, default_value = "artifacts/thermal/regression-thresholds.toml")]
    thresholds: PathBuf,
}

#[derive(Debug, Deserialize, Default)]
struct ThresholdConfig {
    #[serde(default)]
    global: GlobalThresholds,
    #[serde(default)]
    calibrate: CalibrateThresholds,
    #[serde(default)]
    validate: ValidateThresholds,
    #[serde(default)]
    fit_diagnostics: FitDiagnosticsThresholds,
}

#[derive(Debug, Deserialize)]
struct GlobalThresholds {
    #[serde(default = "default_true")]
    enforce_command_match: bool,
    #[serde(default = "default_true")]
    enforce_config_sha256_match: bool,
}

impl Default for GlobalThresholds {
    fn default() -> Self {
        Self {
            enforce_command_match: true,
            enforce_config_sha256_match: true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CalibrateThresholds {
    #[serde(default = "default_score_delta")]
    final_score_abs_delta_max: f64,
    #[serde(default = "default_rmse_delta")]
    rmse_night1_abs_delta_max: f64,
    #[serde(default = "default_rmse_delta")]
    rmse_night2_abs_delta_max: f64,
    #[serde(default = "default_param_delta")]
    param_abs_delta_max: f64,
}

impl Default for CalibrateThresholds {
    fn default() -> Self {
        Self {
            final_score_abs_delta_max: default_score_delta(),
            rmse_night1_abs_delta_max: default_rmse_delta(),
            rmse_night2_abs_delta_max: default_rmse_delta(),
            param_abs_delta_max: default_param_delta(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ValidateThresholds {
    #[serde(default = "default_rmse_delta")]
    aggregate_rmse_abs_delta_max: f64,
    #[serde(default = "default_bias_delta")]
    aggregate_bias_abs_delta_max: f64,
    #[serde(default = "default_within_drop")]
    aggregate_within_1c_drop_max: f64,
    #[serde(default = "default_true")]
    require_aggregate_pass: bool,
}

impl Default for ValidateThresholds {
    fn default() -> Self {
        Self {
            aggregate_rmse_abs_delta_max: default_rmse_delta(),
            aggregate_bias_abs_delta_max: default_bias_delta(),
            aggregate_within_1c_drop_max: default_within_drop(),
            require_aggregate_pass: true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FitDiagnosticsThresholds {
    #[serde(default = "default_fit_rmse_delta")]
    rmse_abs_delta_max: f64,
    #[serde(default = "default_fit_mae_delta")]
    mae_abs_delta_max: f64,
    #[serde(default = "default_fit_ratio_delta")]
    median_ratio_abs_delta_max: f64,
    #[serde(default = "default_records_drop")]
    records_count_drop_max: f64,
    #[serde(default = "default_records_drop")]
    true_cooling_count_drop_max: f64,
    #[serde(default = "default_param_delta")]
    param_abs_delta_max: f64,
}

impl Default for FitDiagnosticsThresholds {
    fn default() -> Self {
        Self {
            rmse_abs_delta_max: default_fit_rmse_delta(),
            mae_abs_delta_max: default_fit_mae_delta(),
            median_ratio_abs_delta_max: default_fit_ratio_delta(),
            records_count_drop_max: default_records_drop(),
            true_cooling_count_drop_max: default_records_drop(),
            param_abs_delta_max: default_param_delta(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_score_delta() -> f64 {
    0.05
}
fn default_rmse_delta() -> f64 {
    0.05
}
fn default_bias_delta() -> f64 {
    0.10
}
fn default_within_drop() -> f64 {
    0.03
}
fn default_param_delta() -> f64 {
    0.25
}
fn default_fit_rmse_delta() -> f64 {
    0.05
}
fn default_fit_mae_delta() -> f64 {
    0.05
}
fn default_fit_ratio_delta() -> f64 {
    0.30
}
fn default_records_drop() -> f64 {
    0.15
}

struct Gate {
    failures: usize,
}

impl Gate {
    fn new() -> Self {
        Self { failures: 0 }
    }

    fn pass(&self) -> bool {
        self.failures == 0
    }

    fn check_bool(&mut self, name: &str, ok: bool, detail: &str) {
        if ok {
            println!("PASS {:<42} {}", name, detail);
        } else {
            println!("FAIL {:<42} {}", name, detail);
            self.failures += 1;
        }
    }

    fn check_abs_delta(&mut self, name: &str, baseline: f64, candidate: f64, max_delta: f64) {
        let delta = (candidate - baseline).abs();
        let ok = delta <= max_delta;
        self.check_bool(
            name,
            ok,
            &format!(
                "baseline={:.6}, candidate={:.6}, |Δ|={:.6}, max={:.6}",
                baseline, candidate, delta, max_delta
            ),
        );
    }

    fn check_drop_fraction(
        &mut self,
        name: &str,
        baseline: usize,
        candidate: usize,
        max_drop_fraction: f64,
    ) {
        if baseline == 0 {
            self.check_bool(name, true, "baseline is 0; skip drop gate");
            return;
        }

        let drop = baseline.saturating_sub(candidate);
        let drop_fraction = drop as f64 / baseline as f64;
        let ok = drop_fraction <= max_drop_fraction;
        self.check_bool(
            name,
            ok,
            &format!(
                "baseline={}, candidate={}, drop={:.2}%, max_drop={:.2}%",
                baseline,
                candidate,
                drop_fraction * 100.0,
                max_drop_fraction * 100.0
            ),
        );
    }

    fn check_min_floor(&mut self, name: &str, baseline: f64, candidate: f64, max_drop: f64) {
        let drop = baseline - candidate;
        let ok = drop <= max_drop;
        self.check_bool(
            name,
            ok,
            &format!(
                "baseline={:.6}, candidate={:.6}, drop={:.6}, max_drop={:.6}",
                baseline, candidate, drop, max_drop
            ),
        );
    }
}

fn main() {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let thresholds_txt = fs::read_to_string(&cli.thresholds).map_err(|e| {
        format!(
            "failed to read thresholds {}: {e}",
            cli.thresholds.display()
        )
    })?;
    let thresholds: ThresholdConfig = toml::from_str(&thresholds_txt).map_err(|e| {
        format!(
            "failed to parse thresholds {}: {e}",
            cli.thresholds.display()
        )
    })?;

    let baseline: Value = read_json(&cli.baseline)?;
    let candidate: Value = read_json(&cli.candidate)?;

    println!("Thermal regression check");
    println!("  baseline : {}", cli.baseline.display());
    println!("  candidate: {}", cli.candidate.display());
    println!("  thresholds: {}", cli.thresholds.display());

    let mut gate = Gate::new();

    let base_cmd = get_str(&baseline, "/command")?;
    let cand_cmd = get_str(&candidate, "/command")?;

    if thresholds.global.enforce_command_match {
        gate.check_bool(
            "command matches",
            base_cmd == cand_cmd,
            &format!("baseline='{base_cmd}' candidate='{cand_cmd}'"),
        );
    }

    if thresholds.global.enforce_config_sha256_match {
        let b_hash = get_str(&baseline, "/config_sha256")?;
        let c_hash = get_str(&candidate, "/config_sha256")?;
        gate.check_bool(
            "config hash matches",
            b_hash == c_hash,
            &format!("baseline='{b_hash}' candidate='{c_hash}'"),
        );
    }

    match base_cmd.as_str() {
        "thermal-calibrate" => {
            compare_calibration(&baseline, &candidate, &thresholds.calibrate, &mut gate)?
        }
        "thermal-validate" => {
            compare_calibration(&baseline, &candidate, &thresholds.calibrate, &mut gate)?;
            compare_validation(&baseline, &candidate, &thresholds.validate, &mut gate)?;
        }
        "thermal-fit-diagnostics" => compare_fit_diagnostics(
            &baseline,
            &candidate,
            &thresholds.fit_diagnostics,
            &mut gate,
        )?,
        other => {
            return Err(format!(
                "unsupported artifact command '{other}' (expected thermal-calibrate, thermal-validate, thermal-fit-diagnostics)"
            ));
        }
    }

    if gate.pass() {
        println!("\nRegression gate PASSED");
        Ok(())
    } else {
        Err(format!(
            "regression gate FAILED ({} failing checks)",
            gate.failures
        ))
    }
}

fn compare_calibration(
    baseline: &Value,
    candidate: &Value,
    t: &CalibrateThresholds,
    gate: &mut Gate,
) -> Result<(), String> {
    println!("\nCalibration checks:");
    gate.check_abs_delta(
        "calibration.final_score",
        get_f64(baseline, "/calibration/final_score")?,
        get_f64(candidate, "/calibration/final_score")?,
        t.final_score_abs_delta_max,
    );
    gate.check_abs_delta(
        "calibration.rmse_night1",
        get_f64(baseline, "/calibration/rmse_night1")?,
        get_f64(candidate, "/calibration/rmse_night1")?,
        t.rmse_night1_abs_delta_max,
    );
    gate.check_abs_delta(
        "calibration.rmse_night2",
        get_f64(baseline, "/calibration/rmse_night2")?,
        get_f64(candidate, "/calibration/rmse_night2")?,
        t.rmse_night2_abs_delta_max,
    );

    for p in [
        "leather_ach",
        "landing_ach",
        "conservatory_ach",
        "office_ach",
        "doorway_cd",
    ] {
        let ptr = format!("/calibration/{p}");
        gate.check_abs_delta(
            &format!("calibration.{p}"),
            get_f64(baseline, &ptr)?,
            get_f64(candidate, &ptr)?,
            t.param_abs_delta_max,
        );
    }

    Ok(())
}

fn compare_validation(
    baseline: &Value,
    candidate: &Value,
    t: &ValidateThresholds,
    gate: &mut Gate,
) -> Result<(), String> {
    println!("\nValidation checks:");
    gate.check_abs_delta(
        "validation.aggregate_metrics.rmse",
        get_f64(baseline, "/validation/aggregate_metrics/rmse")?,
        get_f64(candidate, "/validation/aggregate_metrics/rmse")?,
        t.aggregate_rmse_abs_delta_max,
    );
    gate.check_abs_delta(
        "validation.aggregate_metrics.bias",
        get_f64(baseline, "/validation/aggregate_metrics/bias")?,
        get_f64(candidate, "/validation/aggregate_metrics/bias")?,
        t.aggregate_bias_abs_delta_max,
    );
    gate.check_min_floor(
        "validation.aggregate_metrics.within_1_0c",
        get_f64(baseline, "/validation/aggregate_metrics/within_1_0c")?,
        get_f64(candidate, "/validation/aggregate_metrics/within_1_0c")?,
        t.aggregate_within_1c_drop_max,
    );

    if t.require_aggregate_pass {
        let passed = get_bool(candidate, "/validation/aggregate_pass")?;
        gate.check_bool(
            "validation.aggregate_pass",
            passed,
            &format!("candidate aggregate_pass={passed}"),
        );
    }

    Ok(())
}

fn compare_fit_diagnostics(
    baseline: &Value,
    candidate: &Value,
    t: &FitDiagnosticsThresholds,
    gate: &mut Gate,
) -> Result<(), String> {
    println!("\nFit diagnostics checks:");
    gate.check_abs_delta(
        "summary_true_cooling.rmse",
        get_f64(baseline, "/summary_true_cooling/rmse")?,
        get_f64(candidate, "/summary_true_cooling/rmse")?,
        t.rmse_abs_delta_max,
    );
    gate.check_abs_delta(
        "summary_true_cooling.mae",
        get_f64(baseline, "/summary_true_cooling/mae")?,
        get_f64(candidate, "/summary_true_cooling/mae")?,
        t.mae_abs_delta_max,
    );

    if let (Some(b_med), Some(c_med)) = (
        get_f64_opt(baseline, "/summary_true_cooling/med_ratio"),
        get_f64_opt(candidate, "/summary_true_cooling/med_ratio"),
    ) {
        gate.check_abs_delta(
            "summary_true_cooling.med_ratio",
            b_med,
            c_med,
            t.median_ratio_abs_delta_max,
        );
    } else {
        gate.check_bool(
            "summary_true_cooling.med_ratio",
            true,
            "one or both med_ratio values are null; skipped",
        );
    }

    gate.check_drop_fraction(
        "records.count",
        get_array_len(baseline, "/records")?,
        get_array_len(candidate, "/records")?,
        t.records_count_drop_max,
    );
    gate.check_drop_fraction(
        "summary_true_cooling.n",
        get_usize(baseline, "/summary_true_cooling/n")?,
        get_usize(candidate, "/summary_true_cooling/n")?,
        t.true_cooling_count_drop_max,
    );

    for p in [
        "leather_ach",
        "landing_ach",
        "conservatory_ach",
        "office_ach",
        "doorway_cd",
    ] {
        let ptr = format!("/calibrated_params/{p}");
        gate.check_abs_delta(
            &format!("calibrated_params.{p}"),
            get_f64(baseline, &ptr)?,
            get_f64(candidate, &ptr)?,
            t.param_abs_delta_max,
        );
    }

    Ok(())
}

fn read_json(path: &PathBuf) -> Result<Value, String> {
    let txt = fs::read_to_string(path)
        .map_err(|e| format!("failed to read JSON {}: {e}", path.display()))?;
    serde_json::from_str(&txt).map_err(|e| format!("failed to parse JSON {}: {e}", path.display()))
}

fn get_value<'a>(root: &'a Value, pointer: &str) -> Result<&'a Value, String> {
    root.pointer(pointer)
        .ok_or_else(|| format!("missing JSON field: {pointer}"))
}

fn get_str(root: &Value, pointer: &str) -> Result<String, String> {
    get_value(root, pointer)?
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("field is not a string: {pointer}"))
}

fn get_bool(root: &Value, pointer: &str) -> Result<bool, String> {
    get_value(root, pointer)?
        .as_bool()
        .ok_or_else(|| format!("field is not a bool: {pointer}"))
}

fn get_f64(root: &Value, pointer: &str) -> Result<f64, String> {
    get_value(root, pointer)?
        .as_f64()
        .ok_or_else(|| format!("field is not a number: {pointer}"))
}

fn get_f64_opt(root: &Value, pointer: &str) -> Option<f64> {
    root.pointer(pointer).and_then(Value::as_f64)
}

fn get_usize(root: &Value, pointer: &str) -> Result<usize, String> {
    let n = get_value(root, pointer)?
        .as_u64()
        .ok_or_else(|| format!("field is not a non-negative integer: {pointer}"))?;
    usize::try_from(n).map_err(|_| format!("value too large for usize at {pointer}: {n}"))
}

fn get_array_len(root: &Value, pointer: &str) -> Result<usize, String> {
    get_value(root, pointer)?
        .as_array()
        .map(Vec::len)
        .ok_or_else(|| format!("field is not an array: {pointer}"))
}
