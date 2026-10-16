//! Órdenes firmadas (S4). Contrato: `D:\sb-wiki\Tecnica\S4 - Ordenes firmadas.md`.
//!
//! La API firma cada orden con ed25519 sobre este mensaje (UTF-8, `\n`
//! entre líneas, sin salto final):
//!
//! ```text
//! sb-cmd-v1
//! <command_id>
//! <token_hash>          sha256 hex del token del config del agente
//! <target_agent>        "" para órdenes que ejecuta Nexus
//! <command_type>
//! <expires_at>          segundos Unix
//! <sha256 hex del payload canónico>
//! ```
//!
//! El agente recalcula el payload canónico a partir de lo que recibe
//! (JSONB reordena las claves), así que solo cuentan los valores.
//!
//! Limitación del payload canónico: se serializa con `serde_json` (claves
//! ordenadas, compacto, UTF-8 sin escapar, `\uXXXX` en minúscula para
//! caracteres de control). Coincide con
//! `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
//! de Python para strings, bools, null y enteros dentro de i64/u64. Los
//! floats NO son portables (`1e20` vs `1e+20`, `5.0` que JSONB deja en `5`),
//! y enteros fuera de i64/u64 pierden precisión: los payloads firmados deben
//! usar solo enteros. Si un payload con floats no verifica, es esto.

use crate::command_intake::CommandEnvelope;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Claves públicas ed25519 de la API que firma las órdenes (dos huecos para
/// rotar, como `updater::VERIFYING_KEYS`). Los huecos a cero se ignoran.
///
/// La clave privada se generó en el servidor de la API (`COMMAND_SIGNING_KEY`,
/// 16-10-2026) y nunca sale de allí; aquí solo va la pública.
pub const SIGNING_KEYS: [[u8; 32]; 2] = [
    // sb-cmd-primary (16-10-2026)
    [
        0xab, 0xdd, 0x6b, 0xb8, 0xe3, 0x4b, 0xb6, 0xde, 0xc4, 0x43, 0x2b, 0xa3, 0x16, 0x54, 0xc1, 0x9d,
        0xdf, 0x4f, 0x06, 0x1a, 0xda, 0x4a, 0xef, 0xf7, 0x70, 0x54, 0xa9, 0x30, 0xd9, 0x1c, 0xfa, 0x06,
    ],
    // sb-cmd-backup (sin usar: para rotar la clave)
    [0u8; 32],
];

/// Transición: `false` acepta órdenes sin firma con un aviso; la siguiente
/// versión de los agentes sale con `true`. No configurable desde el servidor.
pub const REQUIRE_SIGNED_COMMANDS: bool = false;

/// Margen de reloj entre la API y el agente al comprobar `expires_at`.
pub const CLOCK_SKEW_SECS: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerifyError {
    /// La orden no trae firma (y se exige).
    MissingSignature,
    /// Trae firma pero no `expires_at`, que forma parte del mensaje firmado.
    MissingExpiry,
    Expired,
    /// La firma no es base64 válido de 64 bytes.
    MalformedSignature,
    /// Ninguna de las claves configuradas valida la firma.
    InvalidSignature,
    /// `SIGNING_KEYS` está a cero: no hay con qué verificar.
    NoKeysConfigured,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            VerifyError::MissingSignature => "command is not signed",
            VerifyError::MissingExpiry => "signed command has no expires_at",
            VerifyError::Expired => "command expired",
            VerifyError::MalformedSignature => "malformed command signature",
            VerifyError::InvalidSignature => "invalid command signature",
            VerifyError::NoKeysConfigured => "no command signing keys configured",
        };
        f.write_str(s)
    }
}

impl std::error::Error for VerifyError {}

/// Lo que el servidor del intake necesita para verificar: a qué servidor y a
/// qué agente (nombre "de producto" de `server_commands.target_agent`, p.ej.
/// `"ferrosentry"`) van atadas las órdenes que acepta.
#[derive(Debug, Clone, Default)]
pub struct SigningContext {
    /// Token del config.toml del agente (el mismo que usa contra la API).
    pub server_token: String,
    pub target_agent: String,
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// JSON canónico: claves ordenadas a todos los niveles, compacto, UTF-8 sin
/// escapar. Ver la limitación de floats en la doc del módulo.
pub fn canonical_json(value: &serde_json::Value) -> String {
    fn sorted(v: &serde_json::Value) -> serde_json::Value {
        match v {
            // Ordenamos a mano por si otra dependencia activa `preserve_order`.
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                serde_json::Value::Object(entries.into_iter().map(|(k, v)| (k.clone(), sorted(v))).collect())
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    // Con `preserve_order` el Map mantiene el orden de inserción (ya
    // ordenado); sin él es un BTreeMap, que ordena igual (bytes UTF-8 =
    // orden de code points, igual que `sort_keys` de Python).
    serde_json::to_string(&sorted(value)).expect("serializing a serde_json::Value cannot fail")
}

/// Mensaje exacto que firma la API.
pub fn build_message(
    command_id: &str,
    server_token: &str,
    target_agent: &str,
    command_type: &str,
    expires_at: i64,
    payload: &serde_json::Value,
) -> String {
    format!(
        "sb-cmd-v1\n{command_id}\n{}\n{target_agent}\n{command_type}\n{expires_at}\n{}",
        sha256_hex(server_token.as_bytes()),
        sha256_hex(canonical_json(payload).as_bytes()),
    )
}

/// Verifica firma y caducidad de `envelope` para este servidor/agente.
/// `now` en segundos Unix. Política de transición aparte: ver [`enforce`].
pub fn verify_command(
    envelope: &CommandEnvelope,
    target_agent: &str,
    server_token: &str,
    now: i64,
) -> Result<(), VerifyError> {
    verify_with_keys(envelope, target_agent, server_token, now, &SIGNING_KEYS)
}

fn verify_with_keys(
    envelope: &CommandEnvelope,
    target_agent: &str,
    server_token: &str,
    now: i64,
    keys: &[[u8; 32]],
) -> Result<(), VerifyError> {
    if let Some(exp) = envelope.expires_at {
        if now > exp + CLOCK_SKEW_SECS {
            return Err(VerifyError::Expired);
        }
    }
    let sig_b64 = envelope.signature.as_deref().ok_or(VerifyError::MissingSignature)?;
    let expires_at = envelope.expires_at.ok_or(VerifyError::MissingExpiry)?;
    let sig_bytes: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(VerifyError::MalformedSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);

    let message = build_message(
        &envelope.command_id,
        server_token,
        target_agent,
        &envelope.command_type,
        expires_at,
        &envelope.payload,
    );

    let mut any_key = false;
    for key in keys.iter().filter(|k| **k != [0u8; 32]) {
        any_key = true;
        if let Ok(vk) = VerifyingKey::from_bytes(key) {
            if vk.verify_strict(message.as_bytes(), &signature).is_ok() {
                return Ok(());
            }
        }
    }
    Err(if any_key { VerifyError::InvalidSignature } else { VerifyError::NoKeysConfigured })
}

/// Política del intake sobre [`verify_command`]: mientras
/// `REQUIRE_SIGNED_COMMANDS` sea `false`, cualquier fallo se acepta con aviso.
/// Rechazar una firma inválida no protege nada mientras se acepten órdenes
/// sin firma (basta con quitarla), y así un token que no coincide con el del
/// servidor se ve en el log antes de activar el rechazo, en vez de romper.
pub fn enforce(envelope: &CommandEnvelope, ctx: &SigningContext, now: i64) -> Result<(), VerifyError> {
    enforce_with(envelope, ctx, now, &SIGNING_KEYS, REQUIRE_SIGNED_COMMANDS)
}

fn enforce_with(
    envelope: &CommandEnvelope,
    ctx: &SigningContext,
    now: i64,
    keys: &[[u8; 32]],
    require: bool,
) -> Result<(), VerifyError> {
    match verify_with_keys(envelope, &ctx.target_agent, &ctx.server_token, now, keys) {
        Ok(()) => Ok(()),
        Err(e) if !require => {
            tracing::warn!(command_id = %envelope.command_id, reason = %e, "accepting unverified command (REQUIRE_SIGNED_COMMANDS = false)");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    const NOW: i64 = 1_800_000_000;
    // Clave de test generada aquí, sin relación con ninguna clave real.
    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn ctx() -> SigningContext {
        SigningContext { server_token: "srv-token".into(), target_agent: "ferrosentry".into() }
    }

    fn signed(payload: serde_json::Value, expires_at: i64) -> CommandEnvelope {
        let msg = build_message("cmd-1", "srv-token", "ferrosentry", "os_upgrade", expires_at, &payload);
        let sig = base64::engine::general_purpose::STANDARD.encode(test_key().sign(msg.as_bytes()).to_bytes());
        CommandEnvelope {
            command_id: "cmd-1".into(),
            command_type: "os_upgrade".into(),
            payload,
            signature: Some(sig),
            expires_at: Some(expires_at),
            ..Default::default()
        }
    }

    fn keys() -> [[u8; 32]; 2] {
        [[0u8; 32], test_key().verifying_key().to_bytes()]
    }

    fn verify(env: &CommandEnvelope, token: &str) -> Result<(), VerifyError> {
        verify_with_keys(env, "ferrosentry", token, NOW, &keys())
    }

    #[test]
    fn canonical_sorts_keys_at_all_levels() {
        let v: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":{"z":[{"y":2,"x":1}],"c":null},"A":true}"#).unwrap();
        assert_eq!(canonical_json(&v), r#"{"A":true,"a":{"c":null,"z":[{"x":1,"y":2}]},"b":1}"#);
    }

    #[test]
    fn canonical_keeps_unicode_unescaped() {
        let v = json!({"ñ": "año €", "e": "a\"b\\c\n\u{1}"});
        assert_eq!(canonical_json(&v), "{\"e\":\"a\\\"b\\\\c\\n\\u0001\",\"ñ\":\"año €\"}");
    }

    #[test]
    fn message_matches_contract() {
        let msg = build_message("id", "tok", "", "deploy_app", 123, &json!({"b": 1, "a": 2}));
        let lines: Vec<_> = msg.split('\n').collect();
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[0], "sb-cmd-v1");
        assert_eq!(lines[1], "id");
        assert_eq!(lines[2], sha256_hex(b"tok"));
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "deploy_app");
        assert_eq!(lines[5], "123");
        assert_eq!(lines[6], sha256_hex(br#"{"a":2,"b":1}"#));
        assert!(!msg.ends_with('\n'));
    }

    #[test]
    fn valid_signature_verifies() {
        assert_eq!(verify(&signed(json!({"pkg": "nginx"}), NOW + 900), "srv-token"), Ok(()));
    }

    #[test]
    fn payload_key_order_does_not_matter() {
        let mut env = signed(json!({"a": 1, "b": {"d": 2, "c": 3}}), NOW + 900);
        env.payload = serde_json::from_str(r#"{"b":{"c":3,"d":2},"a":1}"#).unwrap();
        assert_eq!(verify(&env, "srv-token"), Ok(()));
    }

    #[test]
    fn wrong_token_hash_rejected() {
        assert_eq!(verify(&signed(json!({}), NOW + 900), "other-server"), Err(VerifyError::InvalidSignature));
    }

    #[test]
    fn wrong_target_agent_rejected() {
        let env = signed(json!({}), NOW + 900);
        assert_eq!(verify_with_keys(&env, "cromoforge", "srv-token", NOW, &keys()), Err(VerifyError::InvalidSignature));
    }

    #[test]
    fn tampered_payload_rejected() {
        let mut env = signed(json!({"image": "nginx"}), NOW + 900);
        env.payload = json!({"image": "evil"});
        assert_eq!(verify(&env, "srv-token"), Err(VerifyError::InvalidSignature));
    }

    #[test]
    fn expired_rejected_with_skew() {
        assert_eq!(verify(&signed(json!({}), NOW - CLOCK_SKEW_SECS), "srv-token"), Ok(()));
        assert_eq!(verify(&signed(json!({}), NOW - CLOCK_SKEW_SECS - 1), "srv-token"), Err(VerifyError::Expired));
    }

    #[test]
    fn missing_signature_accepted_only_when_not_required() {
        let env = CommandEnvelope { command_id: "x".into(), command_type: "t".into(), ..Default::default() };
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), false), Ok(()));
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), true), Err(VerifyError::MissingSignature));
    }

    #[test]
    fn bad_signature_accepted_only_when_not_required() {
        let mut env = signed(json!({}), NOW + 900);
        env.command_type = "deploy_app".into();
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), false), Ok(()));
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), true), Err(VerifyError::InvalidSignature));
        env.signature = Some("not base64!".into());
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), false), Ok(()));
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), true), Err(VerifyError::MalformedSignature));
    }

    #[test]
    fn expired_accepted_only_when_not_required() {
        let env = CommandEnvelope { expires_at: Some(NOW - 3600), ..Default::default() };
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), false), Ok(()));
        assert_eq!(enforce_with(&env, &ctx(), NOW, &keys(), true), Err(VerifyError::Expired));
    }

    /// Firma hecha en el servidor de la API con la clave real (16-10-2026):
    /// comprueba que `SIGNING_KEYS[0]` es su pública.
    #[test]
    fn production_key_verifies_api_signature() {
        let env = CommandEnvelope {
            command_id: "00000000-0000-0000-0000-0000000000aa".into(),
            command_type: "os_upgrade".into(),
            payload: json!({"a": 1}),
            signature: Some(
                "qXArmqL7jQ0nvB3YGYHYAlyNMKeswYjDoGTPCMl85oc383JT2pRK7HRd3eUj9VqDOwkv6ni/FXzqN0fo1uiNCQ==".into(),
            ),
            expires_at: Some(1_900_000_000),
            ..Default::default()
        };
        assert_eq!(verify_command(&env, "ferrosentry", "srv-token", NOW), Ok(()));
        assert_eq!(verify_command(&env, "ferrosentry", "otro-token", NOW), Err(VerifyError::InvalidSignature));
    }

    /// Vectores generados en la API con
    /// `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
    #[test]
    fn python_test_vectors() {
        let v1 = json!({});
        assert_eq!(canonical_json(&v1), "{}");
        assert_eq!(sha256_hex(canonical_json(&v1).as_bytes()), "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a");

        let v2: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":{"z":[3,{"y":true,"x":null}],"m":"ñ€ü"}}"#).unwrap();
        assert_eq!(canonical_json(&v2), r#"{"a":{"m":"ñ€ü","z":[3,{"x":null,"y":true}]},"b":1}"#);
        assert_eq!(sha256_hex(canonical_json(&v2).as_bytes()), "03af9c61cbe39e0fe767a5507c2396c41ef5557d1ea0c1dc4d754d448d993664");

        let v3 = json!({"keep_images": 3, "env": {"K": "línea\n\"q\""}, "deployment_id": "d1",
                        "app": {"port": 3000, "name": "mi-web", "domain": null}});
        assert_eq!(
            canonical_json(&v3),
            r#"{"app":{"domain":null,"name":"mi-web","port":3000},"deployment_id":"d1","env":{"K":"línea\n\"q\""},"keep_images":3}"#
        );
        assert_eq!(sha256_hex(canonical_json(&v3).as_bytes()), "2ba331735a1fb4e73b8f2f949dedd2414dbf638ec707cd9ba95ba49b205c1e33");

        let msg = build_message("00000000-0000-0000-0000-000000000001", "abc", "cromoforge", "deploy_app", 1_760_000_000, &v2);
        assert_eq!(sha256_hex(msg.as_bytes()), "5cae16ac08266454b8b9bfc4f9dd0eb888349c29dcbcb0c1957990cd5f859020");
    }
}
