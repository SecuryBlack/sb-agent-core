//! Autenticación del intake de comandos (`command_intake`): un token
//! compartido por todos los agentes de la misma máquina, guardado en un
//! fichero de solo-root/admin. Ver `D:\infra\docs\design-command-intake.md`,
//! sección "Autenticación del intake" — se eligió la opción de token en
//! disco porque hace falta de todos modos en Windows (named pipes no tienen
//! permisos POSIX), así que se usa el mismo mecanismo en ambos SO en vez de
//! añadir un grupo Unix compartido como vía aparte.
//!
//! No es un token por agente: el primero en arrancar (nexus o FerroSentry,
//! el que sea) lo genera; el resto simplemente lo lee. Todos deben poder
//! leer el fichero — en la práctica implica que corran con el mismo usuario
//! o con permisos equivalentes sobre él.

use rand::RngCore;
use std::io::{self, Read, Write};
use std::path::PathBuf;

#[cfg(unix)]
pub fn token_path() -> PathBuf {
    PathBuf::from("/etc/sb-agent/intake.token")
}

#[cfg(windows)]
pub fn token_path() -> PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(base).join("sb-agent").join("intake.token")
}

/// Lee el token compartido, generándolo si todavía no existe. Pensado para
/// llamarse una vez al arrancar el servidor del intake y una vez por cada
/// llamada del cliente (`command_intake_client`) — es una lectura de
/// fichero pequeña, no hace falta cachear entre procesos.
///
/// La creación es atómica (`create_new`, falla si el fichero ya existe) a
/// propósito: si dos agentes arrancan a la vez y ninguno encuentra el
/// fichero, ambos generarían un token distinto sin esto, y el que escribiera
/// segundo pisaría al primero — dejando al primer agente con un token que ya
/// no coincide con el del disco. Con `create_new`, quien pierde la carrera
/// simplemente relee lo que escribió el ganador.
pub fn ensure_token() -> io::Result<String> {
    let path = token_path();

    if let Some(token) = read_existing(&path)? {
        return Ok(token);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let token = generate_token();
    match create_new_token_file(&path, &token) {
        Ok(()) => Ok(token),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            // Perdimos la carrera de creación — el ganador puede seguir
            // escribiendo sus bytes en este mismo instante, así que
            // reintentamos la lectura un par de veces antes de rendirnos.
            for attempt in 0..5 {
                if let Some(token) = read_existing(&path)? {
                    return Ok(token);
                }
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            Err(e)
        }
        Err(e) => Err(e),
    }
}

fn read_existing(path: &std::path::Path) -> io::Result<Option<String>> {
    match std::fs::File::open(path) {
        Ok(mut f) => {
            let mut contents = String::new();
            f.read_to_string(&mut contents)?;
            let trimmed = contents.trim();
            Ok(if trimmed.is_empty() { None } else { Some(trimmed.to_string()) })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn create_new_token_file(path: &std::path::Path, token: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(token.as_bytes())
}

#[cfg(windows)]
fn create_new_token_file(path: &std::path::Path, token: &str) -> io::Result<()> {
    // A diferencia de Unix, aquí no fijamos una ACL explícita todavía —
    // confiamos en que sólo un admin puede escribir bajo `ProgramData` en
    // instalaciones por defecto. Restringir la ACL del fichero en sí queda
    // pendiente (ver TODO.md) si esto no resulta suficiente en la práctica.
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(token.as_bytes())
}

/// Comparación en tiempo constante — evita que un atacante local con acceso
/// de red al socket/pipe pueda deducir el token byte a byte cronometrando
/// respuestas de rechazo.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equal_strings() {
        assert!(constant_time_eq("abc123", "abc123"));
    }

    #[test]
    fn constant_time_eq_rejects_different_strings() {
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("short", "muchlongerstring"));
    }
}
