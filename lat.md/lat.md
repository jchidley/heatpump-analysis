# lat.md Index

This index maps the project's structured knowledge graph so agents can find the current architecture, domain rules, constraints, and operationally important subsystems.

It also links to file-level source index pages when individual source files need their own dedicated `lat.md` entries.

- [[domain]] — Core domain model: operating states, house, DHW cylinder, tariff, feeds
- [[constraints]] — Hard rules, known pitfalls, sensor gotchas, eBUS behaviour
- [[architecture]] — Binaries, module dependencies, data flow, configuration, implicit contracts
- [[heating-control]] — V2 two-loop controller, overnight planner, modes, pilot history
- [[thermal-model]] — 13-room thermal network: calibration, solver, accuracy, regression gates
- [[history-evidence]] — Historical heating/DHW reconstruction, history-review, evidence boundaries
- [[infrastructure]] — Monitoring devices, MQTT topology, eBUS stack, sensors, VRC 700 baseline
- [[plan]] — Open items, next steps, and links to detailed plan docs in `docs/`
- [[reviews]] — Stub page noting that historical review narrative lives in git history rather than current docs
- [[tests]] — Targeted executable specs for controller invariants and other migration-sensitive behaviour that need explicit code coverage
- [[tsdb-migration]] — Current migration state, remaining completion actions, and post-migration backlog for the PostgreSQL/TimescaleDB cutover in this repo
- [[src]] — File-level source index for source files that have dedicated `lat.md` documentation
