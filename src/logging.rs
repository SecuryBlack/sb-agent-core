//! Init de logging compartido: fichero rotado diario con fallback a stdout,
//! limpieza de logs antiguos, y `RUST_LOG`/`EnvFilter` — idéntico hoy en los
//! 4 agentes salvo el nombre del directorio y del fichero.

use std::path::Path;
use std::time::Duration;

/// Inicializa `tracing` escribiendo en `<log_dir>/<agent_name>.log` (rotación
/// diaria) si el directorio es escribible, o en stdout si no (por ejemplo,
/// corriendo en consola sin permisos de administrador). Limpia logs de más
/// de 7 días en cada arranque.
///
/// Prioridad del nivel de log: `RUST_LOG` (si está definida) > `default_level`
/// > `"info"`. La mayoría de agentes no tienen un nivel configurable propio y
/// simplemente pasan `"info"` como `default_level`; FerroSentry, que sí tiene
/// `log_level` en su `config.toml`, pasa ese valor.
pub fn init(agent_name: &str, log_dir: &Path, default_level: &str) {
    let write_test_path = log_dir.join(".write_test");
    let writable = std::fs::create_dir_all(log_dir).is_ok() && std::fs::write(&write_test_path, "").is_ok();
    let _ = std::fs::remove_file(&write_test_path);

    let env_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level.to_string()))
    };

    if writable {
        clean_old_logs(log_dir, Duration::from_secs(7 * 24 * 3600));
        let file_appender = tracing_appender::rolling::daily(log_dir, format!("{agent_name}.log"));
        tracing_subscriber::fmt()
            .with_env_filter(env_filter())
            .with_writer(file_appender)
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter()).init();
    }
}

fn clean_old_logs(log_dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age > max_age {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
}
