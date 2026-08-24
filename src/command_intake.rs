//! Intake de comandos: socket local (Unix socket / named pipe en Windows)
//! separado del status socket, por el que un peer local de confianza (hoy:
//! Nexus, reenviando lo que le llega por el túnel) puede pedirle a un agente
//! que ejecute una acción. Misma idea que `status.rs` pero en la otra
//! dirección: campos fijos comunes (`command_id`, `command_type`, `payload`
//! libre) que este crate no interpreta — el agente registra un handler por
//! `command_type` y decide qué significa.
//!
//! Ver `D:\infra\docs\design-command-intake.md` para el porqué de este
//! diseño (cada agente ejecuta lo suyo; este crate solo enruta y da forma).
//!
//! Autenticación: pendiente de decidir (ver el documento de diseño,
//! sección "Autenticación del intake"). Por ahora el socket confía en quien
//! pueda conectarse localmente — no usar todavía para nada que un proceso
//! sin privilegios no debería poder disparar.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Petición de comando tal y como llega al intake. `payload` es de forma
/// libre a propósito — el core no sabe qué es `"os_upgrade"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub command_id: String,
    pub command_type: String,
    pub payload: serde_json::Value,
    /// 0 = sin timeout propio; el core usa un default razonable.
    pub timeout_secs: u32,
}

/// Progreso intermedio de un comando largo (p.ej. `os_upgrade`). Un handler
/// puede emitir cero o más de estos antes del resultado final.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandProgress {
    pub command_id: String,
    /// Libre por agente: "downloading" | "applying" | "verifying", etc.
    pub stage: String,
    pub message: String,
    /// -1 si no aplica.
    pub percent: i32,
}

/// Lo que devuelve un handler. El core añade `command_id` y `duration_ms`
/// para completar el `CommandResponse` final.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CommandOutcome {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self { success: true, stdout: stdout.into(), stderr: String::new(), exit_code: 0 }
    }

    pub fn failed(stderr: impl Into<String>) -> Self {
        Self { success: false, stdout: String::new(), stderr: stderr.into(), exit_code: 1 }
    }
}

/// Resultado final que viaja de vuelta por el socket, un `CommandOutcome`
/// más los campos que el core rellena.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub command_id: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: i64,
}

/// Motivo de rechazo cuando el core no llega a invocar ningún handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IntakeError {
    UnknownCommandType(String),
    Timeout,
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeError::UnknownCommandType(t) => write!(f, "unknown command_type: {t}"),
            IntakeError::Timeout => write!(f, "command timed out"),
        }
    }
}

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type ProgressSender = tokio::sync::mpsc::UnboundedSender<CommandProgress>;
type Handler = Arc<dyn Fn(serde_json::Value, ProgressSender) -> BoxFuture<CommandOutcome> + Send + Sync>;

const DEFAULT_TIMEOUT_SECS: u64 = 300;
/// Cuántos `command_id` recientes recordamos para no repetir una ejecución
/// tras una reconexión del túnel que reenvíe el mismo comando dos veces.
const SEEN_CAPACITY: usize = 256;

struct SeenCommands {
    order: VecDeque<String>,
    cache: HashMap<String, CommandResponse>,
}

impl SeenCommands {
    fn new() -> Self {
        Self { order: VecDeque::new(), cache: HashMap::new() }
    }

    fn get(&self, id: &str) -> Option<CommandResponse> {
        self.cache.get(id).cloned()
    }

    fn insert(&mut self, id: String, resp: CommandResponse) {
        if !self.cache.contains_key(&id) {
            self.order.push_back(id.clone());
            if self.order.len() > SEEN_CAPACITY {
                if let Some(oldest) = self.order.pop_front() {
                    self.cache.remove(&oldest);
                }
            }
        }
        self.cache.insert(id, resp);
    }
}

/// Handle compartido (clonable, `Send + Sync`): el agente registra sus
/// handlers aquí en el arranque, y el servidor del socket lo usa para
/// despachar cada `CommandEnvelope` que llega.
#[derive(Clone)]
pub struct CommandRegistry {
    handlers: Arc<RwLock<HashMap<String, Handler>>>,
    seen: Arc<Mutex<SeenCommands>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self { handlers: Arc::new(RwLock::new(HashMap::new())), seen: Arc::new(Mutex::new(SeenCommands::new())) }
    }

    /// Registra el handler para un `command_type`. `handler` recibe el
    /// `payload` libre del envelope y un canal para emitir `CommandProgress`
    /// mientras trabaja; el `Ok`/`Err` final lo expresa devolviendo
    /// `CommandOutcome { success, .. }`, no con `Result` — un comando que
    /// falla de forma esperada (paquete bloqueado, permiso denegado) no es
    /// un error del transporte.
    pub fn register<F, Fut>(&self, command_type: &str, handler: F)
    where
        F: Fn(serde_json::Value, ProgressSender) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = CommandOutcome> + Send + 'static,
    {
        let boxed: Handler = Arc::new(move |payload, tx| Box::pin(handler(payload, tx)));
        self.handlers.write().expect("command registry lock poisoned").insert(command_type.to_string(), boxed);
    }

    fn get(&self, command_type: &str) -> Option<Handler> {
        self.handlers.read().expect("command registry lock poisoned").get(command_type).cloned()
    }

    /// Ejecuta un envelope: idempotencia, timeout, y despacho al handler
    /// registrado. Cada `CommandProgress` que el handler emita se reenvía
    /// tal cual por `progress_out` a medida que llega, para que el llamante
    /// (el servidor del socket) la vaya escribiendo al peer sin esperar al
    /// resultado final.
    pub async fn dispatch(
        &self,
        envelope: CommandEnvelope,
        progress_out: ProgressSender,
    ) -> Result<CommandResponse, IntakeError> {
        if let Some(cached) = self.seen.lock().expect("seen lock poisoned").get(&envelope.command_id) {
            return Ok(cached);
        }

        let Some(handler) = self.get(&envelope.command_type) else {
            return Err(IntakeError::UnknownCommandType(envelope.command_type));
        };

        let timeout = Duration::from_secs(if envelope.timeout_secs == 0 {
            DEFAULT_TIMEOUT_SECS
        } else {
            envelope.timeout_secs as u64
        });

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CommandProgress>();
        let started = std::time::Instant::now();
        let work = handler(envelope.payload, tx);
        tokio::pin!(work);

        let outcome = loop {
            tokio::select! {
                progress = rx.recv() => {
                    if let Some(mut p) = progress {
                        // El handler no conoce (ni debería tener que pasar)
                        // el `command_id` del envelope que lo invocó — lo
                        // sellamos aquí para que sea siempre correcto sin
                        // que cada handler tenga que acordarse de hacerlo.
                        p.command_id = envelope.command_id.clone();
                        let _ = progress_out.send(p);
                    }
                }
                result = &mut work => {
                    // Drena progreso que haya quedado en el canal antes de cerrar.
                    while let Ok(mut p) = rx.try_recv() {
                        p.command_id = envelope.command_id.clone();
                        let _ = progress_out.send(p);
                    }
                    break result;
                }
                _ = tokio::time::sleep(timeout) => {
                    return Err(IntakeError::Timeout);
                }
            }
        };

        let response = CommandResponse {
            command_id: envelope.command_id.clone(),
            success: outcome.success,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            exit_code: outcome.exit_code,
            duration_ms: started.elapsed().as_millis() as i64,
        };

        self.seen.lock().expect("seen lock poisoned").insert(envelope.command_id, response.clone());
        Ok(response)
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::path::PathBuf;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;
    use tracing::warn;

    /// `/run/sb-agent/<agent>-cmd.sock`, con el mismo fallback a `$TMPDIR`
    /// que el status socket si `/run` no es escribible.
    pub fn default_socket_path(agent_name: &str) -> PathBuf {
        let run_dir = PathBuf::from("/run/sb-agent");
        let base = if std::fs::create_dir_all(&run_dir).is_ok() {
            run_dir
        } else {
            std::env::temp_dir().join("sb-agent")
        };
        let _ = std::fs::create_dir_all(&base);
        base.join(format!("{agent_name}-cmd.sock"))
    }

    pub fn spawn_server(registry: CommandRegistry, socket_path: PathBuf) {
        let _ = std::fs::remove_file(&socket_path);
        tokio::spawn(async move {
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => {
                    warn!(path = %socket_path.display(), error = %e, "command intake: bind failed");
                    return;
                }
            };
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "command intake: accept failed");
                        continue;
                    }
                };
                let registry = registry.clone();
                tokio::spawn(handle_connection(registry, stream));
            }
        });
    }

    async fn handle_connection(registry: CommandRegistry, stream: tokio::net::UnixStream) {
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();

        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }

        let envelope: CommandEnvelope = match serde_json::from_str(line.trim_end()) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "command intake: malformed envelope");
                return;
            }
        };

        run_and_reply(&registry, envelope, &mut write_half).await;
    }

    async fn run_and_reply<W: tokio::io::AsyncWrite + Unpin>(
        registry: &CommandRegistry,
        envelope: CommandEnvelope,
        out: &mut W,
    ) {
        super::run_and_reply_generic(registry, envelope, out).await;
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;
    use tracing::warn;

    /// `\\.\pipe\sb-agent-cmd-<agent>`
    pub fn default_socket_path(agent_name: &str) -> String {
        format!(r"\\.\pipe\sb-agent-cmd-{agent_name}")
    }

    pub fn spawn_server(registry: CommandRegistry, pipe_name: String) {
        tokio::spawn(async move {
            loop {
                let server = match ServerOptions::new().first_pipe_instance(false).create(&pipe_name) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(pipe = %pipe_name, error = %e, "command intake: pipe create failed");
                        return;
                    }
                };
                if let Err(e) = server.connect().await {
                    warn!(error = %e, "command intake: pipe connect failed");
                    continue;
                }
                let registry = registry.clone();
                tokio::spawn(handle_connection(registry, server));
            }
        });
    }

    async fn handle_connection(registry: CommandRegistry, pipe: tokio::net::windows::named_pipe::NamedPipeServer) {
        let (read_half, mut write_half) = tokio::io::split(pipe);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();

        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }

        let envelope: CommandEnvelope = match serde_json::from_str(line.trim_end()) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "command intake: malformed envelope");
                return;
            }
        };

        super::run_and_reply_generic(&registry, envelope, &mut write_half).await;
    }
}

/// Corre el comando y escribe al peer: cero o más líneas `CommandProgress`
/// seguidas de una línea `CommandResponse` final (o de un envelope de error
/// con la forma `{"error": "..."}` si ni siquiera se llegó a ejecutar).
/// Compartido entre Unix y Windows — la única diferencia entre plataformas
/// es cómo se acepta la conexión, no el protocolo por encima.
async fn run_and_reply_generic<W: tokio::io::AsyncWrite + Unpin>(
    registry: &CommandRegistry,
    envelope: CommandEnvelope,
    out: &mut W,
) {
    use tokio::io::AsyncWriteExt;

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<CommandProgress>();
    let dispatch = registry.dispatch(envelope, progress_tx);
    tokio::pin!(dispatch);

    let result = loop {
        tokio::select! {
            progress = progress_rx.recv() => {
                if let Some(p) = progress {
                    if let Ok(mut json) = serde_json::to_vec(&p) {
                        json.push(b'\n');
                        if out.write_all(&json).await.is_err() {
                            return;
                        }
                    }
                }
            }
            result = &mut dispatch => {
                // Drena progreso que haya quedado en el canal antes de cerrar.
                while let Ok(p) = progress_rx.try_recv() {
                    if let Ok(mut json) = serde_json::to_vec(&p) {
                        json.push(b'\n');
                        let _ = out.write_all(&json).await;
                    }
                }
                break result;
            }
        }
    };

    match result {
        Ok(response) => {
            if let Ok(mut json) = serde_json::to_vec(&response) {
                json.push(b'\n');
                let _ = out.write_all(&json).await;
            }
        }
        Err(e) => {
            let err = serde_json::json!({ "error": e.to_string() });
            if let Ok(mut json) = serde_json::to_vec(&err) {
                json.push(b'\n');
                let _ = out.write_all(&json).await;
            }
        }
    }
    let _ = out.shutdown().await;
}

pub use platform::{default_socket_path, spawn_server};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_runs_registered_handler() {
        let registry = CommandRegistry::new();
        registry.register("echo", |payload, _progress| async move {
            CommandOutcome::ok(payload.to_string())
        });

        let envelope = CommandEnvelope {
            command_id: "1".to_string(),
            command_type: "echo".to_string(),
            payload: serde_json::json!({"hello": "world"}),
            timeout_secs: 5,
        };

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let response = registry.dispatch(envelope, tx).await.unwrap();
        assert!(response.success);
        assert_eq!(response.stdout, r#"{"hello":"world"}"#);
    }

    #[tokio::test]
    async fn dispatch_rejects_unknown_command_type() {
        let registry = CommandRegistry::new();
        let envelope = CommandEnvelope {
            command_id: "1".to_string(),
            command_type: "does_not_exist".to_string(),
            payload: serde_json::Value::Null,
            timeout_secs: 5,
        };

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let err = registry.dispatch(envelope, tx).await.unwrap_err();
        assert!(matches!(err, IntakeError::UnknownCommandType(t) if t == "does_not_exist"));
    }

    #[tokio::test]
    async fn dispatch_is_idempotent_for_repeated_command_id() {
        let registry = CommandRegistry::new();
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls2 = calls.clone();
        registry.register("count", move |_payload, _progress| {
            let calls = calls2.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                CommandOutcome::ok("done")
            }
        });

        let make_envelope = || CommandEnvelope {
            command_id: "same-id".to_string(),
            command_type: "count".to_string(),
            payload: serde_json::Value::Null,
            timeout_secs: 5,
        };

        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        registry.dispatch(make_envelope(), tx1).await.unwrap();
        registry.dispatch(make_envelope(), tx2).await.unwrap();

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_reports_progress() {
        let registry = CommandRegistry::new();
        registry.register("slow", |_payload, progress| async move {
            let _ = progress.send(CommandProgress {
                command_id: "1".to_string(),
                stage: "working".to_string(),
                message: "halfway".to_string(),
                percent: 50,
            });
            CommandOutcome::ok("done")
        });

        let envelope = CommandEnvelope {
            command_id: "1".to_string(),
            command_type: "slow".to_string(),
            payload: serde_json::Value::Null,
            timeout_secs: 5,
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let response = registry.dispatch(envelope, tx).await.unwrap();
        assert!(response.success);
        let seen: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].percent, 50);
    }
}
