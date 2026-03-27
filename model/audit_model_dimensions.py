#!/usr/bin/env python3
"""Audit canonical geometry usage and key dimensional consistency.

Usage:
  uv run python model/audit_model_dimensions.py

Outputs:
  model/data/inventory/model_dimension_audit.csv
"""

from __future__ import annotations

import csv
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
INV_PATH = ROOT / "model" / "data" / "inventory" / "house_inventory.json"
GEO_PATH = ROOT / "data" / "canonical" / "thermal_geometry.json"
PY_MODEL = ROOT / "model" / "house.py"
RS_MODEL = ROOT / "src" / "thermal.rs"
OUT_CSV = ROOT / "model" / "data" / "inventory" / "model_dimension_audit.csv"


def room_lookup(geo: dict[str, Any], room: str) -> dict[str, Any]:
    for r in geo["rooms"]:
        if r["name"] == room:
            return r
    return {}


def fabric_area(room: dict[str, Any], label_contains: str) -> float | None:
    for e in room.get("external_fabric", []):
        if label_contains.lower() in e["description"].lower():
            return float(e["area"])
    return None


def inv_room(inv: dict[str, Any], room: str) -> dict[str, Any]:
    for r in inv.get("room_dimension_summary", []):
        if r.get("room") == room:
            return r
    return {}


def main() -> None:
    inv = json.loads(INV_PATH.read_text())
    geo = json.loads(GEO_PATH.read_text())

    py_text = PY_MODEL.read_text(encoding="utf-8")
    rs_text = RS_MODEL.read_text(encoding="utf-8")

    rows: list[dict[str, Any]] = []

    # 1) Model wiring checks (no magic dims in source; geometry loaded from file)
    rows.append(
        {
            "check": "python_uses_canonical_geometry_file",
            "expected": "data/canonical/thermal_geometry.json",
            "actual": "present" if "thermal_geometry.json" in py_text else "missing",
            "status": "match" if "thermal_geometry.json" in py_text else "mismatch",
        }
    )
    rows.append(
        {
            "check": "rust_uses_canonical_geometry_file",
            "expected": "data/canonical/thermal_geometry.json",
            "actual": "present" if "thermal_geometry.json" in rs_text else "missing",
            "status": "match" if "thermal_geometry.json" in rs_text else "mismatch",
        }
    )

    # 2) Key metric consistency: canonical geometry vs inventory/xlsx-derived canonical values
    checks = [
        ("Conservatory", "floor_area_m2", room_lookup(geo, "conservatory").get("floor_area")),
        ("Conservatory", "external_wall_area_m2", fabric_area(room_lookup(geo, "conservatory"), "External Wall")),
        ("Conservatory", "roof_area_m2", fabric_area(room_lookup(geo, "conservatory"), "Roof")),
        ("Conservatory", "window_area_m2", fabric_area(room_lookup(geo, "conservatory"), "Windows")),
        ("Elvina", "window_area_m2", fabric_area(room_lookup(geo, "elvina"), "Windows")),
        ("Elvina", "velux_area_m2", fabric_area(room_lookup(geo, "elvina"), "Velux")),
        ("Aldora", "window_area_m2", fabric_area(room_lookup(geo, "aldora"), "Windows")),
        ("Shower", "window_area_m2", fabric_area(room_lookup(geo, "shower"), "Windows")),
    ]

    # XLSX overrides in inventory for changed windows
    cross = {r["metric"]: r for r in inv.get("dimension_cross_reference", [])}
    override = {
        "Aldora": cross.get("aldora_window_area_total", {}).get("canonical_value"),
        "Shower": cross.get("shower_window_area_total", {}).get("canonical_value"),
    }

    for room, metric, geo_val in checks:
        inv_val = inv_room(inv, room).get(metric)
        if room in override and override[room] is not None and metric == "window_area_m2":
            inv_val = override[room]

        status = "missing"
        if isinstance(geo_val, (int, float)) and isinstance(inv_val, (int, float)):
            status = "match" if abs(float(geo_val) - float(inv_val)) <= 1e-6 else "mismatch"

        rows.append(
            {
                "check": f"{room.lower()}_{metric}",
                "expected": inv_val,
                "actual": geo_val,
                "status": status,
            }
        )

    OUT_CSV.parent.mkdir(parents=True, exist_ok=True)
    with OUT_CSV.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["check", "expected", "actual", "status"])
        w.writeheader()
        w.writerows(rows)

    mismatches = [r for r in rows if r["status"] != "match"]
    print(f"Wrote {OUT_CSV}")
    print(f"Checks: {len(rows)}")
    print(f"Mismatches: {len(mismatches)}")
    for r in mismatches:
        print(f"  {r['check']}: actual={r['actual']} expected={r['expected']} ({r['status']})")


if __name__ == "__main__":
    main()
