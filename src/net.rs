//! Comprobación de conectividad TCP, sin nada específico de OTLP — la usan
//! el buffer offline y cualquier agente que necesite saber "¿mi destino
//! responde ahora mismo?" antes de decidir si bufferar o enviar.

use tokio::net::TcpStream;
use tracing::warn;

/// Intenta conectar por TCP al host:puerto extraído de `endpoint`. Resuelve
/// el hostname y prioriza IPv4 para no colgarse en hosts sin IPv6. Devuelve
/// true si es alcanzable.
pub async fn is_reachable(endpoint: &str) -> bool {
    let Some(addr) = parse_host_port(endpoint) else {
        warn!(%endpoint, "could not parse endpoint host:port");
        return false;
    };

    let mut addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host(&addr).await {
        Ok(iter) => iter.collect(),
        Err(e) => {
            warn!(%addr, error = %e, "DNS resolution failed");
            return false;
        }
    };

    addrs.sort_by_key(|a| if a.is_ipv4() { 0u8 } else { 1u8 });

    for sa in addrs {
        match tokio::time::timeout(std::time::Duration::from_secs(2), TcpStream::connect(sa)).await {
            Ok(Ok(_)) => return true,
            Ok(Err(e)) => warn!(%sa, error = %e, "reachability check failed"),
            Err(_) => warn!(%sa, "reachability check timed out"),
        }
    }
    false
}

/// Extrae "host:puerto" de URLs como "http://host:4317", "https://host/v1/x"
/// o directamente "host:puerto".
pub fn parse_host_port(endpoint: &str) -> Option<String> {
    let raw = endpoint.trim();
    let without_scheme = raw.trim_start_matches("https://").trim_start_matches("http://");

    let authority = without_scheme.split('/').next()?.split('?').next()?.split('#').next()?.trim();

    if authority.is_empty() {
        return None;
    }

    let addr = if authority.contains(':') {
        authority.to_string()
    } else {
        let default_port = if raw.starts_with("https://") { 443 } else { 4317 };
        format!("{authority}:{default_port}")
    };

    Some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_port() {
        assert_eq!(parse_host_port("http://localhost:4317"), Some("localhost:4317".to_string()));
        assert_eq!(parse_host_port("https://ingest.example.dev"), Some("ingest.example.dev:443".to_string()));
        assert_eq!(
            parse_host_port("https://ingest.example.dev/v1/metrics"),
            Some("ingest.example.dev:443".to_string())
        );
        assert_eq!(
            parse_host_port("https://ingest.example.dev:4317/v1/metrics"),
            Some("ingest.example.dev:4317".to_string())
        );
        assert_eq!(parse_host_port("ingest.example.dev:4317"), Some("ingest.example.dev:4317".to_string()));
    }
}
