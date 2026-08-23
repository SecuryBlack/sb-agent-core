//! Status socket: cada agente expone su estado en un socket local (Unix
//! socket / named pipe en Windows) como un JSON pequeño. Es la pieza más
//! importante del crate según el TODO — de aquí salen gratis `<agente>
//! status`/`<agente> top`, y Nexus puede leer el socket de cada agente en
//! vez de adivinar por heurísticas (proceso/puerto/existencia de fichero).
//!
//! Esquema: campos fijos comunes a los 5 agentes + un `details` de forma
//! libre para lo que cada uno quiera añadir. Es la decisión que el TODO
//! dejaba abierta ("probablemente: campos comunes fijos + blob `details`").

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusPayload {
    pub agent: String,
    pub version: String,
    /// "starting" | "running" | "degraded" | "stopping" — libre a propósito,
    /// distintos agentes tienen distintos estados intermedios (CromoForge:
    /// build → push → pull → healthcheck → live).
    pub state: String,
    pub since_unix: i64,
    /// Campo libre por agente: métricas de OxiPulse, hallazgos de FerroSentry,
    /// fase de deploy de CromoForge, lo que sea. Este crate no lo interpreta.
    pub details: serde_json::Value,
}

/// Handle compartido (clonable, `Send + Sync`) que el agente actualiza desde
/// su loop principal y que el servidor del socket lee para cada conexión.
#[derive(Clone)]
pub struct StatusHandle(Arc<RwLock<StatusPayload>>);

impl StatusHandle {
    pub fn new(agent: &str, version: &str) -> Self {
        let payload = StatusPayload {
            agent: agent.to_string(),
            version: version.to_string(),
            state: "starting".to_string(),
            since_unix: now_unix(),
            details: serde_json::Value::Null,
        };
        Self(Arc::new(RwLock::new(payload)))
    }

    pub fn set_state(&self, state: impl Into<String>) {
        if let Ok(mut p) = self.0.write() {
            p.state = state.into();
        }
    }

    pub fn set_details(&self, details: serde_json::Value) {
        if let Ok(mut p) = self.0.write() {
            p.details = details;
        }
    }

    pub fn snapshot(&self) -> StatusPayload {
        self.0.read().expect("status lock poisoned").clone()
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(unix)]
mod platform {
    use super::StatusHandle;
    use std::path::PathBuf;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;
    use tracing::warn;

    /// `/run/sb-agent/<agent>.sock`, o `$TMPDIR/sb-agent/<agent>.sock` si
    /// `/run` no es escribible (por ejemplo, corriendo sin privilegios en dev).
    pub fn default_socket_path(agent_name: &str) -> PathBuf {
        let run_dir = PathBuf::from("/run/sb-agent");
        let base = if std::fs::create_dir_all(&run_dir).is_ok() {
            run_dir
        } else {
            std::env::temp_dir().join("sb-agent")
        };
        let _ = std::fs::create_dir_all(&base);
        base.join(format!("{agent_name}.sock"))
    }

    /// Arranca el listener en background. Un socket viejo del mismo path (de
    /// un proceso anterior que no limpió al morir) se borra antes de bind —
    /// si sigue en uso por otro proceso, el bind fallará de forma visible.
    pub fn spawn_server(handle: StatusHandle, socket_path: PathBuf) {
        let _ = std::fs::remove_file(&socket_path);
        tokio::spawn(async move {
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => {
                    warn!(path = %socket_path.display(), error = %e, "status socket: bind failed");
                    return;
                }
            };
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "status socket: accept failed");
                        continue;
                    }
                };
                let payload = handle.snapshot();
                let Ok(json) = serde_json::to_vec(&payload) else { continue };
                let _ = stream.write_all(&json).await;
                let _ = stream.shutdown().await;
            }
        });
    }
}

#[cfg(windows)]
mod platform {
    use super::StatusHandle;
    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ServerOptions;
    use tracing::warn;

    /// `\\.\pipe\sb-agent-<agent>`
    pub fn default_socket_path(agent_name: &str) -> String {
        format!(r"\\.\pipe\sb-agent-{agent_name}")
    }

    pub fn spawn_server(handle: StatusHandle, pipe_name: String) {
        tokio::spawn(async move {
            loop {
                let server = match ServerOptions::new().first_pipe_instance(false).create(&pipe_name) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(pipe = %pipe_name, error = %e, "status socket: pipe create failed");
                        return;
                    }
                };
                if let Err(e) = server.connect().await {
                    warn!(error = %e, "status socket: pipe connect failed");
                    continue;
                }
                let mut server = server;
                let payload = handle.snapshot();
                if let Ok(json) = serde_json::to_vec(&payload) {
                    let _ = server.write_all(&json).await;
                }
                let _ = server.flush().await;
                drop(server);
            }
        });
    }
}

pub use platform::{default_socket_path, spawn_server};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_reflects_updates_in_snapshot() {
        let handle = StatusHandle::new("test-agent", "1.0.0");
        assert_eq!(handle.snapshot().state, "starting");
        handle.set_state("running");
        handle.set_details(serde_json::json!({"foo": "bar"}));
        let snap = handle.snapshot();
        assert_eq!(snap.state, "running");
        assert_eq!(snap.details["foo"], "bar");
    }
}
