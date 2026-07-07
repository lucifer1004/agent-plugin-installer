//! Native CLI orchestration helpers for installing agent plugins.
//!
//! This crate intentionally does not know about any parent application's
//! JSON envelope, release process, or package validation rules. Callers
//! provide marketplace/plugin metadata; the crate invokes the selected
//! agent runtime's public CLI and returns command summaries or structured
//! errors. Batch helpers add all-runtime validation and preflight gates while
//! leaving output policy with the caller.

mod batch;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant};
use thiserror::Error;

pub use batch::{
    BatchFailure, BatchOperationError, BatchOperationReport, BatchResult, BatchRuntimeOutcome,
    BatchSkipReason, BatchStatus, FailurePolicy, doctor_many, install_many, uninstall_many,
    update_many,
};

pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentRuntime {
    Codex,
    Claude,
}

impl AgentRuntime {
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn cli(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn supported() -> &'static [Self] {
        &[Self::Codex, Self::Claude]
    }

    fn readiness_commands(
        self,
        operation: AgentPluginOperation,
    ) -> &'static [&'static [&'static str]] {
        match operation {
            AgentPluginOperation::Doctor => self.doctor_commands(),
            AgentPluginOperation::Install => self.install_commands(),
            AgentPluginOperation::Update => self.update_commands(),
            AgentPluginOperation::Uninstall => self.uninstall_commands(),
        }
    }

    fn doctor_commands(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::Codex => &[
                &["plugin", "--help"],
                &["plugin", "marketplace", "add", "--help"],
                &["plugin", "marketplace", "upgrade", "--help"],
                &["plugin", "add", "--help"],
                &["plugin", "remove", "--help"],
            ],
            Self::Claude => &[
                &["plugin", "--help"],
                &["plugin", "marketplace", "add", "--help"],
                &["plugin", "marketplace", "update", "--help"],
                &["plugin", "install", "--help"],
                &["plugin", "update", "--help"],
                &["plugin", "uninstall", "--help"],
            ],
        }
    }

    fn install_commands(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::Codex => &[
                &["plugin", "marketplace", "add", "--help"],
                &["plugin", "add", "--help"],
            ],
            Self::Claude => &[
                &["plugin", "marketplace", "add", "--help"],
                &["plugin", "install", "--help"],
            ],
        }
    }

    fn update_commands(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::Codex => &[
                &["plugin", "marketplace", "upgrade", "--help"],
                &["plugin", "add", "--help"],
            ],
            Self::Claude => &[
                &["plugin", "marketplace", "update", "--help"],
                &["plugin", "update", "--help"],
            ],
        }
    }

    fn uninstall_commands(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::Codex => &[&["plugin", "remove", "--help"]],
            Self::Claude => &[&["plugin", "uninstall", "--help"]],
        }
    }

    fn supports_marketplace_git_ref(self) -> bool {
        matches!(self, Self::Codex)
    }

    fn supports_scopes(self) -> bool {
        matches!(self, Self::Claude)
    }
}

/// Selects one supported agent runtime or every supported runtime.
///
/// Host applications own whether this is a positional argument, an option,
/// or a configuration value. Enable the `clap` feature to use this type
/// directly as a `clap::ValueEnum`.
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSelector {
    Codex,
    Claude,
    All,
}

impl AgentSelector {
    pub fn runtimes(self) -> &'static [AgentRuntime] {
        match self {
            Self::Codex => &[AgentRuntime::Codex],
            Self::Claude => &[AgentRuntime::Claude],
            Self::All => AgentRuntime::supported(),
        }
    }
}

impl From<AgentRuntime> for AgentSelector {
    fn from(runtime: AgentRuntime) -> Self {
        match runtime {
            AgentRuntime::Codex => Self::Codex,
            AgentRuntime::Claude => Self::Claude,
        }
    }
}

impl fmt::Display for AgentSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::All => "all",
        })
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
#[error("unsupported agent selector `{value}`; expected codex, claude, or all")]
pub struct ParseAgentSelectorError {
    value: String,
}

impl FromStr for AgentSelector {
    type Err = ParseAgentSelectorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "all" => Ok(Self::All),
            _ => Err(ParseAgentSelectorError {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPluginOperation {
    Doctor,
    Install,
    Update,
    Uninstall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceSource {
    pub source: OsString,
    pub git_ref: Option<OsString>,
    pub sparse_paths: Vec<OsString>,
}

impl MarketplaceSource {
    pub fn local(path: &Path) -> Self {
        Self::new(path.as_os_str())
    }

    pub fn new(source: impl AsRef<OsStr>) -> Self {
        Self {
            source: source.as_ref().to_os_string(),
            git_ref: None,
            sparse_paths: Vec::new(),
        }
    }

    pub fn with_git_ref(mut self, git_ref: impl AsRef<OsStr>) -> Self {
        self.git_ref = Some(git_ref.as_ref().to_os_string());
        self
    }

    pub fn with_sparse_path(mut self, sparse_path: impl AsRef<OsStr>) -> Self {
        self.sparse_paths.push(sparse_path.as_ref().to_os_string());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginRef<'a> {
    pub selector: &'a str,
    pub name: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest<'a> {
    pub marketplace: MarketplaceSource,
    pub plugin: PluginRef<'a>,
    pub marketplace_scope: Option<&'a str>,
    pub plugin_scope: Option<&'a str>,
    pub command_timeout: Duration,
}

impl<'a> InstallRequest<'a> {
    pub fn new(marketplace: MarketplaceSource, plugin: PluginRef<'a>) -> Self {
        Self {
            marketplace,
            plugin,
            marketplace_scope: None,
            plugin_scope: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn local(marketplace_root: &Path, plugin: PluginRef<'a>) -> Self {
        Self::new(MarketplaceSource::local(marketplace_root), plugin)
    }

    pub fn with_marketplace_scope(mut self, scope: &'a str) -> Self {
        self.marketplace_scope = Some(scope);
        self
    }

    pub fn with_plugin_scope(mut self, scope: &'a str) -> Self {
        self.plugin_scope = Some(scope);
        self
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateRequest<'a> {
    pub plugin: PluginRef<'a>,
    pub marketplace_name: Option<&'a str>,
    pub plugin_scope: Option<&'a str>,
    pub command_timeout: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UninstallRequest<'a> {
    pub plugin: PluginRef<'a>,
    pub plugin_scope: Option<&'a str>,
    pub command_timeout: Duration,
}

impl<'a> UninstallRequest<'a> {
    pub fn new(plugin: PluginRef<'a>) -> Self {
        Self {
            plugin,
            plugin_scope: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn with_plugin_scope(mut self, scope: &'a str) -> Self {
        self.plugin_scope = Some(scope);
        self
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }
}

impl<'a> UpdateRequest<'a> {
    pub fn new(plugin: PluginRef<'a>) -> Self {
        Self {
            plugin,
            marketplace_name: None,
            plugin_scope: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn with_marketplace_name(mut self, marketplace_name: &'a str) -> Self {
        self.marketplace_name = Some(marketplace_name);
        self
    }

    pub fn with_plugin_scope(mut self, scope: &'a str) -> Self {
        self.plugin_scope = Some(scope);
        self
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DoctorStatus {
    Ready,
    Missing,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorOutcome {
    pub runtime: AgentRuntime,
    pub status: DoctorStatus,
    pub commands: Vec<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginCommandOutcome {
    pub runtime: AgentRuntime,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AgentPluginError {
    #[error("{runtime} CLI `{cli}` is not available on PATH")]
    CliMissing {
        runtime: &'static str,
        cli: &'static str,
    },

    #[error("{runtime} {phase} could not start `{command}`: {reason}")]
    CliSpawnFailed {
        runtime: &'static str,
        phase: &'static str,
        /// The command that was attempted. It never spawned, so it belongs
        /// in no executed-command trace.
        command: String,
        reason: String,
    },

    #[error("{runtime} {phase} failed while running `{command}`")]
    CliFailed {
        runtime: &'static str,
        phase: &'static str,
        command: String,
        status: Option<i32>,
        stderr: String,
    },

    #[error("{runtime} does not support option `{option}`: {reason}")]
    UnsupportedOption {
        runtime: &'static str,
        option: &'static str,
        reason: &'static str,
    },
}

/// An operation-level failure: the error that stopped the operation plus
/// every command that completed before it. Completed commands are evidence
/// of a partially applied operation whatever the error kind — an executed
/// failure, a spawn failure, or a vanished CLI — so they travel uniformly
/// on the operation error instead of on individual variants.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{error}")]
pub struct OperationError {
    /// Rendered commands that completed successfully before the failure,
    /// in execution order.
    pub completed: Vec<String>,
    #[source]
    pub error: AgentPluginError,
}

pub fn doctor(runtime: AgentRuntime) -> DoctorOutcome {
    doctor_with_timeout(runtime, DEFAULT_COMMAND_TIMEOUT)
}

pub fn doctor_with_timeout(runtime: AgentRuntime, command_timeout: Duration) -> DoctorOutcome {
    check_operation_with_timeout(runtime, AgentPluginOperation::Doctor, command_timeout)
}

pub fn check_operation(runtime: AgentRuntime, operation: AgentPluginOperation) -> DoctorOutcome {
    check_operation_with_timeout(runtime, operation, DEFAULT_COMMAND_TIMEOUT)
}

pub fn check_operation_with_timeout(
    runtime: AgentRuntime,
    operation: AgentPluginOperation,
    command_timeout: Duration,
) -> DoctorOutcome {
    let mut runner = NativeRunner;
    check_operation_with_runner(runtime, operation, command_timeout, &mut runner)
}

pub fn install(
    runtime: AgentRuntime,
    request: InstallRequest<'_>,
) -> Result<PluginCommandOutcome, OperationError> {
    let mut runner = NativeRunner;
    install_with_runner(runtime, request, &mut runner)
}

pub fn update(
    runtime: AgentRuntime,
    request: UpdateRequest<'_>,
) -> Result<PluginCommandOutcome, OperationError> {
    let mut runner = NativeRunner;
    update_with_runner(runtime, request, &mut runner)
}

pub fn uninstall(
    runtime: AgentRuntime,
    request: UninstallRequest<'_>,
) -> Result<PluginCommandOutcome, OperationError> {
    let mut runner = NativeRunner;
    uninstall_with_runner(runtime, request, &mut runner)
}

fn check_operation_with_runner(
    runtime: AgentRuntime,
    operation: AgentPluginOperation,
    command_timeout: Duration,
    runner: &mut impl CommandRunner,
) -> DoctorOutcome {
    let mut commands = Vec::new();
    for command_args in runtime.readiness_commands(operation) {
        let args: Vec<OsString> = command_args
            .iter()
            .map(|arg| OsString::from(*arg))
            .collect();
        let command = render_command(runtime.cli(), &args);
        // A command enters the trace only once its process actually ran:
        // a probe that spawned and failed is evidence, a probe that never
        // spawned is not.
        match runner.run(runtime.cli(), &args, command_timeout) {
            Ok(output) if output.success => commands.push(command),
            Ok(output) => {
                commands.push(command.clone());
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Failed,
                    commands,
                    message: Some(format!(
                        "required command failed: {command}: {}",
                        summarize_child_output(&output)
                    )),
                };
            }
            Err(CommandRunError::Spawn(source))
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Missing,
                    commands,
                    message: Some(format!("`{}` is not available on PATH", runtime.cli())),
                };
            }
            Err(CommandRunError::Spawn(source)) => {
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Failed,
                    commands,
                    message: Some(format!("could not start {command}: {source}")),
                };
            }
            Err(CommandRunError::Supervision(source)) => {
                commands.push(command.clone());
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Failed,
                    commands,
                    message: Some(format!(
                        "could not monitor spawned command {command}: {source}"
                    )),
                };
            }
        }
    }
    DoctorOutcome {
        runtime,
        status: DoctorStatus::Ready,
        commands,
        message: None,
    }
}

fn install_with_runner(
    runtime: AgentRuntime,
    request: InstallRequest<'_>,
    runner: &mut impl CommandRunner,
) -> Result<PluginCommandOutcome, OperationError> {
    validate_install_options(runtime, &request).map_err(|error| OperationError {
        completed: Vec::new(),
        error,
    })?;
    let steps = match runtime {
        AgentRuntime::Codex => vec![
            (
                "marketplace-add",
                codex_marketplace_add_args(&request.marketplace),
            ),
            (
                "plugin-add",
                vec![
                    OsString::from("plugin"),
                    OsString::from("add"),
                    OsString::from(request.plugin.selector),
                ],
            ),
        ],
        AgentRuntime::Claude => vec![
            ("marketplace-add", claude_marketplace_add_args(&request)),
            (
                "plugin-install",
                claude_scoped_plugin_args("install", request.plugin.selector, request.plugin_scope),
            ),
        ],
    };
    let commands = run_operation_commands(runtime, steps, request.command_timeout, runner)?;
    Ok(PluginCommandOutcome { runtime, commands })
}

/// Run an operation's native commands in order, accumulating each rendered
/// command. A mid-operation failure carries the commands that already
/// completed, so the partially applied state stays auditable.
fn run_operation_commands(
    runtime: AgentRuntime,
    steps: Vec<(&'static str, Vec<OsString>)>,
    command_timeout: Duration,
    runner: &mut impl CommandRunner,
) -> Result<Vec<String>, OperationError> {
    let mut commands = Vec::with_capacity(steps.len());
    for (phase, args) in steps {
        match run_agent_command(runtime, phase, runtime.cli(), args, command_timeout, runner) {
            Ok(rendered) => commands.push(rendered),
            Err(error) => {
                return Err(OperationError {
                    completed: commands,
                    error,
                });
            }
        }
    }
    Ok(commands)
}

fn update_with_runner(
    runtime: AgentRuntime,
    request: UpdateRequest<'_>,
    runner: &mut impl CommandRunner,
) -> Result<PluginCommandOutcome, OperationError> {
    validate_update_options(runtime, &request).map_err(|error| OperationError {
        completed: Vec::new(),
        error,
    })?;
    let steps = match runtime {
        AgentRuntime::Codex => vec![
            (
                "marketplace-upgrade",
                codex_marketplace_upgrade_args(request.marketplace_name),
            ),
            (
                "plugin-add",
                vec![
                    OsString::from("plugin"),
                    OsString::from("add"),
                    OsString::from(request.plugin.selector),
                ],
            ),
        ],
        AgentRuntime::Claude => vec![
            (
                "marketplace-update",
                claude_marketplace_update_args(request.marketplace_name),
            ),
            (
                "plugin-update",
                claude_scoped_plugin_args("update", request.plugin.name, request.plugin_scope),
            ),
        ],
    };
    let commands = run_operation_commands(runtime, steps, request.command_timeout, runner)?;
    Ok(PluginCommandOutcome { runtime, commands })
}

fn uninstall_with_runner(
    runtime: AgentRuntime,
    request: UninstallRequest<'_>,
    runner: &mut impl CommandRunner,
) -> Result<PluginCommandOutcome, OperationError> {
    validate_uninstall_options(runtime, &request).map_err(|error| OperationError {
        completed: Vec::new(),
        error,
    })?;
    let steps = match runtime {
        AgentRuntime::Codex => vec![(
            "plugin-remove",
            vec![
                OsString::from("plugin"),
                OsString::from("remove"),
                OsString::from(request.plugin.selector),
            ],
        )],
        AgentRuntime::Claude => vec![(
            "plugin-uninstall",
            claude_scoped_plugin_args("uninstall", request.plugin.name, request.plugin_scope),
        )],
    };
    let commands = run_operation_commands(runtime, steps, request.command_timeout, runner)?;
    Ok(PluginCommandOutcome { runtime, commands })
}

fn validate_install_options(
    runtime: AgentRuntime,
    request: &InstallRequest<'_>,
) -> Result<(), AgentPluginError> {
    if request.marketplace.git_ref.is_some() && !runtime.supports_marketplace_git_ref() {
        return Err(AgentPluginError::UnsupportedOption {
            runtime: runtime.id(),
            option: "marketplace.git_ref",
            reason: "the native runtime CLI does not expose a git ref option for marketplace add",
        });
    }
    if request.marketplace_scope.is_some() && !runtime.supports_scopes() {
        return Err(AgentPluginError::UnsupportedOption {
            runtime: runtime.id(),
            option: "marketplace_scope",
            reason: "the native runtime CLI does not expose marketplace scopes",
        });
    }
    if request.plugin_scope.is_some() && !runtime.supports_scopes() {
        return Err(AgentPluginError::UnsupportedOption {
            runtime: runtime.id(),
            option: "plugin_scope",
            reason: "the native runtime CLI does not expose plugin scopes",
        });
    }
    Ok(())
}

fn validate_update_options(
    runtime: AgentRuntime,
    request: &UpdateRequest<'_>,
) -> Result<(), AgentPluginError> {
    if request.plugin_scope.is_some() && !runtime.supports_scopes() {
        return Err(AgentPluginError::UnsupportedOption {
            runtime: runtime.id(),
            option: "plugin_scope",
            reason: "the native runtime CLI does not expose plugin scopes",
        });
    }
    Ok(())
}

fn validate_uninstall_options(
    runtime: AgentRuntime,
    request: &UninstallRequest<'_>,
) -> Result<(), AgentPluginError> {
    if request.plugin_scope.is_some() && !runtime.supports_scopes() {
        return Err(AgentPluginError::UnsupportedOption {
            runtime: runtime.id(),
            option: "plugin_scope",
            reason: "the native runtime CLI does not expose plugin scopes",
        });
    }
    Ok(())
}

fn codex_marketplace_add_args(marketplace: &MarketplaceSource) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("plugin"),
        OsString::from("marketplace"),
        OsString::from("add"),
    ];
    if let Some(git_ref) = &marketplace.git_ref {
        args.push(OsString::from("--ref"));
        args.push(git_ref.clone());
    }
    for sparse_path in &marketplace.sparse_paths {
        args.push(OsString::from("--sparse"));
        args.push(sparse_path.clone());
    }
    args.push(marketplace.source.clone());
    args
}

fn codex_marketplace_upgrade_args(marketplace_name: Option<&str>) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("plugin"),
        OsString::from("marketplace"),
        OsString::from("upgrade"),
    ];
    if let Some(marketplace_name) = marketplace_name {
        args.push(OsString::from(marketplace_name));
    }
    args
}

fn claude_marketplace_add_args(request: &InstallRequest<'_>) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("plugin"),
        OsString::from("marketplace"),
        OsString::from("add"),
    ];
    if let Some(scope) = request.marketplace_scope {
        args.push(OsString::from("--scope"));
        args.push(OsString::from(scope));
    }
    if !request.marketplace.sparse_paths.is_empty() {
        args.push(OsString::from("--sparse"));
        args.extend(request.marketplace.sparse_paths.iter().cloned());
    }
    args.push(request.marketplace.source.clone());
    args
}

fn claude_marketplace_update_args(marketplace_name: Option<&str>) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("plugin"),
        OsString::from("marketplace"),
        OsString::from("update"),
    ];
    if let Some(marketplace_name) = marketplace_name {
        args.push(OsString::from(marketplace_name));
    }
    args
}

fn claude_scoped_plugin_args(
    subcommand: &'static str,
    plugin: &str,
    scope: Option<&str>,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("plugin"), OsString::from(subcommand)];
    if let Some(scope) = scope {
        args.push(OsString::from("--scope"));
        args.push(OsString::from(scope));
    }
    args.push(OsString::from(plugin));
    args
}

fn run_agent_command<I, S>(
    runtime: AgentRuntime,
    phase: &'static str,
    program: &'static str,
    args: I,
    command_timeout: Duration,
    runner: &mut impl CommandRunner,
) -> Result<String, AgentPluginError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let rendered = render_command(program, &args);
    let output = runner
        .run(program, &args, command_timeout)
        .map_err(|failure| {
            match failure {
                CommandRunError::Spawn(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    AgentPluginError::CliMissing {
                        runtime: runtime.id(),
                        cli: program,
                    }
                }
                CommandRunError::Spawn(source) => {
                    // The process never spawned: this is not a CliFailed, whose
                    // command field names an executed command.
                    AgentPluginError::CliSpawnFailed {
                        runtime: runtime.id(),
                        phase,
                        command: rendered.clone(),
                        reason: source.to_string(),
                    }
                }
                CommandRunError::Supervision(source) => AgentPluginError::CliFailed {
                    runtime: runtime.id(),
                    phase,
                    command: rendered.clone(),
                    status: None,
                    stderr: format!("could not monitor spawned command: {source}"),
                },
            }
        })?;
    if output.success {
        return Ok(rendered);
    }
    Err(AgentPluginError::CliFailed {
        runtime: runtime.id(),
        phase,
        command: rendered,
        status: output.status_code,
        stderr: summarize_child_output(&output),
    })
}

#[derive(Debug, Error)]
enum CommandRunError {
    #[error("process did not spawn: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("spawned process could not be monitored: {0}")]
    Supervision(#[source] std::io::Error),
}

trait CommandRunner {
    fn run(
        &mut self,
        program: &str,
        args: &[OsString],
        command_timeout: Duration,
    ) -> Result<ProcessOutput, CommandRunError>;
}

struct NativeRunner;

impl CommandRunner for NativeRunner {
    fn run(
        &mut self,
        program: &str,
        args: &[OsString],
        command_timeout: Duration,
    ) -> Result<ProcessOutput, CommandRunError> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(CommandRunError::Spawn)?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    return child
                        .wait_with_output()
                        .map(ProcessOutput::from)
                        .map_err(CommandRunError::Supervision);
                }
                Ok(None) => {}
                Err(source) => {
                    // Avoid leaving a known-running child behind when process
                    // supervision itself fails. The original error remains
                    // the useful diagnostic even if best-effort cleanup fails.
                    if child.kill().is_ok() {
                        let _ = child.wait();
                    }
                    return Err(CommandRunError::Supervision(source));
                }
            }
            if start.elapsed() >= command_timeout {
                if let Err(source) = child.kill() {
                    match child.try_wait() {
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => {
                            return Err(CommandRunError::Supervision(source));
                        }
                    }
                }
                let mut output = child
                    .wait_with_output()
                    .map(ProcessOutput::from)
                    .map_err(CommandRunError::Supervision)?;
                output.success = false;
                output.status_code = None;
                output.stderr = format!(
                    "native CLI timed out after {:.3}s",
                    command_timeout.as_secs_f64()
                )
                .into_bytes();
                return Ok(output);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl From<std::process::Output> for ProcessOutput {
    fn from(output: std::process::Output) -> Self {
        Self {
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

fn render_command(program: &str, args: &[OsString]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| quote_arg(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_arg(arg: &OsStr) -> String {
    let text = arg.to_string_lossy();
    if text.chars().all(is_shell_display_safe) {
        return text.into_owned();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn is_shell_display_safe(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '@' | '=')
}

fn summarize_child_output(output: &ProcessOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return truncate_message(stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return truncate_message(stdout);
    }
    match output.status_code {
        Some(code) => format!("native CLI exited with status {code}"),
        None => "native CLI terminated without an exit status".to_string(),
    }
}

fn truncate_message(message: String) -> String {
    const MAX_CHARS: usize = 2048;
    let mut chars = message.chars();
    let truncated: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn agent_selector_parses_and_expands_without_a_cli_framework() {
        assert_eq!("codex".parse(), Ok(AgentSelector::Codex));
        assert_eq!("claude".parse(), Ok(AgentSelector::Claude));
        assert_eq!("all".parse(), Ok(AgentSelector::All));
        assert_eq!(
            AgentSelector::All.runtimes(),
            [AgentRuntime::Codex, AgentRuntime::Claude]
        );
        assert_eq!(AgentSelector::Claude.to_string(), "claude");
        assert!("other".parse::<AgentSelector>().is_err());
    }

    #[cfg(feature = "clap")]
    #[test]
    fn clap_feature_exposes_selector_as_a_value_enum() {
        use clap::ValueEnum;

        assert_eq!(
            AgentSelector::value_variants(),
            [
                AgentSelector::Codex,
                AgentSelector::Claude,
                AgentSelector::All,
            ]
        );
        assert_eq!(
            <AgentSelector as clap::ValueEnum>::from_str("codex", true),
            Ok(AgentSelector::Codex)
        );
    }

    #[test]
    fn codex_install_uses_marketplace_add_and_plugin_add() -> Result<(), OperationError> {
        let plugin = plugin();
        let request = InstallRequest::new(
            MarketplaceSource::new("owner/repo")
                .with_git_ref("main")
                .with_sparse_path("plugins/veloq"),
            plugin,
        );
        let mut runner = FakeRunner::successes(2);

        let outcome = install_with_runner(AgentRuntime::Codex, request, &mut runner)?;

        assert_eq!(outcome.runtime, AgentRuntime::Codex);
        assert_eq!(
            runner.calls,
            vec![
                call(
                    "codex",
                    [
                        "plugin",
                        "marketplace",
                        "add",
                        "--ref",
                        "main",
                        "--sparse",
                        "plugins/veloq",
                        "owner/repo",
                    ],
                ),
                call("codex", ["plugin", "add", "veloq@veloq"]),
            ]
        );
        assert_eq!(
            outcome.commands,
            vec![
                "codex plugin marketplace add --ref main --sparse plugins/veloq owner/repo",
                "codex plugin add veloq@veloq",
            ]
        );
        Ok(())
    }

    #[test]
    fn claude_install_supports_scopes_and_sparse_paths() -> Result<(), OperationError> {
        let plugin = plugin();
        let request = InstallRequest::new(
            MarketplaceSource::new("owner/repo").with_sparse_path(".claude-plugin"),
            plugin,
        )
        .with_marketplace_scope("project")
        .with_plugin_scope("local");
        let mut runner = FakeRunner::successes(2);

        let outcome = install_with_runner(AgentRuntime::Claude, request, &mut runner)?;

        assert_eq!(outcome.runtime, AgentRuntime::Claude);
        assert_eq!(
            runner.calls,
            vec![
                call(
                    "claude",
                    [
                        "plugin",
                        "marketplace",
                        "add",
                        "--scope",
                        "project",
                        "--sparse",
                        ".claude-plugin",
                        "owner/repo",
                    ],
                ),
                call(
                    "claude",
                    ["plugin", "install", "--scope", "local", "veloq@veloq"]
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn codex_update_can_target_one_marketplace() -> Result<(), OperationError> {
        let request = UpdateRequest::new(plugin()).with_marketplace_name("veloq");
        let mut runner = FakeRunner::successes(2);

        let outcome = update_with_runner(AgentRuntime::Codex, request, &mut runner)?;

        assert_eq!(
            runner.calls,
            vec![
                call("codex", ["plugin", "marketplace", "upgrade", "veloq"]),
                call("codex", ["plugin", "add", "veloq@veloq"]),
            ]
        );
        assert_eq!(
            outcome.commands,
            vec![
                "codex plugin marketplace upgrade veloq",
                "codex plugin add veloq@veloq",
            ]
        );
        Ok(())
    }

    #[test]
    fn claude_update_refreshes_marketplace_before_plugin() -> Result<(), OperationError> {
        let request = UpdateRequest::new(plugin())
            .with_marketplace_name("veloq")
            .with_plugin_scope("user");
        let mut runner = FakeRunner::successes(2);

        let _ = update_with_runner(AgentRuntime::Claude, request, &mut runner)?;

        assert_eq!(
            runner.calls,
            vec![
                call("claude", ["plugin", "marketplace", "update", "veloq"]),
                call("claude", ["plugin", "update", "--scope", "user", "veloq"]),
            ]
        );
        Ok(())
    }

    #[test]
    fn codex_uninstall_uses_plugin_remove() -> Result<(), OperationError> {
        let request = UninstallRequest::new(plugin());
        let mut runner = FakeRunner::successes(1);

        let outcome = uninstall_with_runner(AgentRuntime::Codex, request, &mut runner)?;

        assert_eq!(
            runner.calls,
            vec![call("codex", ["plugin", "remove", "veloq@veloq"])]
        );
        assert_eq!(outcome.commands, vec!["codex plugin remove veloq@veloq"]);
        Ok(())
    }

    #[test]
    fn claude_uninstall_supports_plugin_scope() -> Result<(), OperationError> {
        let request = UninstallRequest::new(plugin()).with_plugin_scope("project");
        let mut runner = FakeRunner::successes(1);

        let outcome = uninstall_with_runner(AgentRuntime::Claude, request, &mut runner)?;

        assert_eq!(
            runner.calls,
            vec![call(
                "claude",
                ["plugin", "uninstall", "--scope", "project", "veloq"]
            )]
        );
        assert_eq!(
            outcome.commands,
            vec!["claude plugin uninstall --scope project veloq"]
        );
        Ok(())
    }

    #[test]
    fn unsupported_options_fail_before_running_commands() {
        let request = InstallRequest::new(MarketplaceSource::local(Path::new(".")), plugin())
            .with_plugin_scope("user");
        let mut runner = FakeRunner::successes(1);

        let err = install_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                error: AgentPluginError::UnsupportedOption {
                    runtime: "codex",
                    option: "plugin_scope",
                    ..
                },
                ..
            })
        ));
        assert!(runner.calls.is_empty());
    }

    #[test]
    fn unsupported_uninstall_options_fail_before_running_commands() {
        let request = UninstallRequest::new(plugin()).with_plugin_scope("user");
        let mut runner = FakeRunner::successes(1);

        let err = uninstall_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                error: AgentPluginError::UnsupportedOption {
                    runtime: "codex",
                    option: "plugin_scope",
                    ..
                },
                ..
            })
        ));
        assert!(runner.calls.is_empty());
    }

    #[test]
    fn missing_cli_returns_structured_error() {
        let request = UpdateRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Err(CommandRunError::Spawn(
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        ))]);

        let err = update_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliMissing {
                    runtime: "codex",
                    cli: "codex"
                },
            }) if completed.is_empty()
        ));
    }

    #[test]
    fn failing_cli_prefers_stderr_and_captures_status() {
        let request = UpdateRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Ok(ProcessOutput {
            success: false,
            status_code: Some(17),
            stdout: b"stdout detail".to_vec(),
            stderr: b"stderr detail".to_vec(),
        })]);

        let err = update_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliFailed {
                    runtime: "codex",
                    phase: "marketplace-upgrade",
                    status: Some(17),
                    stderr,
                    ..
                },
            }) if stderr == "stderr detail" && completed.is_empty()
        ));
    }

    #[test]
    fn failing_uninstall_is_a_structured_error() {
        let request = UninstallRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Ok(ProcessOutput {
            success: false,
            status_code: Some(1),
            stdout: Vec::new(),
            stderr: b"plugin is not installed".to_vec(),
        })]);

        let err = uninstall_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliFailed {
                    runtime: "codex",
                    phase: "plugin-remove",
                    status: Some(1),
                    stderr,
                    ..
                },
            }) if stderr == "plugin is not installed" && completed.is_empty()
        ));
    }

    #[test]
    fn a_probe_that_never_spawned_stays_out_of_the_trace() {
        let request = UpdateRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Err(CommandRunError::Spawn(
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ))]);

        let outcome = check_operation_with_runner(
            AgentRuntime::Codex,
            AgentPluginOperation::Update,
            DEFAULT_COMMAND_TIMEOUT,
            &mut runner,
        );

        assert_eq!(outcome.status, DoctorStatus::Missing);
        assert!(
            outcome.commands.is_empty(),
            "an unspawned probe is not evidence: {:?}",
            outcome.commands
        );
        let _ = request;
    }

    #[test]
    fn a_spawn_failure_is_not_a_ran_command() {
        let request = UninstallRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Err(CommandRunError::Spawn(
            std::io::Error::other("fork bomb"),
        ))]);

        let err = uninstall_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliSpawnFailed {
                    runtime: "codex",
                    phase: "plugin-remove",
                    ..
                },
            }) if completed.is_empty()
        ));
    }

    #[test]
    fn a_spawn_failure_after_a_successful_mutation_keeps_the_prefix() {
        let request = InstallRequest::new(MarketplaceSource::new("owner/repo"), plugin());
        let mut runner = FakeRunner::from_outputs([
            Ok(ProcessOutput {
                success: true,
                status_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Err(CommandRunError::Spawn(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            ))),
        ]);

        let err = install_with_runner(AgentRuntime::Codex, request, &mut runner);

        // The marketplace mutation completed and is evidence; the unspawned
        // plugin add is not — but excluding it must not erase the prefix.
        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliSpawnFailed {
                    phase: "plugin-add",
                    ..
                },
            }) if completed == vec!["codex plugin marketplace add owner/repo".to_owned()]
        ));
    }

    #[test]
    fn a_mid_operation_failure_reports_the_completed_commands() {
        let request = InstallRequest::new(MarketplaceSource::new("owner/repo"), plugin());
        let mut runner = FakeRunner::from_outputs([
            Ok(ProcessOutput {
                success: true,
                status_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Ok(ProcessOutput {
                success: false,
                status_code: Some(7),
                stdout: Vec::new(),
                stderr: b"plugin add exploded".to_vec(),
            }),
        ]);

        let err = install_with_runner(AgentRuntime::Codex, request, &mut runner);

        // The marketplace registration succeeded and mutated the runtime;
        // the failure must not hide it.
        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliFailed {
                    phase: "plugin-add",
                    ..
                },
            }) if completed == vec!["codex plugin marketplace add owner/repo".to_owned()]
        ));
    }

    #[test]
    fn a_supervision_failure_is_an_executed_command() {
        let request = UninstallRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Err(CommandRunError::Supervision(
            std::io::Error::other("wait failed"),
        ))]);

        let err = uninstall_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(OperationError {
                completed,
                error: AgentPluginError::CliFailed {
                    runtime: "codex",
                    phase: "plugin-remove",
                    command,
                    status: None,
                    stderr,
                },
            }) if completed.is_empty()
                && command == "codex plugin remove veloq@veloq"
                && stderr.contains("wait failed")
        ));
    }

    #[test]
    fn a_probe_supervision_failure_enters_the_trace() {
        let mut runner = FakeRunner::from_outputs([Err(CommandRunError::Supervision(
            std::io::Error::other("wait failed"),
        ))]);

        let outcome = check_operation_with_runner(
            AgentRuntime::Codex,
            AgentPluginOperation::Uninstall,
            DEFAULT_COMMAND_TIMEOUT,
            &mut runner,
        );

        assert_eq!(outcome.status, DoctorStatus::Failed);
        assert_eq!(outcome.commands, ["codex plugin remove --help"]);
        assert!(matches!(
            outcome.message.as_deref(),
            Some(message) if message.contains("could not monitor spawned command")
                && message.contains("wait failed")
        ));
    }

    #[test]
    fn command_display_quotes_unsafe_arguments() -> Result<(), OperationError> {
        let request = InstallRequest::new(
            MarketplaceSource::local(Path::new("path with space")),
            plugin(),
        );
        let mut runner = FakeRunner::successes(2);

        let outcome = install_with_runner(AgentRuntime::Codex, request, &mut runner)?;

        assert_eq!(
            outcome.commands,
            vec![
                "codex plugin marketplace add 'path with space'",
                "codex plugin add veloq@veloq",
            ]
        );
        Ok(())
    }

    #[test]
    fn doctor_checks_required_subcommands() {
        let mut runner = FakeRunner::successes(5);

        let outcome = check_operation_with_runner(
            AgentRuntime::Codex,
            AgentPluginOperation::Doctor,
            DEFAULT_COMMAND_TIMEOUT,
            &mut runner,
        );

        assert_eq!(outcome.status, DoctorStatus::Ready);
        assert_eq!(
            runner.calls,
            vec![
                call("codex", ["plugin", "--help"]),
                call("codex", ["plugin", "marketplace", "add", "--help"]),
                call("codex", ["plugin", "marketplace", "upgrade", "--help"]),
                call("codex", ["plugin", "add", "--help"]),
                call("codex", ["plugin", "remove", "--help"]),
            ]
        );
        assert_eq!(
            outcome.commands,
            vec![
                "codex plugin --help",
                "codex plugin marketplace add --help",
                "codex plugin marketplace upgrade --help",
                "codex plugin add --help",
                "codex plugin remove --help",
            ]
        );
    }

    #[test]
    fn operation_readiness_checks_only_required_subcommands() {
        let mut runner = FakeRunner::successes(2);

        let outcome = check_operation_with_runner(
            AgentRuntime::Codex,
            AgentPluginOperation::Install,
            DEFAULT_COMMAND_TIMEOUT,
            &mut runner,
        );

        assert_eq!(outcome.status, DoctorStatus::Ready);
        assert_eq!(
            runner.calls,
            vec![
                call("codex", ["plugin", "marketplace", "add", "--help"]),
                call("codex", ["plugin", "add", "--help"]),
            ]
        );
    }

    #[test]
    fn doctor_reports_failed_required_subcommand() {
        let mut runner = FakeRunner::from_outputs([
            Ok(ProcessOutput::success()),
            Ok(ProcessOutput {
                success: false,
                status_code: Some(2),
                stdout: Vec::new(),
                stderr: b"missing subcommand".to_vec(),
            }),
        ]);

        let outcome = check_operation_with_runner(
            AgentRuntime::Codex,
            AgentPluginOperation::Doctor,
            DEFAULT_COMMAND_TIMEOUT,
            &mut runner,
        );

        assert_eq!(outcome.status, DoctorStatus::Failed);
        assert_eq!(outcome.commands.len(), 2);
        assert!(matches!(
            outcome.message.as_deref(),
            Some(message)
                if message.contains("codex plugin marketplace add --help")
                    && message.contains("missing subcommand")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn native_runner_closes_stdin_and_captures_child_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let args = [
            OsString::from("-c"),
            OsString::from(
                r#"#!/bin/sh
if read line; then
  echo "stdin stayed open: $line" >&2
  exit 9
fi
echo "child stdout"
echo "child stderr" >&2
exit 7
"#,
            ),
        ];
        let mut runner = NativeRunner;
        let output = runner.run("sh", &args, DEFAULT_COMMAND_TIMEOUT)?;

        assert!(!output.success);
        assert_eq!(output.status_code, Some(7));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "child stdout"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "child stderr"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_runner_times_out() -> Result<(), Box<dyn std::error::Error>> {
        let args = [
            OsString::from("-c"),
            OsString::from(
                r#"#!/bin/sh
sleep 2
echo "too late"
"#,
            ),
        ];
        let mut runner = NativeRunner;
        let output = runner.run("sh", &args, Duration::from_millis(50))?;

        assert!(!output.success);
        assert_eq!(output.status_code, None);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("native CLI timed out after"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    fn plugin() -> PluginRef<'static> {
        PluginRef {
            selector: "veloq@veloq",
            name: "veloq",
        }
    }

    fn call<const N: usize>(program: &str, args: [&str; N]) -> (String, Vec<String>) {
        (
            program.to_string(),
            args.into_iter().map(ToString::to_string).collect(),
        )
    }

    struct FakeRunner {
        calls: Vec<(String, Vec<String>)>,
        outputs: VecDeque<Result<ProcessOutput, CommandRunError>>,
    }

    impl FakeRunner {
        fn successes(count: usize) -> Self {
            let outputs = (0..count).map(|_| Ok(ProcessOutput::success())).collect();
            Self {
                calls: Vec::new(),
                outputs,
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

    impl CommandRunner for FakeRunner {
        fn run(
            &mut self,
            program: &str,
            args: &[OsString],
            _command_timeout: Duration,
        ) -> Result<ProcessOutput, CommandRunError> {
            self.calls.push((
                program.to_string(),
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

    impl ProcessOutput {
        fn success() -> Self {
            Self {
                success: true,
                status_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        }
    }
}
