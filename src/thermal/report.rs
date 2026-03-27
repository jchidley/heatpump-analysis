use std::collections::{HashMap, HashSet};

pub fn print_table(title: &str, measured: &HashMap<String, f64>, pred: &HashMap<String, f64>) {
    println!("\n{}", title);
    println!(
        "{:<14} {:>8} {:>8} {:>6} {:>8}",
        "Room", "Measured", "Pred", "Ratio", "Err"
    );
    println!("{}", "─".repeat(50));

    let mut keys: Vec<_> = measured.keys().cloned().collect();
    keys.sort();
    for room in keys {
        let m = measured[&room];
        let p = pred.get(&room).copied().unwrap_or(f64::NAN);
        let ratio = if m.abs() > 1e-9 { p / m } else { 0.0 };
        let err = p - m;
        println!(
            "{:<14} {:>8.3} {:>8.3} {:>6.2} {:>+8.3}",
            room, m, p, ratio, err
        );
    }
}

pub fn rmse(
    measured: &HashMap<String, f64>,
    predicted: &HashMap<String, f64>,
    exclude: &HashSet<String>,
) -> f64 {
    let mut errs = Vec::new();
    for (room, m) in measured {
        if exclude.contains(room) {
            continue;
        }
        if let Some(p) = predicted.get(room) {
            errs.push((m - p).powi(2));
        }
    }
    if errs.is_empty() {
        999.0
    } else {
        (errs.iter().sum::<f64>() / errs.len() as f64).sqrt()
    }
}
