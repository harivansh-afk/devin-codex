# devin-codex

Rust adapter for Devin inference. Uses your existing Devin login.

```text
Codex CLI → local Responses API → Devin gateway
```

**Not ready for daily Codex use.** Text and flat tools work. Full Codex requests
are rejected because developer roles and tool namespaces have no verified mapping.
The adapter adds no model instructions or tools.

## Install and test

From this checkout:

```sh
nix run . -- --help
nix run . -- test
```

`test` starts a private loopback server, makes two inference requests, checks a
custom-tool result, then stops. It uses Devin credits. It executes no generated code.
Sign in with Devin first. Use `--credentials PATH` for a different credentials file.

Install the CLI after this change is merged:

```sh
nix profile add git+https://git.harivan.sh/harivansh-afk/devin-codex
```

A host flake can use `inputs.devin-codex.packages.${system}.default` in its package list.
The flake packages the adapter only; Codex must already be on PATH.

## Test the Codex route

```sh
devin-codex run -- exec --ephemeral 'Reply hello'
```

This starts the adapter on a random loopback port and launches the installed
`codex exec`. It sets a local token and URL for that child only. It keeps your
Codex configuration, closes the listener on exit, and returns Codex's exit code.
Use `run --codex /path/to/codex -- exec ...` to select a binary.

**Expect an unsupported-request error today.** This tests routing, not a working
agent session. Interactive launch is not supported. API-key mode also changes
Codex's image-tool selection. No Codex fork is required for this route.

For a standalone server: `devin-codex token token-file`, then
`devin-codex serve --token-file token-file`. The endpoint is
`http://127.0.0.1:19476/v1/responses`; send the token as Bearer authorization.
Use `devin-codex audit request.json` to check a request without inference.

## Before production

- Find a gateway route that preserves developer roles, namespaces and Responses Lite.
- Verify reasoning replay, compaction, resume, delegation, images and search.
- Restore token streaming; content is currently buffered for late reasoning signatures.
- Verify interactive auth, credential renewal, limits and billing.
- Compare repeated short and long tasks against native Codex.

A client patch cannot restore fields the gateway drops. These remain protocol
work, not packaging work. [Evidence and sources](docs/protocol.md).

## Development

```sh
nix develop
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
nix flake check
```

`src/` holds the CLI, gateway types, translation and HTTP server. Unit tests sit
with their modules; `tests/` covers HTTP behavior. Tests use fake gateways and do
not spend inference credits. `Cargo.lock` and `flake.lock` pin dependencies.
