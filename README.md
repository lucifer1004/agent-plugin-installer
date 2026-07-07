# agent-plugin-installer

`agent-plugin-installer` is a small Rust library for orchestrating native
agent plugin CLIs. It currently supports Codex and Claude by invoking their
public `plugin` commands for doctor, install, update, and uninstall workflows
and returning structured success or failure data.

The crate deliberately does not define package layout, JSON output, logging,
or release policy for a host application. Callers validate their own plugin
package and then pass marketplace/plugin metadata.

## Example

```rust
use std::path::Path;
use agent_plugin_installer::{
    install, AgentRuntime, InstallRequest, MarketplaceSource, PluginRef,
};

fn main() -> Result<(), agent_plugin_installer::OperationError> {
    let plugin = PluginRef {
        selector: "my-plugin@local",
        name: "my-plugin",
    };
    let marketplace = MarketplaceSource::local(Path::new("."));
    let request = InstallRequest::new(marketplace, plugin)
        .with_command_timeout(std::time::Duration::from_secs(30));

    let result = install(AgentRuntime::Codex, request)?;
    for command in result.commands {
        eprintln!("ran: {command}");
    }
    Ok(())
}
```

## Batch Orchestration

Hosts that support more than one agent runtime can use `AgentSelector` and the
`*_many` functions instead of rebuilding lifecycle coordination:

```rust
use agent_plugin_installer::{
    AgentSelector, FailurePolicy, InstallRequest, MarketplaceSource, PluginRef,
    install_many,
};

fn main() -> Result<(), agent_plugin_installer::BatchOperationError> {
    let request = InstallRequest::new(
        MarketplaceSource::new("owner/repo"),
        PluginRef {
            selector: "my-plugin@my-marketplace",
            name: "my-plugin",
        },
    );
    let report = install_many(
        AgentSelector::All,
        |_| request.clone(),
        FailurePolicy::StopOnFailure,
    )?;

    for outcome in &report.outcomes {
        eprintln!("{}: {:?}", outcome.runtime.id(), outcome.status);
    }
    Ok(())
}
```

The request provider runs exactly once per selected runtime before validation.
Return a cloned common request as above, or construct runtime-specific requests
when Codex and Claude need different Git refs, scopes, roots, or timeouts.

Batch mutations have three ordered stages:

1. Validate operation options for every selected runtime.
2. Run operation-specific readiness probes for every selected runtime.
3. Run mutations in runtime order.

Validation or preflight failure blocks every mutation. Once mutations begin,
`FailurePolicy::StopOnFailure` marks later runtimes skipped, while
`FailurePolicy::Continue` attempts them. Each outcome records only commands
that actually spawned, including preflight probes and any completed mutation
prefix.

The `*_many` functions return `BatchResult`. Any unsuccessful runtime produces
`BatchOperationError`, which owns the complete report so a host can emit every
success, failure, and skipped outcome before returning a non-zero exit status.

Package layout validation remains a host responsibility and should run before
calling a batch mutation.

## Optional Clap Integration

The crate has no CLI-framework dependency by default. Enable the `clap`
feature to use `AgentSelector` directly as a `clap::ValueEnum`; the host still
owns whether the selector is positional, optional, or defaulted. Add
`features = ["clap"]` to the host's dependency declaration.

## Runtime Mapping

- Codex install: `codex plugin marketplace add <root>`, then
  `codex plugin add <selector>`.
- Codex update: `codex plugin marketplace upgrade [marketplace]`, then
  `codex plugin add <selector>`.
- Codex uninstall: `codex plugin remove <selector>`.
- Claude install: `claude plugin marketplace add <root>`, then
  `claude plugin install <selector>`.
- Claude update: `claude plugin marketplace update [marketplace]`, then
  `claude plugin update <name>`.
- Claude uninstall: `claude plugin uninstall <name>`.

`MarketplaceSource` supports local paths or string sources accepted by the
native CLI. Codex supports Git refs and sparse paths; Claude supports sparse
paths plus marketplace/plugin scopes.

The child process stdout and stderr are captured. They are not forwarded to the
parent process, which lets host CLIs preserve their own stdout contract. Child
stdin is closed so native CLIs fail instead of blocking on interactive prompts.
Commands have a default 60 second timeout; install, update, and uninstall
requests can override it.

Failures before process creation are reported as spawn failures and excluded
from command traces. Failures while monitoring an already spawned process are
reported as executed-command failures and remain in the trace.

`doctor` checks the native subcommands used by install/update/uninstall, not
only the top-level plugin command. `check_operation` can be used when callers
need readiness for one modifying operation without requiring unrelated native
subcommands.
