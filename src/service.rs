//! Wrapper de servicio: arranque en consola (Linux/macOS y fallback en
//! Windows) o como Windows Service, parametrizado por el nombre de servicio
//! y la corrutina propia del agente. Generaliza el patrón que hoy repite
//! `main.rs` en OxiPulse/FerroSentry/CupraFlow/NexusAgent casi al carácter.
//!
//! El agente pasa su propio "run loop" como una función que recibe el
//! receiver de shutdown y devuelve un future; este módulo no sabe nada de
//! métricas, eventos ni deploys — solo de arrancar/parar ese future dentro
//! del runtime de tokio correcto para cada entorno.

use std::future::Future;
use tokio::sync::oneshot;

/// Corre `run` en un runtime tokio multi-hilo, en modo consola: escucha
/// Ctrl+C y dispara el shutdown. Válido para Linux/macOS (bajo systemd o a
/// mano) y como fallback en Windows cuando el proceso no lo arrancó el SCM.
pub fn run_console<F, Fut>(run: F)
where
    F: FnOnce(oneshot::Receiver<()>) -> Fut,
    Fut: Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            let _ = shutdown_tx.send(());
        });
        run(shutdown_rx).await;
    });
}

#[cfg(windows)]
pub mod windows {
    //! Registro como Windows Service vía `windows-service`. `define_windows_service!`
    //! exige una función `extern "system"` fija, así que el nombre de servicio y
    //! el run loop del agente se guardan en estáticos antes de arrancar el
    //! dispatcher — es el único hueco por el que hay que pasar parámetros aquí.

    use std::ffi::OsString;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::OnceLock;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use windows_service::{
        define_windows_service,
        service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };

    type BoxFut = Pin<Box<dyn Future<Output = ()> + Send>>;
    type RunFn = Box<dyn Fn(oneshot::Receiver<()>) -> BoxFut + Send + Sync>;

    static SERVICE_NAME: OnceLock<String> = OnceLock::new();
    static RUN_FN: OnceLock<RunFn> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    /// Arranca como Windows Service. Devuelve error si el proceso no fue
    /// iniciado por el SCM (código Win32 1063) — en ese caso el agente debe
    /// caer a [`super::run_console`], igual que ya hacían todos por separado.
    pub fn run_service<F, Fut>(service_name: &str, run: F) -> Result<(), windows_service::Error>
    where
        F: Fn(oneshot::Receiver<()>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let _ = SERVICE_NAME.set(service_name.to_string());
        let _ = RUN_FN.set(Box::new(move |rx| Box::pin(run(rx))));
        service_dispatcher::start(service_name, ffi_service_main)
    }

    /// True si el error indica "no arrancado por el SCM" — el caller debe
    /// caer a modo consola en ese caso en vez de tratarlo como fallo real.
    pub fn is_not_started_by_scm(e: &windows_service::Error) -> bool {
        matches!(e, windows_service::Error::Winapi(io_err) if io_err.raw_os_error() == Some(1063))
    }

    fn service_main(_arguments: Vec<OsString>) {
        let service_name = SERVICE_NAME.get().cloned().unwrap_or_default();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let shutdown_tx = std::sync::Mutex::new(Some(shutdown_tx));

        let status_handle = service_control_handler::register(&service_name, move |control_event| match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Ok(mut guard) = shutdown_tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(());
                    }
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .expect("failed to register service control handler");

        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .expect("failed to set service status Running");

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let run_fn = RUN_FN.get().expect("run_service must set RUN_FN before dispatching");
        rt.block_on(run_fn(shutdown_rx));

        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
    }
}
