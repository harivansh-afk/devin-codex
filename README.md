# Devin ↔ Codex

Run the unmodified Codex CLI against Devin's enterprise inference gateway through a local protocol adapter. Keep Codex's instructions, tools, execution loop, approvals, and UI. Add **zero instructions and zero tools** in the adapter.

```text
Codex CLI → local Responses adapter → Devin enterprise inference gateway
```

**Status: research only. There is no working shim yet.** Direct gateway probes succeeded; the actual Codex CLI has not completed an inference/tool round trip through an adapter. The checklist below is the implementation and acceptance backlog, not a claim of compatibility.

## What is established

Research recorded on September 19, 2026 used Codex 0.155.1 and Devin 3000.10.31. Results are account, version, and model specific.

- [x] Direct gateway inference works without Devin's outgoing prompt or tools.
- [x] Caller-defined JSON functions and custom Lark tools complete synthetic result round trips.
- [x] Codex's exact 25,315-byte `exec` description and grammar pass unchanged **without the outer namespace or complete tool catalog**.
- [x] The gateway returns OpenAI encrypted reasoning items and accepts their replay; actual consumption of replayed state remains unproven.
- [x] A local mock receives requests from the unmodified Codex CLI using `openai_base_url`.

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

- [ ] Bind a high, uncommon loopback port; isolate the launch profile and runtime state.
- [ ] Separate adapter authentication from Cognition credentials. Strip incoming OpenAI authorization/account headers and construct upstream auth independently.
- [ ] Validate enterprise routing, credential refresh/expiry, Connect terminal errors even on HTTP 200, rate limits, usage, ACU accounting, and failure visibility.
- [ ] Build repeatable fixtures and fault injection; compare direct Codex, bridged Codex, and native Devin's Codex preset under matched conditions.
- [ ] Repeat short and long tasks; score unwanted permission requests, ignored steering, premature stopping, tool errors, repeated edits, constraint retention, and answer quality.

“Ready” means no unexplained model-facing differences on the supported surface, passing lifecycle tests, and acceptable repeated behavioral comparisons. It does not mean identical stochastic outputs. Patch Codex only after a failing test demonstrates a client-side obstacle; a client patch cannot repair missing gateway semantics.

## Research

- [Evidence and unresolved boundaries](docs/research/evidence.md)
- [Sources and protocol notes](docs/research/sources.md)
