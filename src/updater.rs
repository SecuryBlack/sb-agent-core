//! Auto-update desde GitHub Releases, parametrizado por repo y binario en
//! vez de copy-pasteado. Es exactamente el `updater/mod.rs` de OxiPulse con
//! las constantes convertidas en argumentos — ahí fue donde detectamos el
//! primer drift real entre agentes (`STARTUP_DELAY` 60s vs 300s sin que
//! nadie lo decidiera conscientemente).

use std::time::Duration;
use tracing::{error, info, warn};

pub struct UpdaterConfig {
    pub github_owner: &'static str,
    pub github_repo: &'static str,
    pub bin_name: &'static str,
    pub current_version: &'static str,
    pub startup_delay: Duration,
    pub check_interval: Duration,
}

impl UpdaterConfig {
    pub fn new(
        github_owner: &'static str,
        github_repo: &'static str,
        bin_name: &'static str,
        current_version: &'static str,
    ) -> Self {
        Self {
            github_owner,
            github_repo,
            bin_name,
            current_version,
            startup_delay: Duration::from_secs(60),
            check_interval: Duration::from_secs(86_400),
        }
    }

    pub fn with_startup_delay(mut self, d: Duration) -> Self {
        self.startup_delay = d;
        self
    }

    pub fn with_check_interval(mut self, d: Duration) -> Self {
        self.check_interval = d;
        self
    }
}

/// Lanza una tarea en background que comprueba actualizaciones
/// `startup_delay` después de arrancar, y luego cada `check_interval`.
/// Si aplica una actualización, el proceso sale con código 0 para que el
/// gestor de servicios (systemd / Windows SCM) lo reinicie con el binario
/// nuevo — el reinicio en sí no es cosa de este crate, es la política de
/// `Restart=always` / `sc.exe failure` que ya configura install-lib.
pub fn start_daily_check(cfg: UpdaterConfig) {
    tokio::spawn(async move {
        tokio::time::sleep(cfg.startup_delay).await;

        loop {
            info!("checking for updates…");
            match tokio::task::spawn_blocking({
                let cfg = UpdaterConfig {
                    github_owner: cfg.github_owner,
                    github_repo: cfg.github_repo,
                    bin_name: cfg.bin_name,
                    current_version: cfg.current_version,
                    startup_delay: cfg.startup_delay,
                    check_interval: cfg.check_interval,
                };
                move || check_and_update(&cfg)
            })
            .await
            {
                Ok(Ok(true)) => {
                    info!("update applied — exiting for service restart");
                    std::process::exit(0);
                }
                Ok(Ok(false)) => info!("already on latest version"),
                Ok(Err(e)) => warn!("update check failed: {}", e),
                Err(e) => error!("update task panicked: {}", e),
            }

            tokio::time::sleep(cfg.check_interval).await;
        }
    });
}

fn check_and_update(cfg: &UpdaterConfig) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let target = self_update::get_target();

    let status = self_update::backends::github::Update::configure()
        .repo_owner(cfg.github_owner)
        .repo_name(cfg.github_repo)
        .bin_name(cfg.bin_name)
        .target(&target)
        .current_version(cfg.current_version)
        .build()?
        .update()?;

    Ok(status.updated())
}
