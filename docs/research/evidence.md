# Evidence ledger

Recorded September 19, 2026. These are carried-forward findings from controlled research, not tests provided or rerun by this repository.

[Sanitized probe outcomes](gateway-probe-results.json) record the individual observed results. They are not a complete billing ledger or a repeatable test suite.

## Gateway

The tested enterprise deployment served Connect-framed protobuf at `https://server.enterprise.windsurf.com/exa.api_server_pb.ApiServerService/GetChatMessage`. Public model metadata named a different default host; deployment selection must follow the authenticated account.

| Fixture | Observation | Limit |
| --- | --- | --- |
| No tools, tiny prompt | Successful text response | No proof of hidden server input |
| Top-level instruction conflicts with user | Instruction won twice | One precedence condition |
| Same instruction in history role 5 | User won twice | Role 5 is not a demonstrated developer equivalent |
| Custom JSON function/result | Correct continuation | Synthetic result; no Codex execution |
| Custom Lark call/result | Raw input and continuation accepted | No outer namespace |
| Exact full Codex exec | 25,315-byte description and 177-byte grammar accepted | Does not test full additional-tools item |
| Encrypted reasoning replay | Accepted; arithmetic follow-up correct | Consumption of opaque state unproven |

Responses exposed an OpenAI reasoning signature and `final_answer` phase. No observed reply supplied an actual-model UID. HTTP 200 can contain a terminal Connect error. Successful responses reported nonzero committed ACU costs. Tiny observed request durations are not throughput or latency benchmarks.

## Codex request construction

Codex 0.155.1, matched to official release commit `be2951ea34f0d295ed0becf97079f92fa5f6950e`, was pointed at loopback mocks. Mocks deliberately returned errors; this tested construction only.

Both custom and built-in providers emitted Lite input with one developer additional-tools item, four developer messages, and two user messages. Tools occupied `functions`, `clock`, and `collaboration` namespaces. The request selected medium effort, all-turn reasoning, low verbosity, encrypted reasoning inclusion, and disabled top-level parallel calls. Code-mode concurrency is separate.

The matched built-in override preserved all six message content arrays. It added internal message metadata and 236 lines of nested web documentation inside `functions.exec.description`: 25,315 bytes instead of 12,889. Counting top-level tools alone missed this difference. Built-in requests used zstd; the mock rejected WebSocket upgrades with 426 and received the HTTP fallback.

Release source confirms custom providers strip internal message metadata and encrypted function arguments. Built-in identity retains those gates and V2 compaction selection. Backend-only routes have additional URL checks. The override therefore preserves more client behavior but does not implement required server routes. It also sent OpenAI authorization to loopback: an adapter must never forward that authorization upstream.

## Native Devin comparison baseline

With imports and subagents disabled, switching `agent.codex_tools` changed the captured main system prompt from 18,448 to 14,416 bytes and tools from 23 to 14. The enabled prompt adds Codex-like persistence, steering, permission, and writing guidance. It exposes grammar-based `apply_patch`, but `exec` remains a JSON shell tool rather than Codex's JavaScript orchestration tool. Both captured prompts still mentioned subagent profiles despite disabled subagent tools.

This proves a prompt/tool switch, not improved behavior. Native Devin remains a separate runtime and UI. Direct Astra with this switch is the useful comparison condition; Fusion adds another model/context and should be evaluated separately.
