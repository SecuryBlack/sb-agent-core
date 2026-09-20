# sb-agent-core

Shared runtime and foundational utilities for SecuryBlack native Rust agents (OxiPulse, FerroSentry, CupraFlow, CromoForge, TitanVault, Nexus Agent). Not an agent itself — it provides the core foundation that prevents re-implementing identical primitives across separate open-source repositories.

> **Status:** Active development. Reusable CI/installation infrastructure (reusable GitHub release workflows + shared installer libraries) is actively consumed across agents. The Rust crate modules (configuration, logging, service wrappers, updater, status sockets) are currently in active rollout.

---

## 🎯 Purpose & Value

SecuryBlack Rust agents are intentionally separate repositories — each has its own independent releases, identity, and open-source lifecycle. However, shared cross-cutting concerns (configuration loading, structured logging, service managers for systemd/Windows SCM, auto-update, and installation shell scripts) should not be duplicated across repos.

`sb-agent-core` provides a unified, published foundation crate that each agent consumes and versions independently.

---

## 📦 Contents

### CI & Installation Infrastructure (In Production)

- **`.github/workflows/release.yml`** — Reusable cross-target CI/CD release pipeline (`workflow_call`): multi-target compilation, tar.gz/zip packaging, SHA-256 checksum generation, and automated GitHub Releases publishing.
- **`scripts/install-lib.sh`** / **`scripts/install-lib.ps1`** — Shared cross-platform installer functions: formatted logging, CPU architecture detection, dynamic release resolution, checksum verification, binary placement, and service registration.

### Rust Shared Crate Modules

- Strongly-typed configuration parsing (TOML + env vars + OS paths).
- Structured logging with file rotation (`tracing` + `tracing-appender`).
- Operating system service wrappers: systemd & Windows SCM.
- Parameterized auto-updater from GitHub Releases (`self_update`).
- Offline ring buffer with exponential backoff.
- **Local Status Socket** — Unix domain socket / Windows named pipe exposing runtime state as JSON for `<agent> status`, `<agent> top`, and local discovery.

---

## 🧩 Architectural Guideline

> **Strict Isolation:** Only features without domain-specific agent semantics belong in this crate.

No metric collectors, no security rules, no backup logic, and no deploy engines. Domain logic strictly resides within the respective agent repositories to avoid monolithic coupling.

---

## Consuming Agents

| Agent | Current Modules Used |
|---|---|
| [OxiPulse](https://github.com/SecuryBlack/oxi-pulse) | Release workflow, install-lib |
| [FerroSentry](https://github.com/SecuryBlack/ferro-sentry) | Release workflow, install-lib |
| [CupraFlow](https://github.com/SecuryBlack/cupra-flow) | Release workflow, install-lib |
| [TitanVault](https://github.com/SecuryBlack/titan-vault) | Crate foundation, release workflow, install-lib |
| [CromoForge](https://github.com/SecuryBlack/cromo-forge) | Crate foundation, release workflow, install-lib |
| [Nexus Agent](https://github.com/SecuryBlack/nexus-agent) | Release workflow, install-lib |

---

## License

sb-agent-core is licensed under the [Apache License, Version 2.0](LICENSE).
