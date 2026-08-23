//! Carga de configuración compartida.
//!
//! Cada agente sigue definiendo su propio `struct Config` con sus propios
//! campos (endpoint, token, lo que sea) — eso es semántica de agente y no
//! entra aquí. Lo que sí es común a los cuatro es: dónde vive `config.toml`
//! según el SO, cómo parsearlo, y cómo mantener el campo `version` al día
//! (los 4 agentes tienen, por separado, un commit "auto-write version in
//! config.toml" — es la misma lógica cuatro veces).

use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "config io error: {e}"),
            ConfigError::Parse(msg) => write!(f, "config parse error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Ruta por defecto de `config.toml` para un agente, según el SO.
/// Linux/macOS: `/etc/<agent_name>/config.toml`
/// Windows:     `C:\ProgramData\<agent_name>\config.toml`
pub fn default_config_path(agent_name: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let program_data = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
        PathBuf::from(program_data).join(agent_name).join("config.toml")
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/etc").join(agent_name).join("config.toml")
    }
}

/// Parsea `config.toml` directamente al `struct Config` propio del agente vía
/// serde. El agente aplica overrides de variables de entorno después —
/// esos nombres de variable (`OXIPULSE_ENDPOINT`, etc.) son suyos, no de aquí.
///
/// Si el fichero no existe, devuelve `T::default()` para que el agente decida
/// qué hacer con campos obligatorios ausentes (normalmente fallar con un
/// mensaje claro, como ya hace cada uno).
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, ConfigError> {
    if !path.exists() {
        return Ok(T::default());
    }
    let contents = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    toml::from_str(&contents).map_err(|e| ConfigError::Parse(e.to_string()))
}

/// Si `current_version` no coincide con la línea `version = "..."` del
/// fichero, la reescribe (o la añade al principio si no existe). No-op si
/// el fichero no existe todavía — nada que actualizar antes de la primera
/// instalación.
///
/// Reescritura basada en texto, no en un round-trip TOML completo, a
/// propósito: preserva comentarios y el orden del resto del fichero, que es
/// lo que hoy hacen a mano oxi-pulse/ferro-sentry/cupra-flow/nexus-agent.
pub fn sync_version_field(path: &Path, current_version: &str) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(path)?;
    let already_current = contents
        .lines()
        .any(|l| l.trim_start() == format!("version = \"{current_version}\""));
    if already_current {
        return Ok(());
    }

    let updated = if contents.lines().any(|l| l.trim_start().starts_with("version = ")) {
        contents
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("version = ") {
                    format!("version = \"{current_version}\"")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        format!("version = \"{current_version}\"\n{contents}")
    };
    std::fs::write(path, updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, Default, PartialEq)]
    struct FakeConfig {
        #[serde(default)]
        endpoint: Option<String>,
        #[serde(default)]
        interval_secs: Option<u64>,
    }

    #[test]
    fn load_missing_file_returns_default() {
        let path = Path::new("/nonexistent/sb-agent-core-test/config.toml");
        let cfg: FakeConfig = load(path).unwrap();
        assert_eq!(cfg, FakeConfig::default());
    }

    #[test]
    fn load_parses_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "endpoint = \"https://x\"\ninterval_secs = 30\n").unwrap();
        let cfg: FakeConfig = load(&path).unwrap();
        assert_eq!(cfg.endpoint.as_deref(), Some("https://x"));
        assert_eq!(cfg.interval_secs, Some(30));
    }

    #[test]
    fn sync_version_field_adds_missing_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "endpoint = \"https://x\"\n").unwrap();
        sync_version_field(&path, "1.2.3").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.lines().next().unwrap() == "version = \"1.2.3\"");
    }

    #[test]
    fn sync_version_field_replaces_existing_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "version = \"0.9.0\"\nendpoint = \"https://x\"\n").unwrap();
        sync_version_field(&path, "1.2.3").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"1.2.3\""));
        assert!(!contents.contains("0.9.0"));
    }

    #[test]
    fn sync_version_field_noop_on_missing_file() {
        let path = Path::new("/nonexistent/sb-agent-core-test/config.toml");
        assert!(sync_version_field(path, "1.2.3").is_ok());
    }
}
