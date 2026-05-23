# Reviews

Historical review snapshots live here when a newer review replaces them in [[plan]].

## 2026-05-02 14:52 BST

The previous plan refresh timestamp was **2026-05-02 14:52 BST**.

## Latest Data Review

Most recent controller-log and PostgreSQL review for the open plan items.

Review window: **2026-04-30 09:14 BST** to **2026-05-02 14:52 BST**.

Evidence:
- `adaptive-heating-mvp` remained on PID **2575348** after the **2026-04-30 09:53 BST** start (`NRestarts=0`) and wrote PostgreSQL decisions from **2026-04-30 09:14:12 BST** through **2026-05-02 14:41:11 BST**.
- PostgreSQL recorded **206** controller decisions: `hold=132`, `overnight_coast=42`, `dhw_active=21`, `daytime_model=8`, `overnight_model=2`, and `dhw_schedule_launch=1`. Leather ranged **20.7-22.4°C**, Aldora **20.4-25.0°C**, and outside **11.94-31.31°C**.
- Forecast fetching was clean in the logs: **40** refreshes and **0** fetch failures, but **52/206** decision rows still had null forecast fields, mainly during the eBUS telemetry outage rather than an upstream weather outage.
- Cosy headroom is still misleading during active charging: **11** Cosy rows had `battery_power_w < -500W`, with charging down to **-5.0kW** and headroom still reported between **0.5 and 10.8 kWh**.
- Overnight evidence was mild rather than cold: **42** coast rows and **2** overnight model rows occurred around **11.9-14.4°C**, so they help warm-night validation but do not close the cold-coast question.
- Host-side/eBUS I/O still looked noisy after **2026-05-02 06:31 BST**: logs show **56** eBUS-style `SYN received` / `read timeout` / `no signal` errors, no EAGAINs, one **25m51s** decision gap, and many controller rows with null telemetry/model fields. Cross-repo evidence in `energy-hub` explains part of the window as a real pi5data Wi-Fi outage on **2026-05-01 03:59-07:30 BST**, not a controller algorithm fault.
- Warm/sunny curve saturation looks improved after the fix: there were **0** warm high-curve rows by the previous threshold, only **1** high-curve row overall, and model-required curves stayed at or below **1.14** despite outside readings up to **31.31°C**.
- DHW scheduling wrote **0** raw `00:00` timer values and `multical` was fresh at **2026-05-02 14:52 BST**, but the single `dhw_schedule_launch` failed to write `HwcSFMode=load` and **3** timer writes failed while telemetry was noisy. `energy-hub` confirms the DHW zero/storage anomaly was operator-caused: the DHW connection had been pulled out, so that interval is invalid input data rather than a new unexplained software fault.

Status changes:
- **Headroom unreliable during Cosy:** still **Open**; active Cosy charging again reported apparently usable headroom.
- **Overnight data growth:** still **Progressing**; warm/mild overnight rows accumulated, but no clean cold window arrived.
- **Host-side I/O hang:** still **Open**; the review found noisy eBUS/telemetry rows, but `energy-hub` now explains the larger monitoring interruption as a real Wi-Fi outage plus an operator-caused DHW disconnect, so treat the affected rows as outage-invalidated evidence rather than a fresh controller regression.
- **Forecast API reliability:** still **Open**, but this window points away from Open-Meteo as the primary cause because fetches succeeded and null forecasts correlated with local telemetry/model loss during known infrastructure outages.
- **Wind and PV tuning:** still **Open**; warm/sunny data now validates the saturation fix direction, while wind-specific evidence is still missing.
- **Warm-end outer-loop curve saturation:** still **Progressing**; post-fix data is encouraging but needs a clean run not dominated by eBUS null telemetry.
- **Volume-aware DHW demand prediction:** still **Progressing**; the controller predicted/attempted a fallback DHW launch, but the physical DHW disconnect and write failures make that interval invalid for policy sign-off.
- **Multical stale-data alerting:** still **Open**; `multical` was fresh at review time, but the cross-repo outage reinforces the need for stale-data and invalid-input alerts.
- **Transient HwcStorage zero/null during charge:** still **Open**; no new unexplained zero rows appeared in controller rows, and the bad DHW storage interval is now explained by the accidental unplug. The null-storage path still needs stale-input handling.

New issue:
- No separate heatpump-analysis plan item was added. The automatic post-outage recovery / gap-accounting work is already open in `energy-hub` as **Automate post-outage gap accounting / safe backfill**, and this review should be interpreted through that outage-recovery item.

## 2026-04-30 09:14 BST

The previous plan refresh timestamp was **2026-04-30 09:14 BST**.

### Latest Data Review

Most recent controller-log and PostgreSQL review for the open plan items.

Review window: **2026-04-24 00:31 BST** to **2026-04-30 09:14 BST**.

Evidence:
- `adaptive-heating-mvp` stayed on the same PID since the **2026-04-23 23:36 BST** start (`NRestarts=0`) and wrote PostgreSQL decisions from **2026-04-24 00:41:51 BST** through **2026-04-30 09:14:12 BST**.
- PostgreSQL recorded **582** controller decisions: `daytime_model=183`, `hold=176`, `overnight_coast=153`, `dhw_active=60`, `overnight_model=8`, and `dhw_schedule_launch=2`. Leather ranged **19.9-22.5°C**, Aldora **16.9-22.8°C**, and outside **4.25-28.31°C**.
- Forecasts were mostly live: **141** successful refreshes, **1** Open-Meteo fetch failure at **2026-04-30 04:35 BST** followed by refresh at **04:51 BST**, and **64/582** decision rows with null forecast fields.
- Cosy headroom remained misleading while actively charging: **56** Cosy rows had `battery_power_w < -500W`, with headroom spanning **-4.1 to +10.7 kWh**; examples include roughly **-5.0kW** charging with positive headroom.
- Overnight evidence improved: **153** coast rows include a cold-ish **4.25-5.9°C** early **2026-04-24** coast, but controller I/O and later warm/sunny saturation still confound sign-off.
- Host-side I/O is still noisy: controller logs contain **2** outer-cycle EAGAIN failures, **2** inner-cycle EAGAIN failures, **111** eBUS-style transient read/arbitration/no-signal errors, and two roughly **31 minute** decision gaps.
- Warm/sunny low-load behaviour regressed: **145** rows had `curve_after > 1.5`, model-required curves reached **4.0**, and logs emitted **377** curve warning lines. Several warm/sunny `daytime_model` decisions wrote curves **2.9-4.0** despite target flows around **26-27°C**.
- DHW scheduling wrote **18** `HwcTimer_*` updates and none contained raw `00:00`; there were **60** `dhw_active` rows and **2** `dhw_schedule_launch` rows. `multical` was fresh at review time, but `hwc_storage_temp_c` was null in **35** decisions.

Status changes:
- **Headroom unreliable during Cosy:** still **Open**; new rows again show active grid charging with positive or unstable headroom.
- **Overnight data growth:** still **Progressing**; there is now a useful cold-ish overnight coast/reheat window, but it is not enough to sign off while controller I/O and warm-end regressions remain active.
- **Host-side I/O hang:** still **Open**; no service restart occurred, but EAGAIN failures, eBUS errors, and two 31-minute gaps show the runtime path is not clean.
- **Forecast API reliability:** still **Open**; only one failed fetch and quick recovery, but null forecast/model rows still occur.
- **Wind and PV tuning:** still **Open**; high-solar days now provide evidence, and the evidence is bad enough to fold into the warm/sunny curve-saturation fix.
- **Warm-end outer-loop curve saturation:** changed from **Progressing** to **Open** during the data review; a follow-up code fix is now deployed, making the item **Progressing** pending warm/sunny validation.
- **Volume-aware DHW demand prediction:** still **Progressing**; DHW launches and timer normalization behaved, but storage-temperature nulls remain and volume policy still needs tuning.
- **Multical stale-data alerting:** still **Open**; `multical` was fresh, but no alerting implementation exists.
- **Transient HwcStorage zero/null during charge:** still **Open**; no new zero readings appeared in controller rows, but **35** null storage-temperature rows remain.

New issue:
- No new standalone plan item was added. The material new finding is a status regression in the existing warm/sunny curve-saturation item.

## 2026-04-24 00:31 BST

The previous plan refresh timestamp was **2026-04-24 00:31 BST**.

### Latest Data Review

Most recent controller-log and PostgreSQL review for the open plan items.

Review window: **2026-04-23 17:30 BST** to **2026-04-24 00:31 BST**.

Evidence:
- `adaptive-heating-mvp` restarted several times during deploy work, then remained active after the **23:36 BST** restart. It resolved PostgreSQL conninfo and wrote decisions through **2026-04-24 00:26:12 BST**.
- PostgreSQL recorded **27** controller decisions in the window: `daytime_model=13`, `dhw_active=4`, `overnight_coast=2`, with Leather between **21.2-22.2°C** and Aldora between about **19.3-19.8°C** in the reviewed tail.
- Forecast data was mostly usable but not perfect: **3/27** decision rows had null forecast values, and the log had one Open-Meteo fetch failure at **22:37 BST** followed by a successful refresh at **22:53 BST**.
- Cosy battery observations still show misleading headroom while actively charging, for example `battery_power_w` near **-5kW** with positive headroom during the evening Cosy window.
- DHW volume tracking reached **201L / full** by **2026-04-24 00:00 BST**, but both z2m-hub logs and PostgreSQL rows showed transient `HwcStorageTemp` zero/null readings during the evening charge path before recovery.

Status changes:
- **Headroom unreliable during Cosy:** still **Open**; the review reproduced the misleading-positive charging case.
- **Overnight data growth:** still **Progressing**; new overnight coast rows exist, but they are mild/warm rather than the clean cold regression window needed.
- **Host-side I/O hang:** still **Open**; no new multi-minute blind period after the final restart, but transient eBUS errors (`SYN received`, wrong symbol, read timeout) remain visible.
- **Forecast API reliability:** still **Open**; the failure recovered, so this is intermittent rather than an outage.
- **Warm-end outer-loop curve saturation:** still **Progressing**; the evening data exercised warm-end behaviour without a severe inversion recurrence, but a genuinely warm heating day is still needed.
- **Volume-aware DHW demand prediction:** still **Progressing**; runtime volume recovery behaved plausibly, but the new `HwcStorageTemp` zero/null issue must be handled before marking the input path settled.

New issue:
- **Open: Transient HwcStorage zero/null during charge** — during the evening DHW charge, z2m-hub saw `HwcStorageTemp=0.0` in several no-crossover decisions and adaptive-heating later saw a null storage temperature. The estimate recovered to full once valid readings returned, but the controllers should treat impossible storage readings as stale input rather than physical bottom-zone evidence.

## 2026-04-23 17:30 BST

The previous plan refresh timestamp was **2026-04-23 17:30 BST**.

No separate structured review body was present in [[plan]] before the **2026-04-24 00:31 BST** data review was added. The older state at that timestamp is available through git history; current truth remains in [[plan]] and thematic files such as [[heating-control]], [[domain]], [[infrastructure]], and [[constraints]].
