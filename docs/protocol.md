# Protocol findings

Research snapshot: September 19, 2026. Tested Codex 0.155.1, Devin 3000.10.31, and the authenticated enterprise gateway with `gpt-6-astra-medium`.

**This adapter does not run the full Codex harness.** Flat tools and text work in the tested subset. Developer messages, namespaces and Responses Lite tool placement have no demonstrated lossless gateway mapping. Requests needing them are rejected before inference.

Live prompts were synthetic. Private captures and credentials are not included.

## Gateway tests

The account used `server.enterprise.windsurf.com`, not the public host listed in model metadata. The tested RPC was `/exa.api_server_pb.ApiServerService/GetChatMessage`, using Connect-framed protobuf.

| Test | Observed result |
| --- | --- |
| Tiny prompt without tools | Text response succeeded. |
| Top-level instruction versus conflicting user instruction | Top-level instruction won twice. |
| Same instruction in history role 5 | User instruction won twice. |
| Nonce in role 5 | Returned `Unknown`; user history and top-level prompt retained the same nonce. |
| Custom JSON function and result | Correct continuation. |
| Custom Lark tool and result | Raw input and continuation accepted. |
| Full flat Codex `exec` definition | Unchanged 25,315-byte description and 177-byte grammar accepted. No JavaScript was executed; a synthetic result was returned. |
| Encrypted reasoning replay | Follow-up succeeded. Acceptance does not prove the server used the opaque state. |

Responses included `final_answer` phase and an OpenAI reasoning signature. No successful probe supplied an actual-model UID: the requested alias is not independent proof of model identity. Successful calls reported nonzero committed ACU costs. Enterprise access is not evidence of unmetered inference. Small probe durations are not benchmarks.

HTTP 200 can carry a terminal Connect error. The JSON codec also accepted invented fields and enum strings while ignoring their content. Success alone does not establish field support.

## Namespace and role blockers

| Namespace probe | Result |
| --- | --- |
| `record` with `server_name=orchard`; request `orchard.record` | Model reported that the namespaced tool was unavailable. |
| Same declaration; request bare `record` | Native `record` call succeeded. |
| Duplicate `record` names with different server names/descriptions | Returned only bare `record`; namespace ownership was lost. |
| Literal flat name `orchard.record` | Connect `invalid_argument`. |
| JSON tool `namespace` property | Returned only bare `record`. |
| JSON top-level `tool_namespaces` | Model reported the tool unavailable. |

These exclude the tested shortcuts, not every possible undocumented representation. Renaming tools to `orchard__record` would change what the model sees.

Recovered roles are USER=1, SYSTEM=2, UNKNOWN=3, TOOL=4 and SYSTEM_PROMPT=5. The native assistant uses historical role 2. Role 5 lost a harmless nonce; JSON `SYSTEM_PROMPT` and candidate `DEVELOPER` enums did too. It cannot currently stand in for Codex developer messages.

Combining developer messages into one top-level prompt would lose their separate priorities and positions. Moving `additional_tools` into a global tool list would also change placement. Neither transformation is enabled.

## What Codex sends

Official release `rust-v0.155.1` resolves to `be2951ea34f0d295ed0becf97079f92fa5f6950e`. Installed-client loopback captures confirmed the source behavior without OpenAI inference.

The Astra catalog selects Responses Lite, code mode, v2 delegation, unified exec and freeform patches. Preserve the full catalog and instruction templates; an unknown model alias can change the harness before translation.

The captured request contained a developer `additional_tools` item, four developer messages and two user messages. Tool namespaces were `functions`, `clock` and `collaboration`. It selected medium effort, all-turn reasoning and low verbosity, requested encrypted reasoning, and disabled top-level parallel calls. Code-mode concurrency is separate.

The [tool serializer](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/tools/src/tool_spec.rs#L95) groups flat functions and custom tools into the real `functions` namespace. The [client](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/core/src/client.rs) inserts it as a developer item. Client source does not reveal the inference server's final token rendering; flat-tool equivalence is unproven.

## Provider and authentication differences

| Surface | Custom provider | Built-in OpenAI with loopback `openai_base_url` |
| --- | --- | --- |
| Lite framing | Retained | Retained |
| Internal metadata and encrypted function arguments | Stripped | Retained |
| Remote compaction | Unsupported capability | V2 `compaction_trigger` capability |
| Standalone web search | Requires explicit capability flag | Enabled by provider identity |

In matched captures, all six message content arrays were identical. The built-in route added internal metadata and 236 lines of web-tool definitions inside `functions.exec`: 25,315 bytes versus 12,889. Counting outer tools missed this difference. Built-in requests used zstd; HTTP fallback worked after a 426 WebSocket response.

These gates are defined in [provider capabilities](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider/src/provider.rs), [provider configuration](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider-info/src/lib.rs) and [web search](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/ext/web-search/src/extension.rs). Backend-only routing also checks for a `/backend-api/codex` URL suffix. Changing the URL does not implement those routes.

For `codex exec`, a process-local `CODEX_API_KEY` replaced existing auth with the adapter token. The captured request kept message content and metadata, but lost the image-generation tool definition. Interactive TUI source disables this environment-key shortcut; its auth setup remains untested. Inbound OpenAI authorization and account headers must never become Cognition credentials.

No Codex patch is needed to reach the adapter. A patch cannot make the gateway preserve roles or namespaces that the tested route loses. [Official configuration documentation](https://developers.openai.com/codex/config-advanced).

## Implemented subset and validation

The Rust adapter supports text, flat function/custom tools, unchanged Lark grammars, phases and restricted encrypted-reasoning replay. It uses separate local authentication and independently constructs upstream credentials. Redirects and automatic upstream retries are disabled.

Input audit rejects unknown semantics before inference. Checked Connect termination is required before any tool completion. Unknown tools, `tool_choice=none` violations and multiple calls fail closed. Tests cover framing, compression limits, error trailers, replay, ordering, authentication separation and downstream SSE cancellation. They make no paid calls automatically; run the repository's current tests for current counts.

Live Rust checks completed a flat custom-tool/result exchange and a text/reasoning replay. An actual stock Codex request reached the adapter and received the intended 422 rejection. Codex retried five times before displaying that error; no full agent completion was demonstrated.

Output is buffered until checked completion because the gateway can send its response-scoped signature after answer text. The adapter puts that signature before its one associated assistant item for replay. SSE lifecycle/keepalive events can arrive earlier, but tokens do not stream live. Multi-item reasoning attachment is restricted pending a dedicated probe.

Sampling defaults follow the inspected Devin client; equivalence to OpenAI defaults is unproven. Cache counters have no established overlap rule, so nonzero cache activity leaves standard token totals unavailable. Raw counters and ACU costs are in `x_devin_usage`. Expired auth requires Devin login and adapter restart.

## Remaining work

1. Establish native developer roles, namespace identity and `additional_tools` placement, including later tool updates.
2. Verify encrypted function arguments, multi-item reasoning, long-session compaction and v2 delegation.
3. Implement or explicitly reject search, images, memory and other separate backend routes.
4. Verify interactive authentication and improve rejected-request retry behavior.
5. Compare behavior on the same tasks and model settings. Transport success does not prove conversational parity or billing entitlement.

No alternative lossless Responses route was established. Devin's `openai` backend uses separately configured provider credentials. Recovered `GetDevstralStream` and completion RPCs did not establish the needed representation and were not called. The [public API](https://docs.devin.ai/api-reference/overview) describes agent/control operations. [devin-2api](https://github.com/leookun/devin-2api/blob/e7ddc55d822283b4774495a9868d181cd7c25fb2/internal/adapter/devin/devin.go#L117) also translates through `GetChatMessage`. This bounded search does not prove that no private route exists.

## Native Devin comparison

With imports and subagents disabled, `agent.codex_tools` changed the captured prompt from 18,448 to 14,416 bytes and tools from 23 to 14. It adds Codex-like persistence, steering and permission guidance. Its grammar-based `apply_patch` is closer, but `exec` remains a JSON shell tool, not Codex JavaScript orchestration. Both prompts still mentioned subagent profiles. This proves a prompt/tool switch, not improved behavior.

Compare direct Astra with this switch first; Fusion adds another model/context. See the [changelog](https://docs.devin.ai/cli/changelog/stable), [configuration](https://docs.devin.ai/cli/reference/configuration/config-file), [import controls](https://docs.devin.ai/cli/reference/configuration/read-config-from), [Fusion](https://docs.devin.ai/cli/fusion) and [enterprise accounting](https://docs.devin.ai/cli/enterprise/devin-auth).

## Schema sources

The [official Codeium extension 1.48.2](https://open-vsx.org/extension/Codeium/codeium) confirms older core fields and roles. [Third-party jeopi-catalog 16.5.1](https://registry.npmjs.org/jeopi-catalog/16.5.1) supplied newer candidate fields, selectively validated against captures. Neither is a supported current Cognition API contract.

Key fields: request prompt 2, history 3, tools 10, model 21; history signature 12, signature type 18, phase 19; tool custom flag 9, grammar 10–11, newer strictness 12; response signature 10, signature type 21, actual-model UID 23, phase 25. Completion field 3 is max newlines, not context length. Strictness numbering differs between schema versions.

Devin binary strings contain namespace types but also multiple inference backends. Their presence does not prove gateway support; no complete executed namespace conversion was recovered from assembly. The inspected [opencodex adapter](https://github.com/lidge-jun/opencodex/tree/134c92a01b120162f00c7275189cc47858720379/src/adapters/devin/cloud-direct) rewrites instructions/tools and omits newer state. It is a framing reference, not a parity baseline.
