# devin-codex

Use the Codex TUI with Devin inference.

Requires an installed Codex with the Astra catalog (tested: 0.155.1) and a Devin login.

```sh
nix profile add git+https://git.harivan.sh/harivansh-afk/devin-codex
devin-codex
```

From this checkout: `nix run .`. To update an existing install:
`nix profile upgrade devin-codex`.

Pass Codex arguments after `run --`:

```sh
devin-codex run -- resume --last
devin-codex run -- exec 'Fix the failing tests'
devin-codex run -- --no-alt-screen
```

`run --codex PATH -- ...` selects another Codex binary. `--credentials PATH`
selects a Devin credentials file. The flake packages the adapter; Codex must be
on PATH. Flake consumers can use `inputs.devin-codex.packages.${system}.default`.

## What changes

The launcher starts an authenticated loopback adapter and a child Codex process.
It uses the installed Codex's bundled Astra harness settings, your existing base
configuration, the `devin-codex` profile, and a temporary provider token. 


Codex writes profile settings to `$CODEX_HOME/devin-codex.config.toml` (normally under `~/.codex`);
your base configuration and saved login are not rewritten by the launcher.
The server and temporary catalog close with the session. Resume through
`devin-codex run -- resume`, not the plain `codex resume` command printed by Codex.

Compatibility mode combines developer instructions into the gateway system prompt,
joins text parts, and flattens tool namespaces. Tool names are restored on replies;
custom grammars, arguments and local execution stay intact. Each gateway response
allows one tool call; separate agent requests and concurrency inside Codex code mode
remain available. This changes model behavior.

Subagents use stock Codex v2 with plaintext task messages: the adapter removes the
`encrypted` schema marker only from collaboration message parameters and marks their
returned calls with `encrypted_function_args: []`, Codex's native plaintext dispatch
signal. Agent-message content is translated to text with sender/recipient identity.
Tasks and results are plaintext in local session history, not encrypted agent
payloads. HTTPS to the gateway and opaque model-reasoning replay are unchanged.
Existing encrypted agent messages still fail explicitly; start a fresh child/session
rather than expecting old encrypted handoffs to become readable.

Tested live: file reads, patches, shell commands, tests, multiple TUI turns, resume,
parallel children with inherited history, child progress messages, follow-up tasks,
manual compaction, and automatic compaction under a test-only lowered threshold.
No shared background server is required for these in-session subagents. Cross-session
`codex agents` background-server management is not provided by the launcher.
Full Codex parity is not claimed. Current gaps:

- Output arrives after each gateway response finishes, not token by token.
- Images, native web search and image generation are unavailable.
- Reasoning-context, summary, verbosity and service-tier settings are not forwarded.
- Encrypted agent histories remain unsupported; new delegation uses plaintext instead.
- Compaction uses Codex's local summarization path, not native remote compaction.
  Completed tool history is retained even when its tool definitions are no longer
  advertised. Long-session reliability and dynamic tool updates need more testing.
- Only Astra is exposed. Standard token totals can be unavailable when the gateway
  reports cache usage; the context indicator is not a reliable billing/usage meter.

Local MCP tools use the same translation, but have not been live-tested.
Unsupported content fails explicitly. [Protocol findings](docs/protocol.md).

## Stream failures

Gateway connections time out after 30 seconds; reads time out after 120 seconds
without data. An active inference stream has no two-minute total deadline.
Failures report the safe gateway code/HTTP status or the adapter validation reason,
not raw server messages or partial model/tool output. Automatic retries remain off.
After a failure, inspect the reported cause before continuing the session; previously
completed tool calls may already have changed files or external state.

## Development

```sh
nix develop
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
nix flake check
```

Tests use fake gateways. `devin-codex test` makes two live inference calls without
executing generated code. `audit request.json` checks the strict API subset.
`serve --token-file PATH` serves that subset; add `--compatibility` for Codex
translation. Create a private token file with `devin-codex token PATH`.
