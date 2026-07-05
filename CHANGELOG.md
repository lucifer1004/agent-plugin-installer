# Changelog

## 0.4.0 - 2026-07-05

- A command enters a trace only after its process actually spawned:
  readiness probes that never started no longer appear in
  `DoctorOutcome::commands`, and a spawn failure is the new
  `CliSpawnFailed` variant instead of a `CliFailed` whose command never
  ran.

## 0.3.0 - 2026-07-05

- `CliFailed` carries `completed`: the rendered commands that succeeded
  earlier in the same operation, so a mid-operation failure reports the
  partially applied state instead of hiding successful mutations.

## 0.2.0 - 2026-06-14

- Added Codex and Claude plugin uninstall helpers.
- Added operation-specific readiness checks for install, update, and uninstall.

## 0.1.0 - 2026-06-14

- Initial release with Codex and Claude plugin doctor/install/update helpers.
