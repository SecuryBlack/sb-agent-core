//! Si esto falla, el updater ha dejado de exigir firma (o la exige con claves
//! que no sirven). Reproduce lo que hace `self_update` tras descargar:
//! `collect_keys` + `verify_tar` con el nombre del asset como contexto.

use sb_agent_core::updater::VERIFYING_KEYS;
use std::io::Cursor;
use zipsign_api::sign::copy_and_sign_tar;
use zipsign_api::verify::{collect_keys, verify_tar};
use zipsign_api::SigningKey;

const ASSET: &[u8] = b"oxipulse-x86_64-unknown-linux-gnu.tar.gz";

#[test]
fn production_keys_are_valid_and_distinct() {
    // Con la lista vacía self_update no verifica nada: este es el check clave.
    assert!(!VERIFYING_KEYS.is_empty());
    let keys = collect_keys(VERIFYING_KEYS.iter().copied().map(Ok))
        .expect("todas las claves deben ser VerifyingKey ed25519 válidas");
    assert!(keys.iter().all(|k| !k.is_weak()));
    assert_ne!(VERIFYING_KEYS[0], VERIFYING_KEYS[1]);
    // ponytail: check textual; si se borra la llamada, las claves no sirven de nada.
    assert!(include_str!("../src/updater.rs").contains(".verifying_keys(VERIFYING_KEYS)"));
}

#[test]
fn only_production_keys_verify() {
    // La firma no mira el contenido del tar.gz, solo los bytes: basta con un
    // payload cualquiera para comprobar quién la acepta.
    let test_key = SigningKey::from_bytes(&[7; 32]);
    let mut signed = Cursor::new(Vec::new());
    copy_and_sign_tar(
        &mut Cursor::new(b"payload de prueba".to_vec()),
        &mut signed,
        std::slice::from_ref(&test_key),
        Some(ASSET),
    )
    .unwrap();

    let prod = collect_keys(VERIFYING_KEYS.iter().copied().map(Ok)).unwrap();
    assert!(
        verify_tar(&mut signed, &prod, Some(ASSET)).is_err(),
        "un archivo firmado con una clave ajena no debe pasar"
    );

    let test = [test_key.verifying_key()];
    assert!(verify_tar(&mut signed, &test, Some(ASSET)).is_ok());
    // El contexto (nombre del asset) también cuenta: no se cruzan binarios.
    assert!(verify_tar(&mut signed, &test, Some(b"otro-agente.tar.gz")).is_err());
}
