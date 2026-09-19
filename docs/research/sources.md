# Sources and protocol notes

Research snapshot: September 19, 2026. Versioned source is authoritative only for the inspected version; current documentation can change.

## Codex

- [Advanced configuration](https://developers.openai.com/codex/config-advanced) and [configuration reference](https://developers.openai.com/codex/config-reference).
- 0.155.1 release: [provider configuration](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider-info/src/lib.rs), [capabilities](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/model-provider/src/provider.rs), [client](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/core/src/client.rs), and [web search](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/ext/web-search/src/extension.rs).

The inspected Astra catalog selects Responses Lite, code mode only, multi-agent v2, unified exec, freeform patches, and text/image web search. Preserve the full resolved catalog, including instruction templates; a hand-written subset or unknown model alias can change the harness before translation.

## Devin

- [Stable changelog](https://docs.devin.ai/cli/changelog/stable), [model selection](https://docs.devin.ai/cli/models), and [configuration](https://docs.devin.ai/cli/reference/configuration/config-file).
- [Import controls](https://docs.devin.ai/cli/reference/configuration/read-config-from), [rules](https://docs.devin.ai/cli/extensibility/rules), [commands](https://docs.devin.ai/cli/reference/commands), and [enterprise authentication/accounting](https://docs.devin.ai/cli/enterprise/devin-auth).
- [Fusion configuration](https://docs.devin.ai/cli/fusion) and [vendor announcement](https://cognition.com/blog/local-fusion).

## Recovered gateway schema

The [official Codeium extension 1.48.2](https://open-vsx.org/extension/Codeium/codeium) contains older generated protobuf descriptors. Newer fields were identified in [third-party jeopi-catalog 16.5.1](https://registry.npmjs.org/jeopi-catalog/16.5.1) and validated selectively against captured requests. This is not a published, supported current Cognition API contract.

| Structure | Candidate fields |
| --- | --- |
| Request | metadata 1; prompt 2; history 3; request type 7; configuration 8; tools 10; parallel-call disabling 11; tool choice 12; model UID 21 |
| History message | source 2; text 3; calls 6; result call ID 7; images 10; thinking 11; signature 12; signature type 18; phase 19 |
| Tool | name 1; description 2; schema 3; custom flag 9; grammar 10; grammar syntax 11; strict 12 in newer schema |
| Response | text delta 3; tool deltas 6; usage 7; thinking 9; signature 10; signature type 21; actual-model UID 23; phase 25 |

Recovered roles are USER=1, SYSTEM=2, UNKNOWN=3, TOOL=4, SYSTEM_PROMPT=5. Historical SYSTEM=2 serves as assistant in the inspected adapter. No distinct developer role or namespace representation is established. Field numbers must be versioned and tested, not treated as a stable API.

Completion configuration field 2 is max tokens; field 3 is max newlines, not context length. Strict-tool field numbering differs between schema versions. Preserve unknown fields only when their transport meaning is established; never silently reinterpret them.

The inspected [existing adapter](https://github.com/lidge-jun/opencodex/tree/134c92a01b120162f00c7275189cc47858720379/src/adapters/devin/cloud-direct) collapses system text into user messages, rewrites/truncates tool descriptions, and omits newer state. Its source is evidence of its implementation, not proof of gateway requirements. The local probes accepted original descriptions it described as problematic. Use it as a framing reference, not the fidelity baseline.
