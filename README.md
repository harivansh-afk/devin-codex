# devin-codex

Use the Codex TUI with Devin inference.

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
