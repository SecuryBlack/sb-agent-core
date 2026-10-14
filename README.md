# sb-agent-core

Shared runtime and foundational utilities for SecuryBlack native Rust agents (OxiPulse, FerroSentry, CupraFlow, CromoForge, TitanVault, Nexus Agent). Not an agent itself — it provides the core foundation that prevents re-implementing identical primitives across separate open-source repositories.

## Developer entry points

This repository is a shared library, not an installable monitoring agent. Start with [OxiPulse](https://github.com/SecuryBlack/oxi-pulse) for server telemetry or [FerroSentry](https://github.com/SecuryBlack/ferro-sentry) for security auditing.

> **Status:** Active development. Reusable CI/installation infrastructure (reusable GitHub release workflows + shared installer libraries) is actively consumed across agents. The Rust crate modules (configuration, logging, service wrappers, updater, status sockets) are currently in active rollout.

---

## 🎯 Purpose & Value

SecuryBlack Rust agents are intentionally separate repositories — each has its own independent releases, identity, and open-source lifecycle. However, shared cross-cutting concerns (configuration loading, structured logging, service managers for systemd/Windows SCM, auto-update, and installation shell scripts) should not be duplicated across repos.

`sb-agent-core` provides a unified, published foundation crate that each agent consumes and versions independently.

---

## 📦 Contents

### CI & Installation Infrastructure (In Production)

- **`.github/workflows/release.yml`** — Reusable cross-target CI/CD release pipeline (`workflow_call`): multi-target compilation, tar.gz/zip packaging, SHA-256 checksum generation, and a **draft** GitHub Release (published only after offline signing, see [Releases firmadas](#releases-firmadas)).
- **`scripts/sign-release.sh`** — Signs a draft release's archives with the offline ed25519 key and re-uploads them.
- **`scripts/install-lib.sh`** / **`scripts/install-lib.ps1`** — Shared cross-platform installer functions: formatted logging, CPU architecture detection, dynamic release resolution, checksum verification, binary placement, and service registration.

### Rust Shared Crate Modules

- Strongly-typed configuration parsing (TOML + env vars + OS paths).
- Structured logging with file rotation (`tracing` + `tracing-appender`).
- Operating system service wrappers: systemd & Windows SCM.
- Parameterized auto-updater from GitHub Releases (`self_update`), accepting only ed25519-signed archives.
- Offline ring buffer with exponential backoff.
- **Local Status Socket** — Unix domain socket / Windows named pipe exposing runtime state as JSON for `<agent> status`, `<agent> top`, and local discovery.

---

## Releases firmadas

Los agentes corren como root y se actualizan solos, así que el updater (desde 0.3) solo instala archivos firmados con zipsign (ed25519) por una de las dos claves de `updater::VERIFYING_KEYS`. Las claves privadas no viven en GitHub: quien comprometa la org no puede publicar un binario que los agentes acepten.

1. **Borrador** — el tag dispara `release.yml`, que compila y deja la release siempre en borrador (invisible para agentes e instaladores).
2. **Firma** — quien custodia la clave ejecuta en su máquina `scripts/sign-release.sh <owner/repo> <tag>`: firma cada `.tar.gz`/`.zip` (contexto = nombre del asset), regenera los `.sha256`, verifica y resube.
3. **Publicar** — `gh release edit <tag> -R <owner/repo> --draft=false` (o `--publish` en el script). Ningún workflow de un agente debe publicar por su cuenta.

Claves: convención `~/.sb-signing/sb-release-primary.{priv,pub}` (la que firma) y `sb-release-backup.*` (respaldo, guardada aparte; sirve para rotar si la principal se pierde o se compromete). Requiere `gh` autenticado y `cargo install zipsign`.

---

## 🧩 Architectural Guideline

> **Strict Isolation:** Only features without domain-specific agent semantics belong in this crate.

No metric collectors, no security rules, no backup logic, and no deploy engines. Domain logic strictly resides within the respective agent repositories to avoid monolithic coupling.

---

## Consuming Agents

Nexus, OxiPulse, FerroSentry, CupraFlow, CromoForge and TitanVault declare a dependency on the Rust crate. Runtime adoption and resolved revisions are documented in the [workspace architecture](../sb-wiki/Tecnica/Arquitectura.md); use each Cargo.toml/Cargo.lock for executable dependency resolution.

The [wiki](../sb-wiki/Índice.md) owns shared product status, decisions and pending work. Keep runtime API and install/CI instructions in this repository.

---

## License

sb-agent-core is licensed under the [Apache License, Version 2.0](LICENSE).

## Maintainer context (workspace)

English is the primary language for this repository's public documentation. Shared product decisions, commercial terms and the backlog live in the [workspace wiki](../sb-wiki/Índice.md), [current state](../sb-wiki/Producto/Estado%20actual.md) and [product log](../sb-wiki/Producto/Bitácora.md). These links require `sb-wiki` as a sibling checkout. Keep agent-specific usage and technical contracts here; update shared decisions in the wiki.
