//! Auto-update desde GitHub Releases, parametrizado por repo y binario en
//! vez de copy-pasteado. Es exactamente el `updater/mod.rs` de OxiPulse con
//! las constantes convertidas en argumentos — ahí fue donde detectamos el
//! primer drift real entre agentes (`STARTUP_DELAY` 60s vs 300s sin que
//! nadie lo decidiera conscientemente).
//!
//! Solo se aceptan archivos firmados (zipsign, ed25519ph) con alguna de
//! `VERIFYING_KEYS`: los agentes corren como root, así que sin firma quien
//! comprometa la org de GitHub tendría root en todos los servidores. Las
//! claves privadas no viven en GitHub; las releases salen como borrador y se
//! firman a mano con `scripts/sign-release.sh` antes de publicarlas.
//!
//! Hay dos claves (principal y respaldo, guardada aparte) para poder rotar
//! sin dejar agentes huérfanos: si la principal se pierde o se compromete,
//! se firma con la de respaldo una release que ya trae el par nuevo, y los
//! agentes desplegados la aceptan porque confían en ambas.
//!
//! Limitación conocida: el contexto firmado es solo el nombre del asset
//! (`oxipulse-x86_64-unknown-linux-gnu.tar.gz`), no la versión. Eso impide
//! cruzar binarios entre agentes o targets, pero no un downgrade: un archivo
//! antiguo firmado se puede re-publicar bajo un tag más alto. Como solo hay
//! firmas a partir de versiones que ya verifican, no se rompe la cadena; el
//! riesgo es volver a una versión con un fallo conocido. Pendiente.

use std::time::Duration;
use tracing::{error, info, warn};

/// Claves públicas ed25519 (32 bytes, tal cual las escribe `zipsign gen-key`)
/// que firman las releases. Nunca vacía: con la lista vacía `self_update` se
/// salta la verificación sin avisar (lo comprueba `tests/release_signing.rs`).
pub const VERIFYING_KEYS: [[u8; 32]; 2] = [
    // sb-release-primary
    [
        0x63, 0xbd, 0xcf, 0xef, 0xaa, 0x2d, 0xd7, 0x2c, 0x1f, 0x78, 0xb2, 0xb0, 0xd9, 0x56, 0x05,
        0xaf, 0x63, 0x38, 0xa2, 0xbe, 0x62, 0xf8, 0x6d, 0x09, 0x86, 0x26, 0x08, 0x35, 0xde, 0x9c,
        0x2b, 0x6e,
    ],
    // sb-release-backup
    [
        0x3d, 0xc5, 0x55, 0x62, 0x7f, 0xb5, 0xc0, 0x79, 0x51, 0x7c, 0x7f, 0x21, 0xfe, 0x38, 0x58,
        0xd1, 0x48, 0xac, 0xeb, 0x3f, 0xd6, 0xa1, 0xb4, 0xfb, 0x1b, 0x02, 0xf0, 0x4b, 0xd0, 0xcb,
        0xe1, 0x7c,
    ],
];

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

/// Igual que la comprobación que hace el loop de fondo, pero expuesta para
/// que un agente pueda dispararla bajo demanda (p.ej. un command_type
/// `update_now` recibido por el intake, para el botón "Actualizar" de la
/// app) sin esperar al próximo `check_interval`.
pub fn check_now(cfg: &UpdaterConfig) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    check_and_update(cfg)
}

fn check_and_update(cfg: &UpdaterConfig) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let target = self_update::get_target();

    let status = self_update::backends::github::Update::configure()
        .repo_owner(cfg.github_owner)
        .repo_name(cfg.github_repo)
        .bin_name(cfg.bin_name)
        .target(target)
        .current_version(cfg.current_version)
        .verifying_keys(VERIFYING_KEYS)
        .build()?
        .update()?;

    Ok(status.updated())
}
