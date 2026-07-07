# Changelog

## 0.6.0 - 2026-07-07

- Added framework-independent `AgentSelector` parsing and an optional `clap`
  feature that implements `clap::ValueEnum`. Default builds remain free of a
  CLI-framework dependency, and host applications retain control of their CLI
  shape.
- Added `doctor_many`, `install_many`, `update_many`, and `uninstall_many`.
  Batch mutations validate and preflight every selected runtime before writes,
  then apply an explicit stop-or-continue policy once mutations begin. Runtime
  request providers support per-runtime Git refs, scopes, roots, and timeouts.
- Added complete batch reports for successful, failed, missing, and skipped
  runtimes. Unsuccessful batches retain the full report and distinguish request
  validation, runtime preflight, and native operation failures.
- Fixed command evidence for post-spawn supervision failures. Commands that
  started now remain in readiness and mutation traces, while commands that
  never spawned remain excluded.
- Marked the new batch result and failure types as non-exhaustive so they can be
  extended compatibly in future releases.

## 0.5.0 - 2026-07-05

- Operations return `OperationError { completed, error }`: the
  completed-command prefix travels uniformly on the operation error
  whatever stopped it, so a spawn failure after a successful mutation no
  longer erases the mutation from the evidence. `CliFailed` drops its
  `completed` field.

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
