#!/bin/bash
# backup-sdcard.sh — Back up an SD card to a PiShrink'd xz-compressed image
#
# Usage: sudo ./backup-sdcard.sh /dev/sdX /path/to/output-name
#
# Produces: /path/to/output-name.img.xz (+ .partition-table.dump)
#
# Restore:
#   xz -dc output-name.img.xz | sudo dd of=/dev/sdX bs=4M status=progress

set -euo pipefail

DEVICE="${1:?Usage: $0 /dev/sdX /path/to/output-name}"
OUTPUT="${2:?Usage: $0 /dev/sdX /path/to/output-name}"

mkdir -p "$(dirname "$OUTPUT")"

echo "[1/3] Saving partition table..."
sfdisk -d "$DEVICE" > "${OUTPUT}.partition-table.dump"

echo "[2/3] Imaging ${DEVICE}..."
dd if="$DEVICE" bs=4M conv=sparse of="${OUTPUT}.img" status=noxfer

echo "[3/3] PiShrink + xz..."
pishrink.sh -Za "${OUTPUT}.img"

echo "=== Done ==="
ls -lh "${OUTPUT}.img.xz" "${OUTPUT}.partition-table.dump"
