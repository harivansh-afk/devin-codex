# Devin ↔ Codex

Run the unmodified Codex CLI against Devin's enterprise inference gateway through a local protocol adapter. Keep Codex's instructions, tools, execution loop, approvals, and UI. Add **zero instructions and zero tools** in the adapter.

```text
Codex CLI → local Responses adapter → Devin enterprise inference gateway
```

**Status: experimental Rust adapter for a restricted Responses subset. Full Codex requests are not supported yet.** The adapter rejects unresolved developer messages, namespaces and Responses Lite rather than silently changing them. The actual Codex CLI has not completed an inference/tool round trip through it. The checklist below remains the acceptance backlog.

## Build and run

Requires Rust with edition 2024 support and an existing authenticated Devin CLI installation. No Python runtime, Codex fork, configuration rewrite or extra model instructions.

```sh
cargo build --release --locked
mkdir -p .local
./target/release/devin-codex token .local/token
./target/release/devin-codex serve --token-file .local/token
```

The server binds only `127.0.0.1:19476`. It reads the existing Devin credentials from `$XDG_DATA_HOME/devin/credentials.toml`, or `~/.local/share/devin/credentials.toml`, and uses that account's verified deployment origin. Incoming local authorization is never forwarded upstream. Credentials are loaded at startup; restart after signing in again. The adapter token is separate from the Devin credential.

In another terminal, test the implemented tool path explicitly:

```sh
DEVIN_CODEX_TOKEN="$(cat .local/token)" cargo run --locked --example roundtrip
```

This makes two metered inference requests with a synthetic custom tool and result. It does not execute generated code. It is not a full Codex acceptance test.

Audit an actual private Codex request without inference:

```sh
./target/release/devin-codex audit /absolute/private/request.json
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Audit exits 0 for the implemented subset, 1 for unsupported semantics, and 2 for setup/input errors. Keep captures outside Git. Do not point a daily Codex session here yet: an ordinary current Astra request intentionally receives HTTP 422 with the unsupported fields.

## Implemented boundary

- Authenticated HTTP Responses requests, including bounded zstd bodies; explicit WebSocket rejection allows clients to try SSE.
- User/assistant text, unchanged flat JSON functions and Lark custom tools, one tool call per response, text results and phases.
- Connect protobuf framing, gzip frames, successful terminal-envelope checks, cancellation, bounded responses, no automatic retry or redirect.
- OpenAI encrypted reasoning items carried back into gateway history. Replay acceptance does not prove server consumption.
- Explicit errors for unknown semantic fields and endpoints; no prompt merging, namespace flattening, tool-description rewriting or model fallback.

**SSE content is buffered until the gateway completes.** Live probes found reasoning signatures arriving after answer text. This version waits for the response-scoped signature, emits it before the associated assistant output, and only then finalizes tools. It preserves replayable output at the cost of token-streaming latency. This is a known UX difference to resolve, not a claim of identical streaming.

Reasoning associated with multiple assistant items in one response is rejected until its attachment can be verified. In particular, commentary plus a tool call plus one shared signature remains an acceptance gap. SSE keepalive comments continue while output is buffered.

Only `gpt-6-astra` is accepted, mapped to the observed gateway effort aliases. The adapter uses the inspected Devin client's sampling defaults when none are specified; equivalence to OpenAI's hidden defaults is unproven. Raw usage and metered ACU values are returned in `x_devin_usage`. Standard token totals remain null when cache overlap is unknown.

## Main blockers

1. **Developer-message boundaries:** the tested gateway system-message lane dropped a nonce that ordinary user and top-level prompt lanes retained. Merging developer text into one prompt changes the request.
2. **Namespaces and Responses Lite:** the gateway returns flat tool identities. Duplicate inner names are ambiguous; literal dotted names are rejected. No faithful `additional_tools` mapping is established.
3. **Hidden provider behavior:** direct inference works, but no supported contract proves absence of injected instructions/tools, model substitution, or sampling changes.
4. **Long sessions and backend features:** all-turn reasoning controls, encrypted function arguments, V2 compaction, images/search and delegation remain unsupported.
5. **Client auth and UX:** changing Codex provider or auth mode changes its tool surface. Interactive authentication and streamed continuation need separate tests.

No Codex patch is needed for the basic HTTP route. Patching Codex cannot recover roles or namespaces discarded by the gateway. The next fidelity milestone needs a verified alternative inference route or additional gateway support. We found no raw Responses passthrough under the existing Devin credential in the bounded investigation; this is not proof that none exists.

## What is established

Research recorded on September 19, 2026 used Codex 0.155.1 and Devin 3000.10.31. Results are account, version, and model specific.

- [x] Direct gateway inference works without Devin's outgoing prompt or tools.
- [x] Caller-defined JSON functions and custom Lark tools complete synthetic result round trips.
- [x] Codex's exact 25,315-byte `exec` description and grammar pass unchanged **without the outer namespace or complete tool catalog**.
- [x] The gateway returns OpenAI encrypted reasoning items and accepts their replay; actual consumption of replayed state remains unproven.
- [x] A local mock receives requests from the unmodified Codex CLI using `openai_base_url`.
- [x] The Rust adapter completes the exact flat Codex custom-tool description/grammar round trip against the live gateway.
- [x] The Rust adapter returns encrypted reasoning before its assistant output, and the gateway accepts a follow-up with that state replayed.

Successful probes do not prove the absence of hidden server instructions, tool injection, role conversion, or model substitution. That requires a supported contract or upstream visibility. No black-box experiment can establish identical hidden input. Successful requests also reported metered ACU costs; enterprise access does not establish zero marginal cost.

## Acceptance gates

### 1. Freeze the real baseline

- [ ] Record the Codex binary/release, complete model catalog, effective settings, feature flags, project instructions, and environment.
- [ ] Preserve Codex-facing `gpt-6-astra`; map the verified model and effort only at the gateway boundary. Test unknown aliases, model switches, and routing changes explicitly.
- [ ] Capture sanitized native fixtures and publish structural diffs and hashes. Never commit credentials, full personal prompts, private conversations, or opaque session state.
- [ ] Reproduce provider A/B results: the tested custom provider omitted 236 lines of nested web-tool documentation and strips metadata. Prefer testing the built-in URL override before considering a Codex fork.

### 2. Preserve model-facing semantics

- [ ] Compare instruction bytes, message roles, boundaries, order, phases, and content-part order across translation.
- [ ] Preserve Responses Lite `additional_tools`, namespace names/descriptions, deferred loading, all schemas, strictness, grammars, nested tool documentation, and raw custom input.
- [ ] Resolve the gateway's missing demonstrated developer-role and namespace representations. Do not concatenate developer messages or flatten names as an undocumented fallback.
- [ ] Preserve effort, verbosity, sampling, tool choice, parallelism, token limits, truncation, and `reasoning.context = all_turns`.
- [ ] Test unique tools and conflicting benign instructions; separately seek authoritative evidence about hidden gateway transformations.
- [ ] Reject every unsupported semantic field explicitly. No prompt additions, description rewriting, silent model fallback, or argument repair.

### 3. Complete real Codex sessions

- [ ] First milestone: actual Codex request → gateway call → local tool execution → result continuation → final answer.
- [ ] Verify streaming item IDs, interleaved deltas, commentary/final phases, custom calls, and malformed-stream handling.
- [ ] Preserve encrypted reasoning, internal message metadata, and encrypted function arguments through tool turns, restart, resume, and branching.
- [ ] Force **V2 compaction through Responses** early; verify constraints, opaque state, and tool history survive. Implementing only the older compact endpoint is insufficient.
- [ ] Test cancellation, steering, disconnect, reconnect, timeout, partial-output failure, and retry. Never replay completed tool side effects.

### 4. Cover the full advertised surface

- [ ] Exercise web search with its actual backend contract, images and image/tool-result ordering, dynamic app tools, and authorization.
- [ ] Exercise multi-agent v2 delegation, encrypted inter-agent messages, concurrent workers, and parent continuation.
- [ ] Inventory backend-only routes, memory generation, and realtime. Implement or explicitly declare unavailable; no substituted backend presented as equivalent.
- [ ] Test SSE and WebSocket negotiation/continuation separately; document any lifecycle or latency difference.

### 5. Make daily use dependable

- [x] Bind a high, uncommon loopback port; keep adapter state separate from existing Codex/Devin configuration.
- [x] Separate adapter authentication from Cognition credentials. Strip incoming OpenAI authorization/account headers and construct upstream auth independently.
- [ ] Validate enterprise routing, credential refresh/expiry, Connect terminal errors even on HTTP 200, rate limits, usage, ACU accounting, and failure visibility.
- [ ] Build repeatable fixtures and fault injection; compare direct Codex, bridged Codex, and native Devin's Codex preset under matched conditions.
- [ ] Repeat short and long tasks; score unwanted permission requests, ignored steering, premature stopping, tool errors, repeated edits, constraint retention, and answer quality.

“Ready” means no unexplained model-facing differences on the supported surface, passing lifecycle tests, and acceptable repeated behavioral comparisons. It does not mean identical stochastic outputs. Patch Codex only after a failing test demonstrates a client-side obstacle; a client patch cannot repair missing gateway semantics.

## Research

- [Evidence and unresolved boundaries](docs/research/evidence.md)
- [Sources and protocol notes](docs/research/sources.md)
- [Exact-harness blockers and live probes](docs/research/blockers.md)
- [Pinned Codex translation contract](docs/research/codex-translation-contract.md)
- [Rust implementation validation](docs/research/implementation.md)
