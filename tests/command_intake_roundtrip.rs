use sb_agent_core::command_intake::{default_socket_path, spawn_server, CommandEnvelope, CommandOutcome, CommandRegistry};
use sb_agent_core::command_intake_client::send_command;
use std::time::Duration;

#[tokio::test]
async fn command_intake_roundtrip_with_progress() {
    let agent_name = "test-cmd-roundtrip-agent";
    let registry = CommandRegistry::new();
    registry.register("echo_upper", |payload, progress| async move {
        let _ = progress.send(sb_agent_core::command_intake::CommandProgress {
            command_id: "n/a".to_string(),
            stage: "working".to_string(),
            message: "uppercasing".to_string(),
            percent: 50,
        });
        let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        CommandOutcome::ok(text.to_uppercase())
    });

    spawn_server(registry, default_socket_path(agent_name));
    // Dar tiempo al listener a bindear antes de conectar.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let envelope = CommandEnvelope {
        command_id: "roundtrip-1".to_string(),
        command_type: "echo_upper".to_string(),
        payload: serde_json::json!({"text": "hola"}),
        timeout_secs: 5,
        auth_token: String::new(),
    };

    let agent_name_owned = agent_name.to_string();
    let response = tokio::task::spawn_blocking(move || {
        let mut progress_seen = Vec::new();
        let result = send_command(&agent_name_owned, &envelope, |p| progress_seen.push(p));
        (result, progress_seen)
    })
    .await
    .unwrap();

    let (response, progress_seen) = response;
    let response = response.expect("send_command should succeed against a live server");

    assert!(response.success);
    assert_eq!(response.stdout, "HOLA");
    assert_eq!(progress_seen.len(), 1);
    assert_eq!(progress_seen[0].percent, 50);
}

#[tokio::test]
async fn command_intake_reports_unknown_command_type() {
    let agent_name = "test-cmd-roundtrip-unknown-agent";
    let registry = CommandRegistry::new();
    spawn_server(registry, default_socket_path(agent_name));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let envelope = CommandEnvelope {
        command_id: "roundtrip-2".to_string(),
        command_type: "does_not_exist".to_string(),
        payload: serde_json::Value::Null,
        timeout_secs: 5,
        auth_token: String::new(),
    };

    let agent_name_owned = agent_name.to_string();
    let result = tokio::task::spawn_blocking(move || send_command(&agent_name_owned, &envelope, |_| {}))
        .await
        .unwrap();

    let err = result.expect_err("unknown command_type should be rejected");
    assert!(matches!(err, sb_agent_core::command_intake_client::IntakeClientError::Rejected(_)));
}

/// `send_command` siempre estampa el token correcto, así que para probar el
/// rechazo hay que escribir el envelope a mano por el socket/pipe, saltando
/// el cliente de más arriba.
#[tokio::test]
async fn command_intake_rejects_wrong_auth_token() {
    let agent_name = "test-cmd-roundtrip-badtoken-agent";
    let registry = CommandRegistry::new();
    registry.register("echo", |payload, _progress| async move { CommandOutcome::ok(payload.to_string()) });
    spawn_server(registry, default_socket_path(agent_name));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let envelope = CommandEnvelope {
        command_id: "roundtrip-3".to_string(),
        command_type: "echo".to_string(),
        payload: serde_json::Value::Null,
        timeout_secs: 5,
        auth_token: "definitely-not-the-real-token".to_string(),
    };

    let line = tokio::task::spawn_blocking(move || {
        let mut line = serde_json::to_vec(&envelope).unwrap();
        line.push(b'\n');
        line
    })
    .await
    .unwrap();

    let response_text = tokio::task::spawn_blocking(move || raw_send(agent_name, &line)).await.unwrap();

    assert!(response_text.contains("error"), "expected an error line, got: {response_text}");
    assert!(response_text.contains("auth_token"), "expected the auth_token rejection reason, got: {response_text}");
}

#[cfg(unix)]
fn raw_send(agent_name: &str, line: &[u8]) -> String {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(default_socket_path(agent_name)).unwrap();
    stream.write_all(line).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

#[cfg(windows)]
fn raw_send(agent_name: &str, line: &[u8]) -> String {
    use std::io::{Read, Write};

    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(default_socket_path(agent_name)).unwrap();
    file.write_all(line).unwrap();
    let mut out = String::new();
    file.read_to_string(&mut out).unwrap();
    out
}
