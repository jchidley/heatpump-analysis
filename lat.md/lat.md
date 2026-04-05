This directory defines the high-level concepts, business logic, and architecture of this project using markdown. It is managed by [lat.md](https://www.npmjs.com/package/lat.md) — a tool that anchors source code to these definitions. Install the `lat` command with `npm i -g lat.md` and run `lat --help`.

- [[domain]] — Core domain model: operating states, house, DHW cylinder, tariff, feeds
- [[constraints]] — Hard rules, known pitfalls, sensor gotchas, eBUS behaviour
- [[architecture]] — Binaries, module dependencies, data flow, configuration, implicit contracts
- [[heating-control]] — V2 two-loop controller, overnight planner, modes, pilot history
- [[thermal-model]] — 13-room thermal network: calibration, solver, ventilation, accuracy
- [[infrastructure]] — Monitoring devices, MQTT topology, eBUS stack, sensors, VRC 700 baseline
