# Reviews

This file keeps older review snapshots so [[plan]] can carry the live item descriptions plus the newest dated review only.

## 2026-04-10 08:11 BST Review Snapshot

This snapshot captures the controller/data review that immediately preceded the current `plan.md` refresh.

### Heating Controller

That review window added a mild overnight checkpoint plus the first proper morning validation of the outer/inner conflict fix, but several observability and tuning items remained open.

Open controller concerns at that snapshot were:
- Cosy headroom was still wrong, now reproduced again with an impossible negative reading during the morning Cosy window.
- Overnight evidence had grown usefully on 9–10 Apr, but the window was still confounded by three DHW overlaps rather than becoming a clean regression anchor.
- Forecast refreshes looked healthy in that window, but the earlier intermittent null forecast/model gap was still unexplained.
- Warm-end saturation still had no new near-setpoint validation case.
- Wind/PV tuning still lacked a useful weather day.
- Elvina baseline collection was continuing and both baseline sensors were staying online overnight.

### DHW Scheduling

That snapshot captured the clearest draw-budget miss so far, confirming that demand prediction rather than charge execution was still the main DHW software gap.

After a **00:08–01:00 BST** full-crossover recharge, `remaining_litres` still fell to **0L** by **07:11 BST** and `T1` collapsed from **43.6→26.9°C** by **06:52 BST**, forcing a new morning charge at **06:54 BST**. Seasonal mode remained `eco`.

### Pico eBUS Adapter

No status change was recorded for the Pico Phase 2 PIO UART work in that snapshot.

## 2026-04-09 17:36 BST Review Snapshot

This snapshot captures the controller/data review that immediately preceded the current `plan.md` refresh.

### Heating Controller

That review window was daytime-only, so the main outcome was status maintenance rather than overnight validation.

Open controller concerns at that snapshot were:
- Cosy headroom was still wrong, now confirmed again in an afternoon charging window.
- Overnight data growth still lacked the promised 9–10 Apr overnight checkpoint.
- Forecast refreshes were mostly healthy, but intermittent null forecast/model rows still appeared without a matching upstream outage.
- Warm-end saturation was only partially fixed: the >=setpoint fallback worked, but near-setpoint inversion below 19°C still hit `curve_after=4.00`.
- The outer/inner conflict fix had been deployed, but only over-target late-afternoon evidence existed; the original under-target morning failure mode was still awaiting validation.
- Elvina baseline collection was continuing and the outdoor sensor was staying online.

### DHW Scheduling

The daytime DHW evidence in that snapshot showed a clean afternoon charge and a correct timer decision, but not the missing demand-budget case.

A **12:04–13:01 BST** afternoon session reached full crossover, and the controller then skipped the next morning window because predicted **07:00 T1 = 40.4°C**. Seasonal mode remained `eco`.

### Pico eBUS Adapter

No status change was recorded for the Pico Phase 2 PIO UART work in that snapshot.

## 2026-04-09 08:55 BST Review Snapshot

This snapshot captures the older controller/data review before the later 9 Apr daytime refresh.

### Heating Controller

The 8–9 Apr overnight window was the key new evidence at that point. It gave the first clean post-recovery coast-then-hold night, validated the new overnight strategy shape on a warm night, and confirmed that forecast refreshes were quiet overnight.

Open controller concerns at that snapshot were:
- Cosy headroom was still unresolved because the positive morning reading came from a nearly full battery rather than a true fix.
- Warm-end outer-loop saturation was still repeating at `model_required_curve: 4.0` in mild conditions.
- Morning active-heating traces showed the outer loop resetting the curve while the inner loop was still relearning.
- Wind/PV tuning still lacked a useful weather case.
- Elvina baseline collection had started and the outdoor sensor was staying online.

### DHW Scheduling

A 22:00–00:04 DHW cycle completed successfully and no overnight shower occurred, so the main unresolved DHW item remained the missing volume-aware demand model rather than basic charge execution.

The system stayed in seasonal `eco` mode, which was still the correct manual setting.

### Pico eBUS Adapter

No status change was recorded for the Pico Phase 2 PIO UART work in that snapshot.
