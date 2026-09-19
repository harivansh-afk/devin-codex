# Codex translation contract

Investigated 2026-09-19 against installed `codex-cli 0.155.1`, official release tag `rust-v0.155.1` (commit `be2951ea34f0d295ed0becf97079f92fa5f6950e`), and private loopback captures. No OpenAI inference was performed. This document contains no credentials or full personal prompts.

## Preserve the upstream representation

Codex's current Astra catalog enables Responses Lite, code-mode-only tools and v2 delegation. The installed client sends a developer `additional_tools` item containing namespaces, followed by developer instruction messages. It does not send ordinary top-level `tools` or `instructions`.

The exact release's [tool serializer](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/tools/src/tool_spec.rs), lines 95–140, wraps flat function and freeform specifications in an explicit `functions` namespace, merges an existing `functions` namespace while preserving its description, and leaves other namespaces intact. The [client](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/core/src/client.rs), lines 790–844, places that serialized JSON inside `additional_tools` and adds the base instructions as a developer message.

This source exposes JSON assembly, not the inference server's tokenizer or internal namespace rendering. It does not prove any of the following transformations equivalent:

- `functions.exec` namespace/custom tool → flat custom `exec`.
- Namespace `orchard`, function `record` → flat name `orchard.record`.
- Namespace descriptions → prefixes inside each tool description.
- Developer `additional_tools` → gateway top-level tool definitions.
- Separate ordered developer messages → one concatenated system prompt.

Even when a flat tool can reproduce the action, the model-visible tool organization, output recipient and message roles may differ. Mapping returned call IDs and names is necessary plumbing but does not prove prompt equivalence. The gateway's `server_name` field must not be treated as a native model namespace without a positive test.

Changing `use_responses_lite`, tool mode, multi-agent version or catalog metadata to fit gateway limitations would define a reduced-fidelity mode. It must not be the parity default. The current adapter should reject an unresolved namespace or developer-tool declaration instead of silently flattening it.

## No-patch transport versus exact parity

Supported provider configuration permits either a separate custom provider or the built-in OpenAI provider with `openai_base_url` redirected to loopback. Both were captured from the installed binary using an exact exported model catalog and the same effective user configuration.

| Surface | Custom provider | Built-in OpenAI directed to loopback |
| --- | --- | --- |
| Lite developer item framing | Preserved | Preserved |
| Message text/content | Same in measured first request | Same |
| Internal message metadata | Removed | Preserved |
| `encrypted_function_args` | Removed by client source | Preserved by client source |
| Remote compaction | Unsupported capability | V2 capability |
| Standalone search | Requires `supports_standalone_web_search = true` | Exposed by provider identity |
| Existing OpenAI auth | Not required by provider | Sent to loopback unless replaced |

Release [provider capabilities](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider/src/provider.rs), lines 353–365, select V2 compaction for OpenAI/Azure. This uses `compaction_trigger` in Responses, not merely the older `/responses/compact` endpoint. [Web search availability](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/ext/web-search/src/extension.rs), lines 43–49, already includes the explicit custom-provider search flag. No upstream patch is required merely to enable that tool.

The built-in path is the strongest no-patch starting point because it avoids client-side metadata and encrypted-argument removal. The adapter still must implement or explicitly reject every resulting capability. The [provider route check](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider-info/src/lib.rs), lines 530–537, additionally recognizes backend routes only when the base URL is absent or ends in `/backend-api/codex`. A loopback `/v1` base does not retain every backend-only route.

## Measured first-request differences

Matched custom versus built-in loopback captures had identical six message `content` arrays and generation controls. The built-in request added internal metadata to all six messages. The complete tool arrays differed in exactly one field: the `functions.exec` description gained 236 lines, including nested web-search definitions. The description sizes were 12889 versus 25315 UTF-8 bytes; these are bytes, not token counts. Counting the three outer namespaces would have missed this change.

Neither capture proves subsequent turn, encrypted-state, compaction or output-decoding parity. Those require separate tests.

## Dummy local authentication: tested boundary

For noninteractive `codex exec`, a per-process `CODEX_API_KEY` value overrides persisted auth. This was verified using a nonsecret placeholder and a loopback mock:

```sh
CODEX_API_KEY=local-research-placeholder codex exec --ephemeral \
  -c 'model_provider="openai"' \
  -c 'openai_base_url="http://127.0.0.1:48763/v1"' \
  -c 'model_catalog_json="/absolute/path/to/pinned-models.json"' \
  'Your task'
```

This is a launch sketch for an already-running authorized local adapter, not a claim that this repository supports the full resulting request. The actual test used a private ephemeral port, captured the request and immediately returned HTTP 400. It performed no generation and changed no live configuration.

The listener verified that the Authorization header matched only the placeholder and that no ChatGPT account header was present. It did not log or persist authorization values. Message content, internal metadata and generation controls matched the ChatGPT-authenticated built-in capture.

However, API authentication removed the image-generation tool's 26-line definition from the code-mode tool description. Therefore changing authentication mode itself changes the tool surface. This is an observed parity gap, not speculation. Keeping a tool definition while lacking the corresponding backend would be a different failure, so the adapter must not advertise it dishonestly.

The CLI's interactive TUI does not necessarily honor this environment shortcut: source initialization sets `enable_codex_api_key_env=false` for TUI and true for exec. The dummy-token launcher is verified only for `codex exec`. An interactive profile/auth arrangement remains to be tested; do not claim the command is a ready interactive launcher.

If retaining ChatGPT auth to preserve current client behavior, credentials must terminate at the local adapter. Construct Cognition auth independently and never forward inbound Authorization or ChatGPT account headers. This is an implementation boundary, not permission to access a different account or backend.

## Required evidence before claiming equivalent translation

1. A native gateway representation of namespace names, descriptions, ordering, nested custom grammars and returned namespace identity, or an upstream-supported equivalent whose rendering is documented.
2. Preservation of ordered developer instructions and developer tool declarations, including role/priority, rather than merely placing their text somewhere in the request.
3. Lossless reasoning-state and encrypted-function-argument replay, separate from visible reasoning summaries.
4. Commentary/final phases, tool result identities, malformed-stream handling, cancellation and multi-call continuation.
5. Long-session compaction and v2 delegation behavior.

A successful plain or custom tool round-trip proves one protocol operation. It does not remove unresolved namespace/role semantics. There is no source-supported configuration that makes the namespaced Lite input equivalent to a flat gateway schema while preserving all current catalog behavior.
