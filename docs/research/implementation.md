# Rust implementation validation

Snapshot: 2026-09-19. This is a restricted protocol adapter, not a completed Codex harness replacement.

## Layout

- `config.rs`: existing Devin credential/deployment discovery and separate private local token.
- `proto.rs`: measured protobuf types, with optional scalar presence retained where stream interpretation needs it.
- `gateway.rs`: verified TLS, credential-isolated HTTP and bounded Connect frames. Redirects and automatic retries are disabled.
- `request.rs`: complete input audit before inference; only established mappings are encoded.
- `responses.rs`: response state, opaque reasoning, tool identity and output events. Tool policy violations and unknown tools fail before execution.
- `server.rs`: authenticated Axum endpoint, bounded zstd decoding, JSON/SSE, explicit unsupported routes and disconnect cancellation.
- `main.rs`: `serve`, `audit`, and private `token` setup. No live agent configuration changes.

The repository ships Rust only. The initial disposable Python investigation was replaced, not retained as a runtime dependency.

## Automated checks

`cargo test --locked`: 33 tests passed (29 unit, 4 HTTP integration).

`cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt --all -- --check`: passed.

Coverage includes fragmented/coalesced Connect frames, gzip limits, truncated streams and error trailers even under HTTP 200, independent upstream authentication, redirect rejection, explicit tool policy, grammar/schema preservation, malformed JSON value types, replay validation, text/tool ordering, response limits, zstd request limits, unknown semantics and endpoint rejection. A real socket test drops the downstream SSE consumer and observes the upstream response close.

No test contacts a paid inference endpoint automatically. The `roundtrip` example is an explicitly invoked live check.

## Live checks

The compiled Rust server used the existing authenticated enterprise deployment. Only synthetic task text was sent.

1. **Custom tool:** the unchanged captured Codex `exec` description (25,315 UTF-8 bytes) and its 177-byte Lark grammar produced a native custom call with raw JavaScript input. The test did not evaluate that input; it returned the expected synthetic tool result. The second request returned `BRIDGE_TOOL_OK`.
2. **Opaque reasoning:** a small divisor problem returned a response-scoped OpenAI encrypted reasoning item after answer text. The adapter emitted the reasoning item before the assistant answer. A following request replayed those items and asked for a simple follow-up; the gateway completed successfully. This proves acceptance, not that encrypted state was used internally.
3. **Native-request audit:** an actual captured Codex Astra request is rejected before inference for developer roles, Responses Lite AdditionalTools, internal message metadata, all-turn reasoning context, verbosity and other unimplemented fields. Private prompt text does not appear in the audit output.
4. **Installed Codex to Rust:** unmodified `codex exec` reached the Rust HTTP adapter using `openai_base_url` and a separate local API token. It received the intended HTTP 422 unsupported-semantics response. The client attempted five reconnects before surfacing that error; no full agent completion is claimed. This also leaves an error-UX improvement to investigate separately from inference parity.

These are transport/subset tests. They do not establish full Codex operation, identical server input, long-session quality, billing entitlement, interactive auth, image/search/delegation support or compaction.

## Deliberate limitations

- Output content is buffered until the checked gateway completion. SSE lifecycle/keepalive events can arrive earlier; token streaming does not.
- A response-scoped reasoning signature is replayable only with one associated assistant item in this release. Multi-item attachment needs a dedicated gateway probe.
- Gateway sampling defaults follow the inspected native Devin client; matching OpenAI's hidden defaults is unproven.
- Cached usage counters are exposed without inventing an input/cache overlap rule. Nonzero cache activity makes standard token totals unavailable; raw counters and ACU costs remain in `x_devin_usage`.
- Authentication expiry requires signing in with Devin and restarting the adapter. No refresh scheme was inferred.
- Reported model-family mismatches are rejected, but the gateway did not expose actual-model identity in the measured successful probes. The requested alias is not independent model attestation.

The remaining exact-harness blockers and next research questions are in [blockers.md](blockers.md) and the [README acceptance gates](../../README.md#acceptance-gates).
