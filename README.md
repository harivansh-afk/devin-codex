# devin-codex

Use the Codex TUI with Devin inference. Rust adapter; no Codex fork.
Requires an installed Codex with the Astra catalog (tested: 0.155.1) and a Devin login.
Inference uses your Devin account and may consume credits.

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
It uses the installed Codex's bundled Astra catalog, your existing configuration,
and a temporary provider token. Your saved login and configuration are not changed.
The server and temporary catalog close with the session. Resume through
`devin-codex run -- resume`, not the plain `codex resume` command printed by Codex.

Compatibility mode combines developer instructions into the gateway system prompt,
joins text parts, and flattens tool namespaces. Tool names are restored on replies;
custom grammars, arguments and local execution stay intact. Gateway calls are serial;
concurrency inside Codex code mode remains available. This changes model behavior.

Tested live: file reads, patches, shell commands, test execution, multiple TUI turns and session resume.
Full Codex parity is not claimed. Current gaps:

- Output arrives after each gateway response finishes, not token by token.
- Images, native web search and image generation are unavailable.
- Reasoning-context, summary, verbosity and service-tier settings are not forwarded.
- Long-session compaction, delegation and dynamic tool updates need more testing.
- Only `gpt-6-astra` is exposed; gateway aliases select the requested reasoning effort.

Local MCP tools use the same translation, but have not been live-tested.
Unsupported content fails explicitly. [Protocol findings](docs/protocol.md).

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
