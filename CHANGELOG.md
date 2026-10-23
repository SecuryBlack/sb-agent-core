# Changelog

## [0.4.2] - 2026-10-23

### Added
- The command intake sends a `sig:ok` or `sig:<reason>` progress line before running each order (`missing_signature`, `missing_expiry`, `expired`, `malformed_signature`, `invalid_signature`, `no_keys`). edge-gateway stamps it in `server_commands.stage_times`, so the fleet can be measured before `REQUIRE_SIGNED_COMMANDS` is turned on. It is not a phase: the panel does not show it.
- `command_signing::verification_stage` and `VerifyError::code`, for agents that verify their own orders outside the intake (Nexus).

## [0.4.1] - 2026-10-23

### Fixed
- Old log files were never deleted: rotated files are named `<agent>.log.YYYY-MM-DD`, and the cleanup only looked at the `.log` extension. `logging::init` now keeps the last 7 files with the appender's `max_log_files`, pruning at startup and on every daily rotation.

## [0.4.0] - 2026-10-16

### Added
- `command_signing` (S4): `verify_command`, `enforce`, `canonical_json`, `build_message`, `SIGNING_KEYS` (slot 0: the production key generated on the API server on 2026-10-16; slot 1 free for rotation) and `REQUIRE_SIGNED_COMMANDS = false`.
- `CommandEnvelope.signature` (base64 ed25519) and `CommandEnvelope.expires_at` (Unix secs), both optional; `CommandEnvelope` now derives `Default`.

### Changed
- **Breaking:** `command_intake::spawn_server` takes a third argument, `SigningContext { server_token, target_agent }`. While `REQUIRE_SIGNED_COMMANDS` is `false` the intake accepts any verification failure (unsigned, invalid, expired) with a warning, so a token that does not match the server's shows up in the log instead of breaking the agent; with `true` it rejects them.
- A `command_id` already seen is never executed again: finished ones return the cached result, in-flight or timed-out ones are rejected (`IntakeError::Duplicate`).
- Updater anti-rollback: never installs a release whose semver is less than or equal to the current version.

### Fixed
- The command timeout restarted on every progress message, and never fired if a handler dropped its progress sender.
