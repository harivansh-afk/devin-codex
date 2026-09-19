# Protocol findings

Research snapshot: September 19, 2026. Tested Codex 0.155.1, Devin 3000.10.31, and the authenticated enterprise gateway with `gpt-6-astra-medium`.

**Compatibility mode runs the Codex TUI with deliberate translation losses.** The launcher enables it; standalone `serve` and `audit` retain the strict subset described below. Developer messages, namespaces and Responses Lite tool placement still have no lossless gateway mapping.

The launcher uses a custom Responses provider and the installed binary's bundled Astra catalog. This avoids modifying saved auth and disables unsupported native web tools. It merges developer/system messages into the top-level prompt, joins text parts and hoists tools from `additional_tools`. Namespaced tools receive flat aliases, restored to their original namespace/name on output and translated again in history. Colliding aliases fail before inference. Multiple assistant items may share response-scoped reasoning in compatibility mode; consumption by the backend remains unverified.

Live acceptance: stock Codex 0.155.1 created a Rust file, patched it, ran tests, and completed two interactive turns with three passing tests, then resumed the session and recalled its test cases. No Codex patch was needed. Historical strict-mode failures and provider differences below remain useful evidence, not a statement that the compatibility launcher rejects all Codex sessions.

Model-selector acceptance: the custom catalog preserves Astra's instruction templates,
code mode, Responses Lite, and v2 tool configuration while exposing the routing label
`gpt-6-astra-devin`. `/model` offered Low, Medium, High, Extra high and Max, without
Ultra. Switching to High completed live inference. The selector wrote the separate
`devin-codex` profile; the base Codex config hash was unchanged. The alias is normalized
only at the adapter boundary, and response events retain the visible alias. It does
not independently attest the backend model identity.

Delegation initially failed because the collaboration message schema requested
`encrypted: true`. The gateway honored that marker, but its chat history cannot
represent Codex's encrypted agent-message item. Codex already has a native plaintext
path: [`ToolCall::direct_source`](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/core/src/tools/router.rs)
selects it when collaboration `spawn_agent`, `send_message`, or `followup_task`
returns `encrypted_function_args: []`.

Compatibility mode now removes only those message-schema encryption markers and
returns the empty argument-encryption marker. It retains the v2 harness and converts
plaintext agent messages with sender/recipient identity. This is a plaintext transport
choice, not decryption; old encrypted messages still fail explicitly. Other tool
schemas, HTTPS, and opaque reasoning signatures are unchanged. The source confirms
provider inheritance in
[`build_agent_shared_config`](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/core/src/tools/handlers/multi_agents_common.rs).

Live delegation passed a fresh child task and a follow-up on that child. A second
TUI test spawned two children with full history before waiting; each edited a different
Rust module and sent progress messages. The parent ran all four fixture tests
successfully. Session records confirmed the Devin provider and plaintext handoffs.

Compaction uses Codex's local summarization path. A tool-heavy session initially
failed because historical tool calls were required to have current definitions and
nullable reasoning content was rejected. Both are now supported without advertising
historical tools for new calls; current output-tool validation remains unchanged.
Manual compaction then completed. A child reply arriving during a parent turn also
exposed standalone signed reasoning with no subsequent assistant item. Compatibility
mode now represents that reasoning-only turn with an empty assistant boundary before
a user/agent message or EOF, preserving the opaque signature without attaching it to
someone else's message. It does not invent assistant text or decrypt reasoning.
The session was resumed again with a test-only 1,000-token auto-compaction threshold.
Automatic compaction ran before inference and after tool execution; the parent retained
the marker and existing child's Unicode conclusion, reran all four tests, and completed.
The launcher does not change the production threshold. This is not native remote
compaction or proof of unbounded long-session reliability.

Initial live acceptance prompts were synthetic. Private captures and credentials are not included.

## Stream-failure investigation (0.3.1)

A reported session had two failures after completed tool results: one at 120.04
seconds and another at 28.16 seconds. Version 0.3.0 discarded both gateway error
codes and translator errors, reporting only `Upstream inference stream failed`.
The exact cause of the shorter failure cannot be recovered from that log.

The HTTP client also imposed a 120-second total deadline. A synthetic streaming
server reproduced cancellation despite continuous frames. The client now uses a
30-second connection timeout and a 120-second read-idle timeout instead; tests
verify both continued active streaming and cancellation of a stalled stream.
SSE and JSON failures retain safe gateway codes/statuses or static translation
reasons, with no raw server error bodies or partial tool arguments in failed output.
No automatic inference/tool retry was introduced.

Two inference-only replays from an ephemeral fork of the reported session completed
translation successfully, including one in the original project context. A local mock
captured the translated request in memory and rejected Codex before any model-generated
tools could run; replay then called the gateway directly with no tool executor attached.
No request bodies, credentials, or user session data were committed. This did not
reproduce or establish the historical cause of the 28-second failure; a recurrence
with the improved diagnostics is still needed.

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

Combining developer messages into one top-level prompt would lose their separate priorities and positions. Moving `additional_tools` into a global tool list would also change placement. Strict mode rejects both transformations; compatibility mode applies them explicitly.

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

For `codex exec`, a process-local `CODEX_API_KEY` replaced existing auth with the adapter token. The captured request kept message content and metadata, but lost the image-generation tool definition. Interactive TUI source disables this environment-key shortcut; the compatibility launcher instead uses a tested custom-provider environment key. Inbound OpenAI authorization and account headers must never become Cognition credentials.

No Codex patch is needed to reach the adapter. A patch cannot make the gateway preserve roles or namespaces that the tested route loses. [Official configuration documentation](https://developers.openai.com/codex/config-advanced).

## Implemented subset and validation

The Rust adapter supports text, flat function/custom tools, unchanged Lark grammars, phases and restricted encrypted-reasoning replay. It uses separate local authentication and independently constructs upstream credentials. Redirects and automatic upstream retries are disabled.

Input audit rejects unknown semantics before inference. Checked Connect termination is required before any tool completion. Unknown tools, `tool_choice=none` violations and multiple calls fail closed. Tests cover framing, compression limits, error trailers, replay, ordering, authentication separation and downstream SSE cancellation. They make no paid calls automatically; run the repository's current tests for current counts.

Live Rust checks completed a flat custom-tool/result exchange and a text/reasoning replay. An actual stock Codex request reached the adapter and received the intended 422 rejection. Codex retried five times before displaying that error; that strict-mode run did not attempt compatibility translation.

Output is buffered until checked completion because the gateway can send its response-scoped signature after answer text. The adapter puts that signature before its one associated assistant item for replay. SSE lifecycle/keepalive events can arrive earlier, but tokens do not stream live. Multi-item reasoning attachment is restricted pending a dedicated probe.

Sampling defaults follow the inspected Devin client; equivalence to OpenAI defaults is unproven. Cache counters have no established overlap rule, so nonzero cache activity leaves standard token totals unavailable. Raw counters and ACU costs are in `x_devin_usage`. Expired auth requires Devin login and adapter restart.

## Remaining work

1. Establish native developer roles, namespace identity and `additional_tools` placement, including later tool updates.
2. Verify encrypted function arguments, multi-item reasoning, long-session compaction and v2 delegation.
3. Implement or explicitly reject search, images, memory and other separate backend routes.
4. Extend interactive coverage to cancellation, long sessions and expired credentials; launcher retries are disabled.
5. Compare behavior on the same tasks and model settings. Transport success does not prove conversational parity or billing entitlement.

No alternative lossless Responses route was established. Devin's `openai` backend uses separately configured provider credentials. Recovered `GetDevstralStream` and completion RPCs did not establish the needed representation and were not called. The [public API](https://docs.devin.ai/api-reference/overview) describes agent/control operations. [devin-2api](https://github.com/leookun/devin-2api/blob/e7ddc55d822283b4774495a9868d181cd7c25fb2/internal/adapter/devin/devin.go#L117) also translates through `GetChatMessage`. This bounded search does not prove that no private route exists.

## Native Devin comparison

With imports and subagents disabled, `agent.codex_tools` changed the captured prompt from 18,448 to 14,416 bytes and tools from 23 to 14. It adds Codex-like persistence, steering and permission guidance. Its grammar-based `apply_patch` is closer, but `exec` remains a JSON shell tool, not Codex JavaScript orchestration. Both prompts still mentioned subagent profiles. This proves a prompt/tool switch, not improved behavior.

Compare direct Astra with this switch first; Fusion adds another model/context. See the [changelog](https://docs.devin.ai/cli/changelog/stable), [configuration](https://docs.devin.ai/cli/reference/configuration/config-file), [import controls](https://docs.devin.ai/cli/reference/configuration/read-config-from), [Fusion](https://docs.devin.ai/cli/fusion) and [enterprise accounting](https://docs.devin.ai/cli/enterprise/devin-auth).

## Schema sources

Recovered complete protobuf descriptors from the Go language server in [official Devin Desktop 3.10.31](https://windsurf-stable.codeiumdata.com/linux-x64/stable/b98cc43128712ba73c60cca73876f58a710aaa27/Devin-linux-x64-3.10.31.tar.gz), linked by the [download page](https://devin.ai/download). Commit `b98cc43128712ba73c60cca73876f58a710aaa27` matches the installed CLI build prefix. These replace the older extension and third-party schemas as the current field reference. They describe the shipped client, not every possible server capability or a supported API contract.

| Descriptor | SHA-256 |
| --- | --- |
| `api_server.proto` | `ccb85afce45c576029b239bd7da3f1df9ec86854e1ca7392221cb00850faaba9` |
| `chat.proto` | `2907de0f002bc457cf84c3514a77108fec458302e89ec76db6d84c4ec3d09aa7` |
| `codeium_common.proto` | `a077eecd046d7e5f7143ac3974a63d175ddc1c500499f117c66ffbf0e542b395` |

`exa.api_server_pb.GetChatMessageRequest` ends at field 27, `prompt_cache_key`, with no raw provider payload. `ChatToolDefinition` ends at strictness field 12, with no namespace. `ChatMessagePrompt` adds videos/documents at 20/21, but no developer role or `additional_tools` item. Completion configuration adds service tier and structured-output fields, but no verbosity or arbitrary provider request. The similarly named `exa.chat_pb.GetChatMessageRequest` is a separate local-editor message.

| Alternative route | Input limitation in current descriptor |
| --- | --- |
| `RawGetChatMessage` | Local language-server RPC, not cloud passthrough. Takes history, system override and model selection. |
| `GetDevstralStream` | History and tools JSON; no model selector or raw Responses body. |
| `GetChatCompletions` / `GetStreamingExternalChatCompletions` | Older completion representations; no Responses items or native tool namespaces. |
| `GetStreamingModelAPITextCompletion` | Allows `override_model_info`, but has no tool declarations or raw Responses body. |

`ModelInfo.inference_config.openai` includes reasoning effort, service tier, extended prompt-cache retention, `reasoning_context` and cache-write control. The tool-capable `GetChatMessage` request has no model-info override. A fresh authenticated `GetCliModelConfigs` read succeeded, but all ten direct Astra aliases omitted `inference_config`, leaving reasoning context undisclosed. Medium advertised 1,000,000 maximum tokens, 128,000 output tokens, and image/tool/thinking/parallel-call support. These are advertised capabilities, not tested limits or proof of Responses parity. The returned generic inference URL did not establish a raw provider endpoint or show that enterprise credentials work there.

Key fields: request prompt 2, history 3, tools 10, model 21; history signature 12, signature type 18, phase 19; tool custom flag 9, grammar 10–11, newer strictness 12; response signature 10, signature type 21, actual-model UID 23, phase 25. Completion field 3 is max newlines, not context length. Strictness numbering differs between schema versions.

The installed CLI is stripped ARM64, not x86. Disassembly located references to the cloud `GetChatMessage` and `GetDevstralStream` routes and translation source metadata. No complete executed namespace conversion was recovered. Namespace strings also belong to other inference backends, so their presence does not prove gateway support. The inspected [opencodex adapter](https://github.com/lidge-jun/opencodex/tree/134c92a01b120162f00c7275189cc47858720379/src/adapters/devin/cloud-direct) rewrites instructions/tools and omits newer state. It is a framing reference, not a parity baseline.
