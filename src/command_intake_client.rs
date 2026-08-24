//! Cliente del intake de comandos: conecta, manda un `CommandEnvelope`,
//! lee cero o más `CommandProgress` seguidos de un `CommandResponse` final.
//! Síncrono y bloqueante, igual que `status_client` — pensado para que
//! Nexus lo llame desde `spawn_blocking` al reenviar un comando que le
//! llegó por el túnel al intake local del agente destino.

use crate::command_intake::{CommandEnvelope, CommandProgress, CommandResponse};

#[derive(Debug)]
pub enum IntakeClientError {
    Connect(std::io::Error),
    Write(std::io::Error),
    Read(std::io::Error),
    Parse(serde_json::Error),
    /// No se pudo cargar el token compartido de autenticación (ver
    /// `crate::intake_auth`) para estampar el envelope.
    Auth(std::io::Error),
    /// El agente entendió el envelope pero no pudo ejecutarlo (p.ej.
    /// `command_type` desconocido, o timeout del lado del handler).
    Rejected(String),
    /// El peer cerró la conexión sin mandar una respuesta final.
    NoResponse,
}

impl std::fmt::Display for IntakeClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeClientError::Connect(e) => write!(f, "could not connect to command intake: {e}"),
            IntakeClientError::Write(e) => write!(f, "could not write command envelope: {e}"),
            IntakeClientError::Read(e) => write!(f, "could not read from command intake: {e}"),
            IntakeClientError::Parse(e) => write!(f, "could not parse intake response: {e}"),
            IntakeClientError::Auth(e) => write!(f, "could not load shared auth token: {e}"),
            IntakeClientError::Rejected(msg) => write!(f, "command rejected: {msg}"),
            IntakeClientError::NoResponse => write!(f, "command intake closed without a final response"),
        }
    }
}

impl std::error::Error for IntakeClientError {}

/// Manda `envelope` al intake de `agent_name` y bloquea hasta la respuesta
/// final. Cada `CommandProgress` recibido en el camino se pasa a
/// `on_progress` a medida que llega.
///
/// `envelope.auth_token` se sobrescribe siempre con el token compartido de
/// la máquina (`crate::intake_auth::ensure_token`) — el llamante no
/// necesita conocer ese detalle, igual que el core sella `command_id` en
/// cada `CommandProgress` sin que el handler tenga que hacerlo.
pub fn send_command(
    agent_name: &str,
    envelope: &CommandEnvelope,
    on_progress: impl FnMut(CommandProgress),
) -> Result<CommandResponse, IntakeClientError> {
    let token = crate::intake_auth::ensure_token().map_err(IntakeClientError::Auth)?;
    let mut envelope = envelope.clone();
    envelope.auth_token = token;
    platform::send_command(agent_name, &envelope, on_progress)
}

/// Una línea del socket es progreso, respuesta final, o un rechazo — se
/// distinguen por la forma del JSON, no por un campo de tipo explícito
/// (mismo estilo que `status.rs`: el core no versiona un enum de mensajes).
fn classify_line(line: &str) -> Result<LineKind, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(line)?;
    if value.get("error").is_some() {
        return Ok(LineKind::Error(serde_json::from_value::<ErrorLine>(value)?.error));
    }
    if value.get("duration_ms").is_some() {
        return Ok(LineKind::Response(serde_json::from_value(value)?));
    }
    Ok(LineKind::Progress(serde_json::from_value(value)?))
}

enum LineKind {
    Progress(CommandProgress),
    Response(CommandResponse),
    Error(String),
}

#[derive(serde::Deserialize)]
struct ErrorLine {
    error: String,
}

fn read_lines_until_final(
    reader: impl std::io::BufRead,
    mut on_progress: impl FnMut(CommandProgress),
) -> Result<CommandResponse, IntakeClientError> {
    for line in reader.lines() {
        let line = line.map_err(IntakeClientError::Read)?;
        if line.trim().is_empty() {
            continue;
        }
        match classify_line(&line).map_err(IntakeClientError::Parse)? {
            LineKind::Progress(p) => on_progress(p),
            LineKind::Response(r) => return Ok(r),
            LineKind::Error(e) => return Err(IntakeClientError::Rejected(e)),
        }
    }
    Err(IntakeClientError::NoResponse)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::io::{BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    pub fn send_command(
        agent_name: &str,
        envelope: &CommandEnvelope,
        on_progress: impl FnMut(CommandProgress),
    ) -> Result<CommandResponse, IntakeClientError> {
        let path = crate::command_intake::default_socket_path(agent_name);
        let mut stream = UnixStream::connect(&path).map_err(IntakeClientError::Connect)?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));

        let mut line = serde_json::to_vec(envelope).map_err(IntakeClientError::Parse)?;
        line.push(b'\n');
        stream.write_all(&line).map_err(IntakeClientError::Write)?;
        stream.shutdown(std::net::Shutdown::Write).map_err(IntakeClientError::Write)?;

        read_lines_until_final(BufReader::new(stream), on_progress)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::io::{BufReader, Write};

    pub fn send_command(
        agent_name: &str,
        envelope: &CommandEnvelope,
        on_progress: impl FnMut(CommandProgress),
    ) -> Result<CommandResponse, IntakeClientError> {
        let pipe_name = crate::command_intake::default_socket_path(agent_name);
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_name)
            .map_err(IntakeClientError::Connect)?;

        let mut line = serde_json::to_vec(envelope).map_err(IntakeClientError::Parse)?;
        line.push(b'\n');
        file.write_all(&line).map_err(IntakeClientError::Write)?;

        let reader = BufReader::new(file);
        read_lines_until_final(reader, on_progress)
    }
}
