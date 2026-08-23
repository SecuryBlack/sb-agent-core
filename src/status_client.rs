//! Cliente del status socket: conecta, lee un snapshot JSON, cierra.
//! Es lo que usan `<agente> status` (una foto) y `<agente> top` (repetido
//! en bucle) — y, más adelante, el `registry` de Nexus para dejar de adivinar
//! qué agentes hay corriendo.

use crate::status::StatusPayload;
use std::time::Duration;

#[derive(Debug)]
pub enum StatusClientError {
    Connect(std::io::Error),
    Read(std::io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for StatusClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusClientError::Connect(e) => write!(f, "could not connect to status socket: {e}"),
            StatusClientError::Read(e) => write!(f, "could not read from status socket: {e}"),
            StatusClientError::Parse(e) => write!(f, "could not parse status payload: {e}"),
        }
    }
}

impl std::error::Error for StatusClientError {}

/// Lee un snapshot del status socket de `agent_name`, de forma síncrona y
/// bloqueante (pensado para `<agente> status`/`top`, no para código async).
pub fn read_once(agent_name: &str) -> Result<StatusPayload, StatusClientError> {
    platform::read_once(agent_name)
}

/// Como [`read_once`], pero con un timeout de conexión — para `top`, donde no
/// queremos que un socket colgado bloquee el refresco indefinidamente.
pub fn read_once_timeout(agent_name: &str, timeout: Duration) -> Result<StatusPayload, StatusClientError> {
    platform::read_once_timeout(agent_name, timeout)
}

#[cfg(unix)]
mod platform {
    use super::StatusClientError;
    use crate::status::StatusPayload;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    pub fn read_once(agent_name: &str) -> Result<StatusPayload, StatusClientError> {
        let path = crate::status::default_socket_path(agent_name);
        let mut stream = UnixStream::connect(&path).map_err(StatusClientError::Connect)?;
        read_payload(&mut stream)
    }

    pub fn read_once_timeout(agent_name: &str, timeout: Duration) -> Result<StatusPayload, StatusClientError> {
        let path = crate::status::default_socket_path(agent_name);
        let stream = UnixStream::connect(&path).map_err(StatusClientError::Connect)?;
        let _ = stream.set_read_timeout(Some(timeout));
        let mut stream = stream;
        read_payload(&mut stream)
    }

    fn read_payload(stream: &mut UnixStream) -> Result<StatusPayload, StatusClientError> {
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).map_err(StatusClientError::Read)?;
        serde_json::from_slice(&buf).map_err(StatusClientError::Parse)
    }
}

#[cfg(windows)]
mod platform {
    use super::StatusClientError;
    use crate::status::StatusPayload;
    use std::io::Read;
    use std::time::Duration;

    /// Named pipes de Windows no tienen un "connect timeout" trivial desde
    /// `std`, así que aquí usamos siempre un intento simple; `top` tolera un
    /// fallo puntual de refresco mostrando el último snapshot conocido.
    pub fn read_once(agent_name: &str) -> Result<StatusPayload, StatusClientError> {
        let pipe_name = crate::status::default_socket_path(agent_name);
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .open(&pipe_name)
            .map_err(StatusClientError::Connect)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(StatusClientError::Read)?;
        serde_json::from_slice(&buf).map_err(StatusClientError::Parse)
    }

    pub fn read_once_timeout(agent_name: &str, _timeout: Duration) -> Result<StatusPayload, StatusClientError> {
        read_once(agent_name)
    }
}
