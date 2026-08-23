# sb-agent-core

Runtime compartido para los agentes Rust de SecuryBlack (OxiPulse, FerroSentry, CupraFlow, Nexus Agent y el futuro CromoForge). No es un agente en sí — es la base que evita reimplementar lo mismo cinco veces en cinco repos open source separados.

> **Estado:** en construcción. La parte de CI/instalación (release workflow reutilizable + librerías de instalador) ya está en uso por los cuatro agentes existentes. El crate Rust (config, logging, wrapper de servicio, updater, status socket) es diseño, todavía sin código — ver [`TODO.md`](TODO.md).

---

## 🎯 Qué resuelve

Los agentes Rust de SecuryBlack son repos independientes a propósito — cada uno tiene su propio release, su propia identidad, su propio ciclo de vida open source. Eso es correcto, pero tiene un coste: config loading, logging, wrapper de servicio (systemd/Windows SCM), auto-update y scripts de instalación se copiaban literalmente de un repo a otro. Ese copy-paste ya derivó en comportamiento distinto sin que nadie lo decidiera (ver el detalle en `TODO.md`).

`sb-agent-core` es la respuesta: un crate publicado (no un monorepo, no submodules) que cada agente consume y versiona por su cuenta.

---

## 📦 Contenido

### CI / instalación (ya en uso)

- **`.github/workflows/release.yml`** — pipeline de release reutilizable (`workflow_call`): build cross-target, empaquetado tar.gz/zip, checksum, publicación en GitHub Releases. Cada agente lo invoca con su propia matriz de targets.
- **`scripts/install-lib.sh`** / **`scripts/install-lib.ps1`** — funciones compartidas para los `install.sh`/`install.ps1` de cada agente: logging, detección de arquitectura, resolución de última versión, descarga + verificación de checksum, instalación de binario, registro de servicio.

### Crate Rust (diseño, sin implementar todavía)

- Carga de configuración (TOML + env + rutas por SO).
- Logging con rotación (`tracing` + `tracing-appender`).
- Wrapper de servicio: systemd + Windows SCM.
- Updater parametrizado desde GitHub Releases (`self_update`).
- Buffer offline con backoff exponencial.
- **Status socket** — un socket local (Unix socket / named pipe) que expone el estado del agente como JSON, para `<agente> status`/`<agente> top` y para que Nexus Agent descubra agentes locales sin heurísticas frágiles.

Ver [`TODO.md`](TODO.md) para el detalle completo de las decisiones y el orden de trabajo.

---

## 🧩 Regla de diseño

> En este crate solo entra lo que **no tiene semántica de agente**.

Nada de métricas, nada de reglas de seguridad, nada de lógica de despliegue. El día que algo específico de un agente quiera colarse aquí, la respuesta es no — así es como se evita que esto se convierta en un god-crate que obligue a los cinco agentes a subir de versión a la vez.

---

## Agentes que lo consumen

| Agente | Qué usa hoy |
|---|---|
| [OxiPulse](https://github.com/SecuryBlack/oxi-pulse) | release workflow, install-lib |
| [FerroSentry](https://github.com/SecuryBlack/ferro-sentry) | release workflow, install-lib |
| [Nexus Agent](https://github.com/SecuryBlack/nexus-agent) | release workflow, install-lib |
| [CupraFlow](https://github.com/SecuryBlack/cupra-flow) | release workflow, install-lib (parcial) |
| CromoForge | primer consumidor previsto del crate Rust (todavía sin publicar) |

---

## License

sb-agent-core is licensed under the [Apache License, Version 2.0](LICENSE).
