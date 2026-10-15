# Changelog

## [Unreleased]

### Added
- `command_signing` (S4): `verify_command`, `enforce`, `canonical_json`, `build_message`, `SIGNING_KEYS` (two slots, all-zero until the API's public key is filled in at deploy time) and `REQUIRE_SIGNED_COMMANDS = false`.
- `CommandEnvelope.signature` (base64 ed25519) and `CommandEnvelope.expires_at` (Unix secs), both optional; `CommandEnvelope` now derives `Default`.

### Changed
- **Breaking:** `command_intake::spawn_server` takes a third argument, `SigningContext { server_token, target_agent }`. The intake rejects invalid or expired signatures always, and accepts unsigned commands with a warning while `REQUIRE_SIGNED_COMMANDS` is `false`.
- A `command_id` already seen is never executed again: finished ones return the cached result, in-flight or timed-out ones are rejected (`IntakeError::Duplicate`).
- Updater anti-rollback: never installs a release whose semver is less than or equal to the current version.

### Fixed
- The command timeout restarted on every progress message, and never fired if a handler dropped its progress sender.
