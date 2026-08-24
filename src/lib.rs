//! Runtime compartido para los agentes Rust de SecuryBlack. Ver `README.md`
//! y `TODO.md` para el porqué y el orden de trabajo.
//!
//! Regla de diseño: aquí no entra nada con semántica de agente (métricas,
//! reglas de seguridad, lógica de deploy). Solo lo que sería idéntico si lo
//! escribiera cualquiera de los cinco agentes desde cero.

pub mod buffer;
pub mod cli;
pub mod command_intake;
pub mod command_intake_client;
pub mod config;
pub mod intake_auth;
pub mod logging;
pub mod net;
pub mod service;
pub mod status;
pub mod status_client;
pub mod tui;
pub mod updater;
