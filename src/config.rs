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

/// Igual que `sync_version_field` pero genérico para un campo booleano
/// cualquiera (`field = true`/`false`) — pensado para ajustes que la nube
/// puede pedir cambiar en remoto vía comando (p.ej.
/// `allow_remote_os_upgrade`), no solo la versión que el propio agente se
/// escribe a sí mismo. Crea el fichero (y sus directorios) si no existe
/// todavía, a diferencia de `sync_version_field` — aquí sí puede ser la
/// primera escritura real de config de un agente instalado sin fichero
/// previo.
pub fn sync_bool_field(path: &Path, field: &str, value: bool) -> std::io::Result<()> {
    let contents = if path.exists() { std::fs::read_to_string(path)? } else { String::new() };

    let target_line = format!("{field} = {value}");
    let already_current = contents.lines().any(|l| l.trim_start() == target_line);
    if already_current {
        return Ok(());
    }

    let prefix = format!("{field} = ");
    let updated = if contents.lines().any(|l| l.trim_start().starts_with(&prefix)) {
        contents
            .lines()
            .map(|line| if line.trim_start().starts_with(&prefix) { target_line.clone() } else { line.to_string() })
            .collect::<Vec<_>>()
            .join("\n")
    } else if contents.is_empty() {
        target_line
    } else {
        format!("{}\n{target_line}", contents.trim_end())
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{updated}\n"))
}

/// Igual que `sync_bool_field` pero para un campo de texto (`field = "..."`)
/// — pensado para `token`: cuando la nube regenera el secreto compartido de
/// un servidor, este helper deja que el propio agente reciba el valor nuevo
/// por el túnel de comandos (autenticación distinta a la que usa el token
/// que se está reemplazando) y lo persista en su config.toml, en vez de
/// quedarse con el viejo hasta que alguien lo note por un 401 silencioso.
pub fn sync_string_field(path: &Path, field: &str, value: &str) -> std::io::Result<()> {
    let contents = if path.exists() { std::fs::read_to_string(path)? } else { String::new() };

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let target_line = format!("{field} = \"{escaped}\"");
    let already_current = contents.lines().any(|l| l.trim_start() == target_line);
    if already_current {
        return Ok(());
    }

    let prefix = format!("{field} = ");
    let updated = if contents.lines().any(|l| l.trim_start().starts_with(&prefix)) {
        contents
            .lines()
            .map(|line| if line.trim_start().starts_with(&prefix) { target_line.clone() } else { line.to_string() })
            .collect::<Vec<_>>()
            .join("\n")
    } else if contents.is_empty() {
        target_line
    } else {
        format!("{}\n{target_line}", contents.trim_end())
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{updated}\n"))
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

    #[test]
    fn sync_bool_field_adds_missing_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "endpoint = \"https://x\"\n").unwrap();
        sync_bool_field(&path, "allow_remote_os_upgrade", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("allow_remote_os_upgrade = true"));
        assert!(contents.contains("endpoint = \"https://x\""));
    }

    #[test]
    fn sync_bool_field_replaces_existing_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "allow_remote_os_upgrade = false\nendpoint = \"https://x\"\n").unwrap();
        sync_bool_field(&path, "allow_remote_os_upgrade", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("allow_remote_os_upgrade = true"));
        assert!(!contents.contains("allow_remote_os_upgrade = false"));
    }

    #[test]
    fn sync_bool_field_is_noop_when_already_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "allow_remote_os_upgrade = true\n").unwrap();
        sync_bool_field(&path, "allow_remote_os_upgrade", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "allow_remote_os_upgrade = true\n");
    }

    /// Reproduce el incidente real: un fichero sin salto de línea final no
    /// debe acabar con la línea nueva pegada a la última (lo que rompió
    /// `ferro-sentry`'s config.toml en producción cuando se editó a mano con
    /// `echo >>` en vez de con esta función).
    #[test]
    fn sync_bool_field_handles_missing_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "endpoint = \"https://x\"").unwrap(); // sin \n final
        sync_bool_field(&path, "allow_remote_os_upgrade", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("endpoint = \"https://x\"\nallow_remote_os_upgrade = true\n"));
        toml::from_str::<toml::Value>(&contents).expect("resulting file must still be valid TOML");
    }

    #[test]
    fn sync_bool_field_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        sync_bool_field(&path, "allow_remote_os_upgrade", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "allow_remote_os_upgrade = true\n");
    }
}
