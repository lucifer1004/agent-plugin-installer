//! Native CLI orchestration helpers for installing agent plugins.
//!
//! This crate intentionally does not know about any parent application's
//! JSON envelope, release process, or package validation rules. Callers
//! provide marketplace/plugin metadata; the crate invokes the selected
//! agent runtime's public CLI and returns command summaries or structured
//! errors.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum AgentPluginError {
    #[error("{runtime} CLI `{cli}` is not available on PATH")]
    CliMissing {
        runtime: &'static str,
        cli: &'static str,
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
) -> Result<PluginCommandOutcome, AgentPluginError> {
    let mut runner = NativeRunner;
    install_with_runner(runtime, request, &mut runner)
}

pub fn update(
    runtime: AgentRuntime,
    request: UpdateRequest<'_>,
) -> Result<PluginCommandOutcome, AgentPluginError> {
    let mut runner = NativeRunner;
    update_with_runner(runtime, request, &mut runner)
}

pub fn uninstall(
    runtime: AgentRuntime,
    request: UninstallRequest<'_>,
) -> Result<PluginCommandOutcome, AgentPluginError> {
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
        commands.push(command.clone());
        match runner.run(runtime.cli(), &args, command_timeout) {
            Ok(output) if output.success => {}
            Ok(output) => {
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
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Missing,
                    commands,
                    message: Some(format!("`{}` is not available on PATH", runtime.cli())),
                };
            }
            Err(source) => {
                return DoctorOutcome {
                    runtime,
                    status: DoctorStatus::Failed,
                    commands,
                    message: Some(format!("required command failed: {command}: {source}")),
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
) -> Result<PluginCommandOutcome, AgentPluginError> {
    validate_install_options(runtime, &request)?;
    let commands = match runtime {
        AgentRuntime::Codex => vec![
            run_agent_command(
                runtime,
                "marketplace-add",
                runtime.cli(),
                codex_marketplace_add_args(&request.marketplace),
                request.command_timeout,
                runner,
            )?,
            run_agent_command(
                runtime,
                "plugin-add",
                runtime.cli(),
                [
                    OsString::from("plugin"),
                    OsString::from("add"),
                    OsString::from(request.plugin.selector),
                ],
                request.command_timeout,
                runner,
            )?,
        ],
        AgentRuntime::Claude => vec![
            run_agent_command(
                runtime,
                "marketplace-add",
                runtime.cli(),
                claude_marketplace_add_args(&request),
                request.command_timeout,
                runner,
            )?,
            run_agent_command(
                runtime,
                "plugin-install",
                runtime.cli(),
                claude_scoped_plugin_args("install", request.plugin.selector, request.plugin_scope),
                request.command_timeout,
                runner,
            )?,
        ],
    };
    Ok(PluginCommandOutcome { runtime, commands })
}

fn update_with_runner(
    runtime: AgentRuntime,
    request: UpdateRequest<'_>,
    runner: &mut impl CommandRunner,
) -> Result<PluginCommandOutcome, AgentPluginError> {
    validate_update_options(runtime, &request)?;
    let commands = match runtime {
        AgentRuntime::Codex => vec![
            run_agent_command(
                runtime,
                "marketplace-upgrade",
                runtime.cli(),
                codex_marketplace_upgrade_args(request.marketplace_name),
                request.command_timeout,
                runner,
            )?,
            run_agent_command(
                runtime,
                "plugin-add",
                runtime.cli(),
                [
                    OsString::from("plugin"),
                    OsString::from("add"),
                    OsString::from(request.plugin.selector),
                ],
                request.command_timeout,
                runner,
            )?,
        ],
        AgentRuntime::Claude => vec![
            run_agent_command(
                runtime,
                "marketplace-update",
                runtime.cli(),
                claude_marketplace_update_args(request.marketplace_name),
                request.command_timeout,
                runner,
            )?,
            run_agent_command(
                runtime,
                "plugin-update",
                runtime.cli(),
                claude_scoped_plugin_args("update", request.plugin.name, request.plugin_scope),
                request.command_timeout,
                runner,
            )?,
        ],
    };
    Ok(PluginCommandOutcome { runtime, commands })
}

fn uninstall_with_runner(
    runtime: AgentRuntime,
    request: UninstallRequest<'_>,
    runner: &mut impl CommandRunner,
) -> Result<PluginCommandOutcome, AgentPluginError> {
    validate_uninstall_options(runtime, &request)?;
    let commands = match runtime {
        AgentRuntime::Codex => vec![run_agent_command(
            runtime,
            "plugin-remove",
            runtime.cli(),
            [
                OsString::from("plugin"),
                OsString::from("remove"),
                OsString::from(request.plugin.selector),
            ],
            request.command_timeout,
            runner,
        )?],
        AgentRuntime::Claude => vec![run_agent_command(
            runtime,
            "plugin-uninstall",
            runtime.cli(),
            claude_scoped_plugin_args("uninstall", request.plugin.name, request.plugin_scope),
            request.command_timeout,
            runner,
        )?],
    };
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
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                AgentPluginError::CliMissing {
                    runtime: runtime.id(),
                    cli: program,
                }
            } else {
                AgentPluginError::CliFailed {
                    runtime: runtime.id(),
                    phase,
                    command: rendered.clone(),
                    status: None,
                    stderr: source.to_string(),
                }
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

trait CommandRunner {
    fn run(
        &mut self,
        program: &str,
        args: &[OsString],
        command_timeout: Duration,
    ) -> std::io::Result<ProcessOutput>;
}

struct NativeRunner;

impl CommandRunner for NativeRunner {
    fn run(
        &mut self,
        program: &str,
        args: &[OsString],
        command_timeout: Duration,
    ) -> std::io::Result<ProcessOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let start = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                return child.wait_with_output().map(ProcessOutput::from);
            }
            if start.elapsed() >= command_timeout {
                let _ = child.kill();
                let mut output = child.wait_with_output().map(ProcessOutput::from)?;
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
    fn codex_install_uses_marketplace_add_and_plugin_add() -> Result<(), AgentPluginError> {
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
    fn claude_install_supports_scopes_and_sparse_paths() -> Result<(), AgentPluginError> {
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
    fn codex_update_can_target_one_marketplace() -> Result<(), AgentPluginError> {
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
    fn claude_update_refreshes_marketplace_before_plugin() -> Result<(), AgentPluginError> {
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
    fn codex_uninstall_uses_plugin_remove() -> Result<(), AgentPluginError> {
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
    fn claude_uninstall_supports_plugin_scope() -> Result<(), AgentPluginError> {
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
            Err(AgentPluginError::UnsupportedOption {
                runtime: "codex",
                option: "plugin_scope",
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
            Err(AgentPluginError::UnsupportedOption {
                runtime: "codex",
                option: "plugin_scope",
                ..
            })
        ));
        assert!(runner.calls.is_empty());
    }

    #[test]
    fn missing_cli_returns_structured_error() {
        let request = UpdateRequest::new(plugin());
        let mut runner = FakeRunner::from_outputs([Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        ))]);

        let err = update_with_runner(AgentRuntime::Codex, request, &mut runner);

        assert!(matches!(
            err,
            Err(AgentPluginError::CliMissing {
                runtime: "codex",
                cli: "codex"
            })
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
            Err(AgentPluginError::CliFailed {
                runtime: "codex",
                phase: "marketplace-upgrade",
                status: Some(17),
                stderr,
                ..
            }) if stderr == "stderr detail"
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
            Err(AgentPluginError::CliFailed {
                runtime: "codex",
                phase: "plugin-remove",
                status: Some(1),
                stderr,
                ..
            }) if stderr == "plugin is not installed"
        ));
    }

    #[test]
    fn command_display_quotes_unsafe_arguments() -> Result<(), AgentPluginError> {
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
        let temp_dir = unique_temp_dir("native-capture")?;
        let script = temp_dir.join("fake-agent");
        write_executable(
            &script,
            r#"#!/bin/sh
if read line; then
  echo "stdin stayed open: $line" >&2
  exit 9
fi
echo "child stdout"
echo "child stderr" >&2
exit 7
"#,
        )?;
        let mut runner = NativeRunner;
        let output = runner.run(
            script.as_os_str().to_string_lossy().as_ref(),
            &[],
            DEFAULT_COMMAND_TIMEOUT,
        )?;

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
        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_runner_times_out() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = unique_temp_dir("native-timeout")?;
        let script = temp_dir.join("slow-agent");
        write_executable(
            &script,
            r#"#!/bin/sh
sleep 2
echo "too late"
"#,
        )?;
        let mut runner = NativeRunner;
        let output = runner.run(
            script.as_os_str().to_string_lossy().as_ref(),
            &[],
            Duration::from_millis(50),
        )?;

        assert!(!output.success);
        assert_eq!(output.status_code, None);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("native CLI timed out after"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(temp_dir)?;
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
        outputs: VecDeque<std::io::Result<ProcessOutput>>,
    }

    impl FakeRunner {
        fn successes(count: usize) -> Self {
            let outputs = (0..count).map(|_| Ok(ProcessOutput::success())).collect();
            Self {
                calls: Vec::new(),
                outputs,
            }
        }

        fn from_outputs<const N: usize>(outputs: [std::io::Result<ProcessOutput>; N]) -> Self {
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
        ) -> std::io::Result<ProcessOutput> {
            self.calls.push((
                program.to_string(),
                args.iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            ));
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Err(std::io::Error::other("missing fake output")))
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

    #[cfg(unix)]
    fn unique_temp_dir(label: &str) -> std::io::Result<std::path::PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "agent-plugin-installer-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir(&dir)?;
        Ok(dir)
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body)?;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)
    }
}
