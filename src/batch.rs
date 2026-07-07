use crate::{
    AgentPluginError, AgentPluginOperation, AgentRuntime, AgentSelector, CommandRunner,
    DoctorOutcome, DoctorStatus, InstallRequest, NativeRunner, OperationError,
    PluginCommandOutcome, UninstallRequest, UpdateRequest, check_operation_with_runner,
    install_with_runner, uninstall_with_runner, update_with_runner, validate_install_options,
    validate_uninstall_options, validate_update_options,
};
use thiserror::Error;

/// Controls whether mutations continue after one runtime fails.
///
/// Request validation and runtime preflight are always all-or-nothing gates;
/// this policy applies only after mutations begin.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailurePolicy {
    StopOnFailure,
    Continue,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchStatus {
    Succeeded,
    Missing,
    Failed,
    Skipped,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchSkipReason {
    RequestValidationFailed,
    PreflightFailed,
    EarlierOperationFailed,
}

impl BatchSkipReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::RequestValidationFailed => {
                "mutation not attempted because another runtime request is invalid"
            }
            Self::PreflightFailed => "mutation not attempted because another runtime is not ready",
            Self::EarlierOperationFailed => {
                "mutation not attempted after an earlier runtime failed"
            }
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BatchFailure {
    #[error("request validation failed: {0}")]
    Validation(#[source] AgentPluginError),
    #[error("runtime preflight failed: {message}")]
    Preflight { message: String },
    #[error("runtime operation failed: {0}")]
    Operation(#[source] OperationError),
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRuntimeOutcome {
    pub runtime: AgentRuntime,
    pub operation: AgentPluginOperation,
    pub status: BatchStatus,
    /// Every command that actually spawned, in execution order. This includes
    /// preflight probes followed by mutation commands when mutation began.
    pub commands: Vec<String>,
    pub failure: Option<BatchFailure>,
    pub skip_reason: Option<BatchSkipReason>,
}

#[must_use = "batch reports must be inspected or converted into a result"]
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchOperationReport {
    pub operation: AgentPluginOperation,
    pub outcomes: Vec<BatchRuntimeOutcome>,
}

impl BatchOperationReport {
    pub fn is_success(&self) -> bool {
        self.outcomes
            .iter()
            .all(|outcome| outcome.status == BatchStatus::Succeeded)
    }

    pub fn into_result(self) -> BatchResult {
        if self.is_success() {
            Ok(self)
        } else {
            Err(BatchOperationError { report: self })
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("agent plugin batch operation did not succeed")]
pub struct BatchOperationError {
    /// Complete outcomes for every selected runtime, including successes and
    /// skipped runtimes before or after the failure.
    pub report: BatchOperationReport,
}

impl BatchOperationError {
    pub fn report(&self) -> &BatchOperationReport {
        &self.report
    }

    pub fn into_report(self) -> BatchOperationReport {
        self.report
    }
}

/// A batch succeeds only when every selected runtime succeeds. On failure,
/// the error retains the complete report.
pub type BatchResult = Result<BatchOperationReport, BatchOperationError>;

/// Diagnoses every runtime selected by `selector`, preserving runtime order.
pub fn doctor_many(selector: AgentSelector) -> Vec<DoctorOutcome> {
    selector
        .runtimes()
        .iter()
        .copied()
        .map(crate::doctor)
        .collect()
}

/// Installs a plugin into the selected runtimes.
///
/// `request_for` is called exactly once for every selected runtime before
/// validation or native commands begin. This permits runtime-specific Git
/// refs, scopes, marketplace roots, and timeouts without weakening the
/// all-runtime validation gate.
///
/// Every request is validated and every runtime is preflighted before the
/// first native mutation command runs.
pub fn install_many<'a>(
    selector: AgentSelector,
    mut request_for: impl FnMut(AgentRuntime) -> InstallRequest<'a>,
    failure_policy: FailurePolicy,
) -> BatchResult {
    let requests = collect_requests(selector, &mut request_for);
    let mut runner = NativeRunner;
    install_many_with_runner(&requests, failure_policy, &mut runner)
}

fn install_many_with_runner(
    requests: &[(AgentRuntime, InstallRequest<'_>)],
    failure_policy: FailurePolicy,
    runner: &mut impl CommandRunner,
) -> BatchResult {
    run_mutation_batch_with(
        requests,
        AgentPluginOperation::Install,
        failure_policy,
        runner,
        validate_install_options,
        |runtime, operation, request, runner| {
            check_operation_with_runner(runtime, operation, request.command_timeout, runner)
        },
        |runtime, request, runner| install_with_runner(runtime, request.clone(), runner),
    )
    .into_result()
}

/// Updates a plugin in the selected runtimes after an all-runtime preflight.
/// `request_for` is called exactly once per selected runtime before validation.
pub fn update_many<'a>(
    selector: AgentSelector,
    mut request_for: impl FnMut(AgentRuntime) -> UpdateRequest<'a>,
    failure_policy: FailurePolicy,
) -> BatchResult {
    let requests = collect_requests(selector, &mut request_for);
    let mut runner = NativeRunner;
    update_many_with_runner(&requests, failure_policy, &mut runner)
}

fn update_many_with_runner(
    requests: &[(AgentRuntime, UpdateRequest<'_>)],
    failure_policy: FailurePolicy,
    runner: &mut impl CommandRunner,
) -> BatchResult {
    run_mutation_batch_with(
        requests,
        AgentPluginOperation::Update,
        failure_policy,
        runner,
        validate_update_options,
        |runtime, operation, request, runner| {
            check_operation_with_runner(runtime, operation, request.command_timeout, runner)
        },
        |runtime, request, runner| update_with_runner(runtime, *request, runner),
    )
    .into_result()
}

/// Uninstalls a plugin from the selected runtimes after an all-runtime
/// preflight. `request_for` is called exactly once per selected runtime before
/// validation.
pub fn uninstall_many<'a>(
    selector: AgentSelector,
    mut request_for: impl FnMut(AgentRuntime) -> UninstallRequest<'a>,
    failure_policy: FailurePolicy,
) -> BatchResult {
    let requests = collect_requests(selector, &mut request_for);
    let mut runner = NativeRunner;
    uninstall_many_with_runner(&requests, failure_policy, &mut runner)
}

fn uninstall_many_with_runner(
    requests: &[(AgentRuntime, UninstallRequest<'_>)],
    failure_policy: FailurePolicy,
    runner: &mut impl CommandRunner,
) -> BatchResult {
    run_mutation_batch_with(
        requests,
        AgentPluginOperation::Uninstall,
        failure_policy,
        runner,
        validate_uninstall_options,
        |runtime, operation, request, runner| {
            check_operation_with_runner(runtime, operation, request.command_timeout, runner)
        },
        |runtime, request, runner| uninstall_with_runner(runtime, *request, runner),
    )
    .into_result()
}

fn collect_requests<T>(
    selector: AgentSelector,
    mut request_for: impl FnMut(AgentRuntime) -> T,
) -> Vec<(AgentRuntime, T)> {
    selector
        .runtimes()
        .iter()
        .copied()
        .map(|runtime| (runtime, request_for(runtime)))
        .collect()
}

fn run_mutation_batch_with<T, R>(
    requests: &[(AgentRuntime, T)],
    operation: AgentPluginOperation,
    failure_policy: FailurePolicy,
    runner: &mut R,
    mut validate: impl FnMut(AgentRuntime, &T) -> Result<(), AgentPluginError>,
    mut preflight: impl FnMut(AgentRuntime, AgentPluginOperation, &T, &mut R) -> DoctorOutcome,
    mut invoke: impl FnMut(AgentRuntime, &T, &mut R) -> Result<PluginCommandOutcome, OperationError>,
) -> BatchOperationReport {
    let validation_failures = requests
        .iter()
        .map(|(runtime, request)| (*runtime, validate(*runtime, request).err()))
        .collect::<Vec<_>>();
    if validation_failures
        .iter()
        .any(|(_, failure)| failure.is_some())
    {
        let outcomes = validation_failures
            .into_iter()
            .map(|(runtime, failure)| match failure {
                Some(error) => validation_failed_outcome(runtime, operation, error),
                None => skipped_outcome(
                    runtime,
                    operation,
                    Vec::new(),
                    BatchSkipReason::RequestValidationFailed,
                ),
            })
            .collect();
        return BatchOperationReport {
            operation,
            outcomes,
        };
    }

    let probes = requests
        .iter()
        .map(|(runtime, request)| preflight(*runtime, operation, request, runner))
        .collect::<Vec<_>>();
    if probes
        .iter()
        .any(|outcome| outcome.status != DoctorStatus::Ready)
    {
        let outcomes = requests
            .iter()
            .zip(probes)
            .map(|((runtime, _), outcome)| match outcome.status {
                DoctorStatus::Ready => skipped_outcome(
                    *runtime,
                    operation,
                    outcome.commands,
                    BatchSkipReason::PreflightFailed,
                ),
                DoctorStatus::Missing | DoctorStatus::Failed => {
                    let status = match outcome.status {
                        DoctorStatus::Missing => BatchStatus::Missing,
                        DoctorStatus::Failed => BatchStatus::Failed,
                        DoctorStatus::Ready => BatchStatus::Skipped,
                    };
                    BatchRuntimeOutcome {
                        runtime: *runtime,
                        operation,
                        status,
                        commands: outcome.commands,
                        failure: Some(BatchFailure::Preflight {
                            message: outcome
                                .message
                                .unwrap_or_else(|| "runtime is not ready".to_owned()),
                        }),
                        skip_reason: None,
                    }
                }
            })
            .collect();
        return BatchOperationReport {
            operation,
            outcomes,
        };
    }

    let mut outcomes = Vec::with_capacity(requests.len());
    let mut stopped = false;
    for ((runtime, request), probe) in requests.iter().zip(probes) {
        let runtime = *runtime;
        if stopped {
            outcomes.push(skipped_outcome(
                runtime,
                operation,
                probe.commands,
                BatchSkipReason::EarlierOperationFailed,
            ));
            continue;
        }

        match invoke(runtime, request, runner) {
            Ok(outcome) => {
                let mut commands = probe.commands;
                commands.extend(outcome.commands);
                outcomes.push(BatchRuntimeOutcome {
                    runtime,
                    operation,
                    status: BatchStatus::Succeeded,
                    commands,
                    failure: None,
                    skip_reason: None,
                });
            }
            Err(failure) => {
                let mut commands = probe.commands;
                commands.extend(failure.completed.iter().cloned());
                if let AgentPluginError::CliFailed { command, .. } = &failure.error {
                    commands.push(command.clone());
                }
                outcomes.push(failed_outcome(runtime, operation, commands, failure));
                stopped = failure_policy == FailurePolicy::StopOnFailure;
            }
        }
    }

    BatchOperationReport {
        operation,
        outcomes,
    }
}

fn validation_failed_outcome(
    runtime: AgentRuntime,
    operation: AgentPluginOperation,
    failure: AgentPluginError,
) -> BatchRuntimeOutcome {
    BatchRuntimeOutcome {
        runtime,
        operation,
        status: BatchStatus::Failed,
        commands: Vec::new(),
        failure: Some(BatchFailure::Validation(failure)),
        skip_reason: None,
    }
}

fn failed_outcome(
    runtime: AgentRuntime,
    operation: AgentPluginOperation,
    commands: Vec<String>,
    failure: OperationError,
) -> BatchRuntimeOutcome {
    let status = if matches!(failure.error, AgentPluginError::CliMissing { .. }) {
        BatchStatus::Missing
    } else {
        BatchStatus::Failed
    };
    BatchRuntimeOutcome {
        runtime,
        operation,
        status,
        commands,
        failure: Some(BatchFailure::Operation(failure)),
        skip_reason: None,
    }
}

fn skipped_outcome(
    runtime: AgentRuntime,
    operation: AgentPluginOperation,
    commands: Vec<String>,
    reason: BatchSkipReason,
) -> BatchRuntimeOutcome {
    BatchRuntimeOutcome {
        runtime,
        operation,
        status: BatchStatus::Skipped,
        commands,
        failure: None,
        skip_reason: Some(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandRunError, MarketplaceSource, PluginRef, ProcessOutput};
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::time::Duration;

    #[test]
    fn install_wiring_preflights_all_runtimes_before_runtime_specific_mutations()
    -> Result<(), BatchOperationError> {
        let codex = InstallRequest::new(
            MarketplaceSource::new("owner/repo").with_git_ref("main"),
            plugin(),
        );
        let claude = InstallRequest::new(MarketplaceSource::new("owner/repo"), plugin())
            .with_marketplace_scope("user")
            .with_plugin_scope("user");
        let requests = [(AgentRuntime::Codex, codex), (AgentRuntime::Claude, claude)];
        let mut runner = FakeBatchRunner::successes(8);

        let report =
            install_many_with_runner(&requests, FailurePolicy::StopOnFailure, &mut runner)?;

        assert_eq!(
            runner.calls,
            vec![
                call("codex", ["plugin", "marketplace", "add", "--help"]),
                call("codex", ["plugin", "add", "--help"]),
                call("claude", ["plugin", "marketplace", "add", "--help"],),
                call("claude", ["plugin", "install", "--help"]),
                call(
                    "codex",
                    [
                        "plugin",
                        "marketplace",
                        "add",
                        "--ref",
                        "main",
                        "owner/repo",
                    ],
                ),
                call("codex", ["plugin", "add", "plugin@marketplace"]),
                call(
                    "claude",
                    [
                        "plugin",
                        "marketplace",
                        "add",
                        "--scope",
                        "user",
                        "owner/repo",
                    ],
                ),
                call(
                    "claude",
                    ["plugin", "install", "--scope", "user", "plugin@marketplace",],
                ),
            ]
        );
        assert!(report.is_success());
        Ok(())
    }

    #[test]
    fn update_and_uninstall_wiring_use_operation_specific_commands()
    -> Result<(), BatchOperationError> {
        let update_requests = [(
            AgentRuntime::Claude,
            UpdateRequest::new(plugin()).with_marketplace_name("marketplace"),
        )];
        let mut update_runner = FakeBatchRunner::successes(4);

        let update_report = update_many_with_runner(
            &update_requests,
            FailurePolicy::StopOnFailure,
            &mut update_runner,
        )?;
        assert!(update_report.is_success());

        assert_eq!(
            update_runner.calls,
            vec![
                call("claude", ["plugin", "marketplace", "update", "--help"],),
                call("claude", ["plugin", "update", "--help"]),
                call("claude", ["plugin", "marketplace", "update", "marketplace"],),
                call("claude", ["plugin", "update", "plugin"]),
            ]
        );

        let uninstall_requests = [(AgentRuntime::Codex, UninstallRequest::new(plugin()))];
        let mut uninstall_runner = FakeBatchRunner::successes(2);

        let uninstall_report = uninstall_many_with_runner(
            &uninstall_requests,
            FailurePolicy::StopOnFailure,
            &mut uninstall_runner,
        )?;
        assert!(uninstall_report.is_success());

        assert_eq!(
            uninstall_runner.calls,
            vec![
                call("codex", ["plugin", "remove", "--help"]),
                call("codex", ["plugin", "remove", "plugin@marketplace"]),
            ]
        );
        Ok(())
    }

    #[test]
    fn supervision_failure_is_an_error_with_complete_batch_trace() -> std::io::Result<()> {
        let requests = [(AgentRuntime::Codex, UninstallRequest::new(plugin()))];
        let mut runner = FakeBatchRunner::from_outputs([
            Ok(success_output()),
            Err(CommandRunError::Supervision(std::io::Error::other(
                "wait failed",
            ))),
        ]);

        let result =
            uninstall_many_with_runner(&requests, FailurePolicy::StopOnFailure, &mut runner);
        let error = match result {
            Ok(_) => return Err(std::io::Error::other("supervision failure returned Ok")),
            Err(error) => error,
        };

        assert_eq!(
            error.report().outcomes.first().map(|row| row.status),
            Some(BatchStatus::Failed)
        );
        assert_eq!(
            error
                .report()
                .outcomes
                .first()
                .map(|row| row.commands.as_slice()),
            Some(
                [
                    "codex plugin remove --help".to_owned(),
                    "codex plugin remove plugin@marketplace".to_owned(),
                ]
                .as_slice()
            )
        );
        assert!(matches!(
            error
                .report()
                .outcomes
                .first()
                .and_then(|row| row.failure.as_ref()),
            Some(BatchFailure::Operation(OperationError {
                error: AgentPluginError::CliFailed { status: None, .. },
                ..
            }))
        ));
        Ok(())
    }

    #[test]
    fn public_install_batch_validates_all_requests_before_native_probes() -> std::io::Result<()> {
        let request = InstallRequest::new(
            MarketplaceSource::new("owner/repo"),
            PluginRef {
                selector: "plugin@marketplace",
                name: "plugin",
            },
        )
        .with_plugin_scope("user");
        let provider_calls = Cell::new(0);

        let result = install_many(
            AgentSelector::All,
            |_| {
                provider_calls.set(provider_calls.get() + 1);
                request.clone()
            },
            FailurePolicy::StopOnFailure,
        );
        let error = match result {
            Ok(_) => return Err(std::io::Error::other("invalid Codex request returned Ok")),
            Err(error) => error,
        };
        let report = error.report();

        assert_eq!(provider_calls.get(), 2);
        assert_eq!(
            statuses(report),
            [BatchStatus::Failed, BatchStatus::Skipped]
        );
        assert!(matches!(
            report.outcomes.first().and_then(|row| row.failure.as_ref()),
            Some(BatchFailure::Validation(
                AgentPluginError::UnsupportedOption {
                    runtime: "codex",
                    option: "plugin_scope",
                    ..
                }
            ))
        ));
        Ok(())
    }

    #[test]
    fn request_validation_blocks_all_preflight_and_mutation() {
        let preflights = Cell::new(0);
        let mutations = Cell::new(0);

        let mut runner = ();
        let report = run_mutation_batch_with(
            &test_requests(),
            AgentPluginOperation::Install,
            FailurePolicy::Continue,
            &mut runner,
            |runtime, _| {
                if runtime == AgentRuntime::Claude {
                    Err(AgentPluginError::UnsupportedOption {
                        runtime: runtime.id(),
                        option: "test",
                        reason: "unsupported in test",
                    })
                } else {
                    Ok(())
                }
            },
            |runtime, _, _, _| {
                preflights.set(preflights.get() + 1);
                ready(runtime)
            },
            |runtime, _, _| {
                mutations.set(mutations.get() + 1);
                Ok(success(runtime, "mutation"))
            },
        );

        assert_eq!(preflights.get(), 0);
        assert_eq!(mutations.get(), 0);
        assert_eq!(
            statuses(&report),
            [BatchStatus::Skipped, BatchStatus::Failed]
        );
        assert_eq!(
            report.outcomes.first().and_then(|row| row.skip_reason),
            Some(BatchSkipReason::RequestValidationFailed)
        );
    }

    #[test]
    fn failed_preflight_blocks_every_mutation_and_keeps_probe_traces() {
        let mutations = Cell::new(0);
        let mut runner = ();
        let report = run_mutation_batch_with(
            &test_requests(),
            AgentPluginOperation::Update,
            FailurePolicy::Continue,
            &mut runner,
            |_, _| Ok(()),
            |runtime, _, _, _| {
                if runtime == AgentRuntime::Claude {
                    DoctorOutcome {
                        runtime,
                        status: DoctorStatus::Missing,
                        commands: vec!["claude probe".to_owned()],
                        message: Some("missing".to_owned()),
                    }
                } else {
                    ready(runtime)
                }
            },
            |runtime, _, _| {
                mutations.set(mutations.get() + 1);
                Ok(success(runtime, "mutation"))
            },
        );

        assert_eq!(mutations.get(), 0);
        assert_eq!(
            statuses(&report),
            [BatchStatus::Skipped, BatchStatus::Missing]
        );
        assert_eq!(
            report.outcomes.first().map(|row| row.commands.as_slice()),
            Some(["codex probe".to_owned()].as_slice())
        );
    }

    #[test]
    fn stop_policy_skips_later_runtime_after_mutation_failure() {
        let mutations = Cell::new(0);
        let mut runner = ();
        let report = run_mutation_batch_with(
            &test_requests(),
            AgentPluginOperation::Update,
            FailurePolicy::StopOnFailure,
            &mut runner,
            |_, _| Ok(()),
            |runtime, _, _, _| ready(runtime),
            |runtime, _, _| {
                mutations.set(mutations.get() + 1);
                Err(executed_failure(runtime))
            },
        );

        assert_eq!(mutations.get(), 1);
        assert_eq!(
            statuses(&report),
            [BatchStatus::Failed, BatchStatus::Skipped]
        );
        assert_eq!(
            report.outcomes.first().map(|row| row.commands.as_slice()),
            Some(
                ["codex probe", "codex completed", "codex failed"]
                    .map(str::to_owned)
                    .as_slice()
            )
        );
        assert_eq!(
            report.outcomes.get(1).and_then(|row| row.skip_reason),
            Some(BatchSkipReason::EarlierOperationFailed)
        );
    }

    #[test]
    fn continue_policy_attempts_every_runtime() {
        let mutations = Cell::new(0);
        let mut runner = ();
        let report = run_mutation_batch_with(
            &test_requests(),
            AgentPluginOperation::Uninstall,
            FailurePolicy::Continue,
            &mut runner,
            |_, _| Ok(()),
            |runtime, _, _, _| ready(runtime),
            |runtime, _, _| {
                mutations.set(mutations.get() + 1);
                if runtime == AgentRuntime::Codex {
                    Err(executed_failure(runtime))
                } else {
                    Ok(success(runtime, "claude mutation"))
                }
            },
        );

        assert_eq!(mutations.get(), 2);
        assert_eq!(
            statuses(&report),
            [BatchStatus::Failed, BatchStatus::Succeeded]
        );
        assert!(!report.is_success());
    }

    #[test]
    fn successful_batch_combines_probe_and_mutation_traces() {
        let mut runner = ();
        let report = run_mutation_batch_with(
            &[(AgentRuntime::Codex, ())],
            AgentPluginOperation::Install,
            FailurePolicy::StopOnFailure,
            &mut runner,
            |_, _| Ok(()),
            |runtime, _, _, _| ready(runtime),
            |runtime, _, _| Ok(success(runtime, "codex mutation")),
        );

        assert!(report.is_success());
        assert_eq!(
            report.outcomes.first().map(|row| row.commands.as_slice()),
            Some(
                ["codex probe", "codex mutation"]
                    .map(str::to_owned)
                    .as_slice()
            )
        );
    }

    fn ready(runtime: AgentRuntime) -> DoctorOutcome {
        DoctorOutcome {
            runtime,
            status: DoctorStatus::Ready,
            commands: vec![format!("{} probe", runtime.id())],
            message: None,
        }
    }

    fn test_requests() -> [(AgentRuntime, ()); 2] {
        [(AgentRuntime::Codex, ()), (AgentRuntime::Claude, ())]
    }

    fn plugin() -> PluginRef<'static> {
        PluginRef {
            selector: "plugin@marketplace",
            name: "plugin",
        }
    }

    fn call<const N: usize>(program: &str, args: [&str; N]) -> (String, Vec<String>) {
        (
            program.to_owned(),
            args.into_iter().map(str::to_owned).collect(),
        )
    }

    struct FakeBatchRunner {
        calls: Vec<(String, Vec<String>)>,
        outputs: VecDeque<Result<ProcessOutput, CommandRunError>>,
    }

    impl FakeBatchRunner {
        fn successes(count: usize) -> Self {
            Self {
                calls: Vec::new(),
                outputs: (0..count).map(|_| Ok(success_output())).collect(),
            }
        }

        fn from_outputs<const N: usize>(
            outputs: [Result<ProcessOutput, CommandRunError>; N],
        ) -> Self {
            Self {
                calls: Vec::new(),
                outputs: VecDeque::from(outputs),
            }
        }
    }

    impl CommandRunner for FakeBatchRunner {
        fn run(
            &mut self,
            program: &str,
            args: &[OsString],
            _command_timeout: Duration,
        ) -> Result<ProcessOutput, CommandRunError> {
            self.calls.push((
                program.to_owned(),
                args.iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            ));
            self.outputs.pop_front().unwrap_or_else(|| {
                Err(CommandRunError::Supervision(std::io::Error::other(
                    "missing fake output",
                )))
            })
        }
    }

    fn success_output() -> ProcessOutput {
        ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn success(runtime: AgentRuntime, command: &str) -> PluginCommandOutcome {
        PluginCommandOutcome {
            runtime,
            commands: vec![command.to_owned()],
        }
    }

    fn executed_failure(runtime: AgentRuntime) -> OperationError {
        OperationError {
            completed: vec![format!("{} completed", runtime.id())],
            error: AgentPluginError::CliFailed {
                runtime: runtime.id(),
                phase: "test",
                command: format!("{} failed", runtime.id()),
                status: Some(1),
                stderr: "failure".to_owned(),
            },
        }
    }

    fn statuses(report: &BatchOperationReport) -> Vec<BatchStatus> {
        report.outcomes.iter().map(|row| row.status).collect()
    }
}
