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
# Ok::<(), agent_plugin_installer::AgentPluginError>(())
```

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
Commands have a default 60 second timeout; install, update, and uninstall requests can
override it.

`doctor` checks the native subcommands used by install/update/uninstall, not
only the top-level plugin command. `check_operation` can be used when callers
need readiness for one modifying operation without requiring unrelated native
subcommands.
