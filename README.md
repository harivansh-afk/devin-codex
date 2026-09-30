# devin-codex

Use the Codex TUI with Devin inference.

A four-minute Astra-medium test passed 88/88 tasks across up to 10 instances with no observed request mixing and faster completion than default-tier Codex, though output buffering and a background-notification bug remain.

Requirements:

- installed Codex (tested: 0.155.1)
- Devin CLI on PATH.

```sh
nix profile add git+https://git.harivan.sh/harivansh-afk/devin-codex
devin-codex
```

For live terminal theme changes, use the package with a patched Codex renderer:

```sh
nix run .#with-terminal-theme
```

This variant supports Linux and Apple Silicon macOS. It includes Codex 0.154.0
with terminal color notifications and asynchronous OSC 10/11 replies. Chat backgrounds follow the terminal palette.
Terminals without color notifications refresh their colors when focus returns.
Explicit syntax themes, including custom `.tmTheme` files, keep their selected
colors. The standard package uses the installed Codex. `run --codex PATH`
selects another binary in either package.

# Development

- [x] Model selection
- [x] Reasoning effort selection
- [x] Function and grammar tools
- [x] Code-mode concurrency
- [x] Subagents
- [x] Image attachments
- [x] Screenshot tools
- [x] Session resume
- [x] Local compaction
- [x] Reasoning replay
- [x] Cancellation
- [ ] Token streaming
- [ ] Native web search
- [ ] Image generation
- [ ] Native instruction roles
- [ ] Native tool namespaces
- [ ] Remote compaction
- [ ] Encrypted agent messages
- [ ] Parallel tool calls
- [ ] Full inference controls
- [ ] Complete token accounting
- [ ] Opus harness compatibility
- [ ] Fusion gateway access

The standalone Responses server can load the same account-checked model catalog with
`devin-codex serve --compatibility --devin /absolute/path/to/devin --token-file /path/to/token`.
This enables GPT-6 Sol, Astra, and the other supported account models and reasoning levels.
Without `--devin`, the server retains its Astra-only behavior. Catalog lookup failures stop startup.

Gateway failures are retried inside the adapter, up to four attempts with 1/2/4-second
backoff and jitter. Retries replay the same inference request and discard the failed
attempt's buffered output, including tool calls and reasoning. Codex's own retries
remain disabled to avoid multiplying attempts. Cancellation also stops backoff.
While output is buffered, SSE data heartbeats reset Codex's idle timer every
15 seconds. Comment-only keepalives do not reset that timer.
HTTP 408/429/500/502/503/504, transient Connect errors (including `unknown`),
connection failures, idle timeouts, and truncated streams are eligible. Invalid
requests, authentication failures, malformed responses, and recognized credit or
context limits are not. Server retry delays are respected; waits over 60 seconds
are reported to the user instead of retried early.

Final Responses include `x_devin_gateway` with attempt counts and safe failure
metadata. CLI launches also append private diagnostics to
`$XDG_STATE_HOME/devin-codex/gateway.jsonl` (default
`~/.local/state/devin-codex/gateway.jsonl`), capped at 10 MiB. The log records
failure/recovery times, error categories, trace IDs when provided, and retry
scheduling. It never records upstream free text, prompts, tool output, or credentials.
A full or unwritable log does not interrupt inference. Unknown causes stay
unclassified; the adapter cannot reconstruct details already discarded by older versions.

Run `cargo test` for local regression tests. The opt-in
`cargo test --test live_recovery -- --ignored` checks a real Devin tool round trip
through injected gateway failures, using inference credits without executing model-generated code.
