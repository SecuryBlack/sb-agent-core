//! Los ficheros rotados se llaman `<agente>.log.AAAA-MM-DD`: la limpieza
//! anterior buscaba la extensión `.log` y nunca borraba ninguno.

#[test]
fn keeps_the_last_seven_log_files_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    for day in 1..=20 {
        std::fs::write(dir.path().join(format!("agent.log.2026-01-{day:02}")), "x").unwrap();
    }
    std::fs::write(dir.path().join("config.toml"), "token = \"x\"").unwrap();

    sb_agent_core::logging::init("agent", dir.path(), "info");

    let logs = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("agent.log."))
        .count();
    assert_eq!(logs, 7);
    assert!(dir.path().join("config.toml").exists());
}
