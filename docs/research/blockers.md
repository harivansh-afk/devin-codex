# Exact-harness compatibility blockers

Research date: 2026-09-19. Tested model: `gpt-6-astra-medium` through the authenticated account's existing Devin gateway. These results concern that route and model, not every Cognition deployment. All prompts were synthetic; no repository contents or personal conversation were sent. No configuration was changed.

## Decision

There is still no demonstrated lossless mapping for the stock Codex request. A strict adapter should reject unsupported namespaces, distinct developer messages, and Responses Lite `additional_tools` rather than silently flattening them. The supported flat-tool subset can be implemented and tested independently.

The most important new result is that a nominal system-message source in the gateway schema did **not** retain even a harmless nonce in our controlled test. This is stronger evidence of lost context than an instruction-priority conflict alone. The top-level prompt field and ordinary user-message history did retain the same nonce.

## Live namespace tests

Tools used an ordinary JSON schema with one string property, `marker`. No tool was executed: this tested the returned call identity only.

| Test | Result | Consequence |
|---|---|---|
| One tool named `record`, `server_name=orchard`; ask for `orchard.record` | Text says namespaced tool is unavailable | `server_name` is not demonstrated to create a model-visible namespace. |
| Same declaration; ask for bare `record` | Native tool call named `record`, marker `APPLE` | The underlying flat tool works. |
| Same flat tool without `server_name` | Same native call shape | Baseline succeeds. |
| Two tools both named `record`, server names `orchard` and `quarry`; ask for `orchard.record` | Text says namespaced tool is unavailable | Grouping is not established. |
| Same duplicate names but different descriptions; ask for the ORCHARD description | Native call named only `record`; no namespace identity | The adapter cannot distinguish which duplicate was selected. |
| Literal flat tool name `orchard.record` | Connect `invalid_argument`, no model output | Dot concatenation is not an available workaround on this path. |
| JSON tool property `namespace: orchard` | Native call named only `record` | This candidate property does not establish namespace support. |
| JSON top-level `tool_namespaces` containing `orchard.record` | Text says tool unavailable | This candidate grouping is not effective. |

These results exclude the obvious field-6 and literal-dot shortcuts. They do not prove that no undocumented representation exists. Renaming to `orchard__record` may avoid collisions, but changes the tool name the model sees; it is not an exact mapping and was not adopted.

## Live role tests

Controlled nonce: `LAVENDER_817`.

| Placement of nonce instruction | Following user request | Result |
|---|---|---|
| `ChatMessagePrompt.source=5` | Ask for nonce only | `Unknown` |
| `ChatMessagePrompt.source=1` | Same request | `LAVENDER_817` |
| Top-level `GetChatMessageRequest.prompt` | Same request | `LAVENDER_817` |
| `source=5`, require `LAVENDER_817` | User requires `MARBLE_426` | `MARBLE_426` |
| JSON enum `CHAT_MESSAGE_SOURCE_SYSTEM_PROMPT` | Ask for nonce only | `Unknown` |
| JSON enum candidate `CHAT_MESSAGE_SOURCE_DEVELOPER` | Ask for nonce only | `Unknown` |

The recovered official enum calls 5 `SYSTEM_PROMPT`, 2 `SYSTEM`, and 3 `UNKNOWN`; the assistant uses the historical source-2 lane. A field's name is not proof that the current model adapter preserves it. Source 5 is not suitable for translating Codex developer or system messages based on these tests.

Putting every developer instruction into the top-level prompt would retain its text but merge priorities and positions. Later developer messages can govern later turns without rewriting earlier history; a single static prompt cannot express that structure by simple concatenation. This would be an explicit compatibility mode, not strict parity.

## Connect JSON does not provide schema validation

The same RPC accepts `application/connect+json`: a known request returned `CODEC_OK`. However, an invented top-level field was silently ignored, and an invented enum string was also accepted without preserving its content. Therefore HTTP 200 or a successful completion is **not** evidence that an undocumented field is recognized.

This matters when experimenting with newer protocol fields. Protobuf also normally ignores unknown fields. A candidate must cause a specifically observable behavior and survive a round trip; a successful response alone is insufficient.

## Responses Lite and AdditionalTools

In the exact Codex `rust-v0.155.1` source, `create_tools_json_for_responses_lite` wraps ordinary functions and freeform tools in a real `functions` namespace, preserves an existing namespace description, and keeps other namespaces. This is not just a spelling convention. [Pinned upstream implementation](https://github.com/openai/codex/blob/be2951ea34f0d295ed0becf97079f92fa5f6950e/codex-rs/tools/src/tool_spec.rs#L95).

The client assembles JSON, but does not contain the inference server's final token serialization. Consequently:

- Moving `additional_tools` into top-level tools is not known to preserve the same model-visible representation, role, or insertion position.
- Flattening the `functions` namespace loses its explicit description and grouping even if inner tool names are unique.
- Disabling Lite changes the emitted request. It can be a useful isolated compatibility experiment, but does not establish equivalence to the original harness request.
- Multiple or later `additional_tools` entries cannot safely be treated as a single initial global tool list without demonstrated semantics.

No representation for Responses Lite `additional_tools` was found in the recovered gateway schema or installed Devin binary strings. Absence from strings is weak evidence, not proof that the server lacks it.

## What static inspection does and does not establish

The installed Devin 3000.10.31 binary is a stripped ARM64 ELF. Its internal type strings include `ToolNamespaceDefinition`, `ToolNamespaceEntry`, `namespace`, `defer_loading`, `grammar`, and `grammar_syntax`. It also explicitly supports several inference backends: Windsurf, OpenAI, Anthropic, and mock. Thus namespace types can exist for another backend without the Windsurf gateway preserving them. We did not recover a complete executed namespace-to-protobuf conversion from assembly and do not claim that we did.

The newer generated protocol declarations distributed in `jeopi-catalog` 16.5.1 expose custom-tool grammar fields 9–11 and strictness field 12, but no namespace field in `ChatToolDefinition`, no developer source, and no AdditionalTools message in the recovered request. That package is third-party evidence, not an official current server contract. [Package metadata](https://registry.npmjs.org/jeopi-catalog/16.5.1).

The official older Codeium extension independently confirms the role enum and core protobuf field names. Its older tool schema lacks several later additions, so it does not establish current support for those features. [Official extension distribution](https://open-vsx.org/extension/Codeium/codeium).

## Implementation consequences

1. Fail before network inference when a request requires an unproven mapping. Report the exact field and why it cannot be preserved.
2. Keep native grammar tools intact in the supported flat-tool path; do not turn their input into a JSON wrapper.
3. Preserve phases and signed reasoning replay separately. A visible summary is not interchangeable with continuation state.
4. Reject duplicate flat tool names. A response with only a bare name cannot recover namespace ownership.
5. Keep a per-request audit of field preservation and explicit rejections. Never classify an ignored gateway field as supported merely because inference returned 200.
6. Do not patch Codex to hide these gaps and call the result exact. If a compatibility mode is added later, enumerate its transformations in the interface and report them in tests.

The remaining route to stronger parity is a supported gateway representation or server-side confirmation for developer roles and namespaces, followed by end-to-end tests. Neither a prompt telling the model about namespaces nor a model self-report of its tools proves the original harness representation was retained.

## Alternative raw-inference routes: bounded follow-up

A second pass looked for an existing Responses passthrough under the same entitlement, using installed binary strings, recovered RPC declarations, official documentation, and another adapter's actual source. It made no speculative requests to guessed endpoints and did not send credentials to another host.

| Candidate | Evidence | Assessment |
|---|---|---|
| Devin's `openai` backend | Installed binary explicitly describes `ACP_BACKEND=openai`, `OPENAI_API_BASE`, `OPENAI_API_KEY`, and `USE_COMPLETIONS` for OpenRouter-hosted models. | A client backend for a separately configured provider/key; no evidence that it converts the Devin entitlement into an OpenAI API key. |
| `GetDevstralStream` | Present in installed binary and recovered schema. Request has metadata, `ChatMessagePrompt` history, and `tools_json`; no model-selection field in that schema. | Another model-specific RPC, not a demonstrated raw Astra Responses transport. The same restricted message type remains. Not called. |
| `GetStreamingModelAPITextCompletion` | Recovered request has model enum, system prompt, the same chat-message type, completion settings, and optional model-info override. | Not a Responses passthrough; recovered request lacks the needed namespace/AdditionalTools surface. Not called. |
| `GetChatCompletions` / `GetStreamingExternalChatCompletions` | Recovered RPC declarations use chat/completion-specific protobuf types. | Their names do not establish OpenAI protocol compatibility or preservation of current Responses semantics. Not called. |
| Public Devin REST API | Official overview describes agent sessions, organization resources, and enterprise management; the inspected API documentation index has no raw inference/Responses entry. | A supported agent/control API, not evidence of a model-only transport. [Official overview](https://docs.devin.ai/api-reference/overview). |
| `devin-2api` | Its README advertises a Responses endpoint, but its implementation directly calls `ApiServerService/GetChatMessage`. | Another translation adapter, not discovery of a separate passthrough route. [Pinned implementation](https://github.com/leookun/devin-2api/blob/e7ddc55d822283b4774495a9868d181cd7c25fb2/internal/adapter/devin/devin.go#L117). |

The installed gateway code contains `GetChatMessage`, `GetDevstralStream`, model discovery and assignment methods, but no identifiable `/v1/responses` passthrough string. This negative static result does not prove that no private service exists.

**No usable alternative raw Responses route was established under the existing Devin authentication.** The documented enterprise CLI login explains how its access and billing are inherited; it does not document a generic OpenAI-compatible inference entitlement. [Enterprise authentication](https://docs.devin.ai/cli/enterprise/devin-auth).

A Codex upstream patch could change request transport, expose missing client configuration, or deliberately emit a reduced compatibility request. It cannot make a gateway retain message roles or namespace identities that the current demonstrated path loses. With present evidence, exact stock-harness parity requires either discovering and verifying an already-existing lossless gateway route, or Cognition providing the missing gateway capability. The bounded search establishes neither that an undocumented route exists nor that none exists.

## Reproducibility

Seventeen bounded probes were run for this note. They covered two independent protobuf nonce baselines, source-5 tests, namespace identities/collisions, and the JSON codec controls above. Results were inspected for native tool-call names and trailers, not only generated prose. Private local probe files are deliberately excluded from the repository because their helper reads the authenticated CLI capture. Only synthetic prompts and summarized outcomes are recorded here.
