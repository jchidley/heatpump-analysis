#!/usr/bin/env python3
"""Full-schema audit for canonical thermal geometry.

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
GEO_PATH = ROOT / "data" / "canonical" / "thermal_geometry.json"
PY_MODEL = ROOT / "model" / "house.py"
RS_MODEL = ROOT / "src" / "thermal.rs"
OUT_CSV = ROOT / "model" / "data" / "inventory" / "model_dimension_audit.csv"


def flatten_leaf_paths(obj: Any, prefix: str = "") -> dict[str, Any]:
    out: dict[str, Any] = {}
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "provenance":
                continue
            p = f"{prefix}.{k}" if prefix else k
            out.update(flatten_leaf_paths(v, p))
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            p = f"{prefix}[{i}]"
            out.update(flatten_leaf_paths(v, p))
    else:
        out[prefix] = obj
    return out


def main() -> None:
    geo = json.loads(GEO_PATH.read_text(encoding="utf-8"))
    provenance = geo.get("provenance", {})

    py_text = PY_MODEL.read_text(encoding="utf-8")
    rs_text = RS_MODEL.read_text(encoding="utf-8")

    rows: list[dict[str, Any]] = []

    # Wiring checks
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

    # Full-schema provenance coverage
    leaves = flatten_leaf_paths(geo)
    for path, value in sorted(leaves.items()):
        prov = provenance.get(path)
        if not isinstance(prov, dict):
            rows.append(
                {
                    "check": f"provenance:{path}",
                    "expected": "present",
                    "actual": "missing",
                    "status": "mismatch",
                }
            )
            continue

        source_type = prov.get("source_type")
        source_ref = prov.get("source_ref")
        ok = isinstance(source_type, str) and source_type and isinstance(source_ref, str) and source_ref
        rows.append(
            {
                "check": f"provenance:{path}",
                "expected": "source_type+source_ref",
                "actual": f"{source_type}|{source_ref}",
                "status": "match" if ok else "mismatch",
            }
        )

    # Extra provenance entries not mapping to geometry leaves
    for path in sorted(provenance.keys()):
        if path not in leaves:
            rows.append(
                {
                    "check": f"provenance_extra:{path}",
                    "expected": "leaf_path",
                    "actual": "extra_entry",
                    "status": "mismatch",
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
    for r in mismatches[:50]:
        print(f"  {r['check']}: actual={r['actual']} expected={r['expected']}")


if __name__ == "__main__":
    main()
