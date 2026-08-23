use sb_agent_core::status::{spawn_server, StatusHandle};
use sb_agent_core::status_client::read_once;
use std::time::Duration;

#[tokio::test]
async fn status_socket_roundtrip() {
    let agent_name = "test-roundtrip-agent";
    let handle = StatusHandle::new(agent_name, "9.9.9");
    handle.set_state("running");
    handle.set_details(serde_json::json!({"hello": "world"}));

    spawn_server(handle.clone(), sb_agent_core::status::default_socket_path(agent_name));
    // Dar tiempo al listener a bindear antes de conectar.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let payload = tokio::task::spawn_blocking(move || read_once(agent_name))
        .await
        .unwrap()
        .expect("read_once should succeed against a live server");

    assert_eq!(payload.agent, agent_name);
    assert_eq!(payload.version, "9.9.9");
    assert_eq!(payload.state, "running");
    assert_eq!(payload.details["hello"], "world");
}
