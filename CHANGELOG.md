# Changelog

## Unreleased

- Added `AgentSelector` with framework-independent parsing and an optional
  `clap` feature that implements `clap::ValueEnum` without changing host CLI
  shape.
- Added `doctor_many`, `install_many`, `update_many`, and `uninstall_many`.
  Batch mutations validate and preflight every selected runtime before writes,
  preserve executed-command traces, and expose explicit stop-or-continue
  failure policy. Runtime request providers support different native options,
  and unsuccessful batches return an error that retains the complete report.
- Distinguished process spawn failures from supervision failures after spawn,
  so every command that actually starts remains in readiness and mutation
  traces.
- Batch failures distinguish request validation, runtime preflight, and native
  operation stages. New batch result types are non-exhaustive for future
  compatible extension.

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
