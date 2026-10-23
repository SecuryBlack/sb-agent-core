//! Init de logging compartido: fichero rotado diario con fallback a stdout,
//! retención de 7 días, y `RUST_LOG`/`EnvFilter` — idéntico hoy en los
//! 4 agentes salvo el nombre del directorio y del fichero.

use std::path::Path;

/// Inicializa `tracing` escribiendo en `<log_dir>/<agent_name>.log` (rotación
/// diaria) si el directorio es escribible, o en stdout si no (por ejemplo,
/// corriendo en consola sin permisos de administrador). Guarda los 7 últimos
/// ficheros: el appender borra los anteriores al arrancar y en cada rotación.
///
/// Prioridad del nivel de log: `RUST_LOG` (si está definida) >
/// `default_level` > `"info"`. La mayoría de agentes no tienen un nivel configurable propio y
/// simplemente pasan `"info"` como `default_level`; FerroSentry, que sí tiene
/// `log_level` en su `config.toml`, pasa ese valor.
pub fn init(agent_name: &str, log_dir: &Path, default_level: &str) {
    let write_test_path = log_dir.join(".write_test");
    let writable = std::fs::create_dir_all(log_dir).is_ok() && std::fs::write(&write_test_path, "").is_ok();
    let _ = std::fs::remove_file(&write_test_path);

    let env_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level))
    };

    if writable {
        match tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(format!("{agent_name}.log"))
            .max_log_files(7)
            .build(log_dir)
        {
            Ok(file_appender) => {
                tracing_subscriber::fmt()
                    .with_env_filter(env_filter())
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .init();
                return;
            }
            Err(e) => {
                eprintln!("[{agent_name}] warning: could not initialize log file ({e}), falling back to stdout");
            }
        }
    }
    tracing_subscriber::fmt().with_env_filter(env_filter()).init();
}
