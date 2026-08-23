//! Manejo común de `--version`/`-V`, `status` y `top` como primer argumento.
//! Cada agente llamaba a esto con su propio nombre y versión copy-pasteado;
//! ahora es una función parametrizada, igual que el resto del crate.

/// Comprueba `std::env::args()[1]` contra los comandos comunes. Si coincide,
/// actúa y termina el proceso (`std::process::exit`) — nunca devuelve en ese
/// caso. Si no coincide con nada (incluyendo "sin argumentos"), devuelve el
/// control al caller para que siga con su arranque normal de servicio/consola.
///
/// `bin_name` es el nombre que se imprime en `--version` (puede diferir del
/// nombre usado para el socket/status si el binario se llama distinto).
pub fn dispatch_common_args(agent_name: &str, bin_name: &str, version: &str) {
    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 {
        return;
    }

    match args[1].as_str() {
        "--version" | "-V" => {
            println!("{bin_name} {version}");
            std::process::exit(0);
        }
        "status" => match crate::status_client::read_once(agent_name) {
            Ok(payload) => {
                println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("[{agent_name}] {e}");
                std::process::exit(1);
            }
        },
        "top" => {
            if let Err(e) = crate::tui::run_top(agent_name) {
                eprintln!("[{agent_name}] {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        _ => {}
    }
}
