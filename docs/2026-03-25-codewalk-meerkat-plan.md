# CodeWalk → Meerkat Agent: Phased Implementation Plan

**Created:** 2026-03-25
**Status:** Draft
**Scope:** Replace CodeWalk's stateless single-prompt loop with a proper agentic harness using [Meerkat](https://docs.rkat.ai/introduction)

---

## Problem Statement

CodeWalk currently:
- Reads 1–2 files, then asks the model to "explain" them
- Has no ability to navigate — it cannot follow imports, trace call graphs, or prioritize what matters
- Has no memory — large repos silently overflow context
- Has no modes — audit, onboarding, and change review all look the same
- Hardcodes Claude Sonnet regardless of user config

The result is a walkthrough that is superficial by construction, not by model quality.

---

## Target Architecture

```
gist codewalk <path> [--mode audit|onboarding|review|security]
       │
       ▼
┌──────────────────────────┐
│  Phase 1: Recon Agent    │  shell + glob + git log → repo map
│  (lightweight model)     │  structured output: RepoMap
└────────────┬─────────────┘
             │ RepoMap context
             ▼
┌──────────────────────────┐
│  Phase 2: Walk Agent     │  read_file + grep + task tools
│  (configurable model)    │  navigates intelligently, streams steps
└────────────┬─────────────┘
             │ session stored
             ▼
┌──────────────────────────┐
│  Phase 3: Memory Store   │  HNSW semantic index, auto-compaction
│  (meerkat-store)         │  resumable sessions, cross-repo knowledge
└──────────────────────────┘
```

---

## Phases

---

### Phase 0 — Spike: Meerkat Integration Viability

**Goal:** Confirm Meerkat compiles into this binary, talks to OpenRouter, and can run a trivial agent loop. No CodeWalk changes yet.

**Scope:**
- Add `meerkat-core` to `Cargo.toml`
- Write a standalone `src/codewalk/meerkat_spike.rs` (behind a feature flag `meerkat`)
- Implement a minimal `AgentToolDispatcher` with one tool: `read_file`
- Run one agent turn against OpenRouter using the configured model (`z-ai/glm-5-turbo`)
- Print raw output, verify tool call round-trip works

**Definition of Done:**
- [x] `cargo build --features meerkat` succeeds with no errors
- [x] Agent loop executes at least one tool call and returns a response
- [x] Works against OpenRouter endpoint (not hardcoded Anthropic base URL)
- [x] Feature flag off = zero behavior change to existing CodeWalk

**Risk to assess:**
- Meerkat's provider abstraction compatibility with OpenRouter's API surface
- Whether GLM-5-Turbo correctly emits tool-call JSON (some smaller models don't)
- Binary size impact of adding meerkat-core

**Exit gate:** If tool calls don't work with GLM-5-Turbo on OpenRouter, we evaluate a fallback: use Anthropic/OpenAI for the agent loop only, GLM for narrative steps. Decision deferred to end of Phase 0.

---

### Phase 1 — Recon Agent: Build Context Before Walking

**Goal:** Before showing the user anything, run a structured recon pass that produces a `RepoMap`: a prioritized understanding of the codebase.

**Scope:**
- New `src/codewalk/recon.rs` — standalone agent, separate from the walk loop
- Tools available to recon agent:
  - `shell` (allowlist: `git log --oneline`, `git diff --stat`, `find`, `wc -l`)
  - `read_file` (path-scoped to target repo)
  - `glob` (pattern matching for entry points)
- Structured output: `RepoMap` struct
  ```rust
  struct RepoMap {
      entry_points: Vec<String>,     // main.rs, lib.rs, index.ts, etc.
      key_modules: Vec<ModuleSummary>,
      dependency_edges: Vec<(String, String)>,
      recent_changes: Vec<CommitSummary>, // last 20 commits
      estimated_complexity: Complexity,   // Low / Medium / High
      suggested_walk_order: Vec<String>,
  }
  ```
- RepoMap is passed as context to the walk agent (Phase 2)
- TUI shows "Building context..." spinner during recon; no user-visible steps yet

**Definition of Done:**
- [ ] Recon completes in <30s on a repo of ≤50k LOC
- [ ] `RepoMap` parses without error (structured output validated)
- [ ] `suggested_walk_order` contains at least the entry point + 3 key modules
- [ ] Recon agent respects token budget (configurable, default 4k tokens)
- [ ] Existing `--no-meerkat` flag bypasses recon and uses legacy behavior

**Not in scope:** User interaction during recon, custom prompts, multi-repo support.

---

### Phase 2 — Walk Agent: Intelligent Navigation

**Goal:** Replace the current stream-one-prompt loop with an agent that can navigate the codebase during the walk, following what's actually interesting.

**Scope:**
- Refactor `src/codewalk/claude.rs` → `src/codewalk/agent.rs`
  - Keep `claude.rs` as legacy path (feature-flagged off by default once Phase 2 stable)
- Walk agent tools:
  - `read_file` — read any file in scope
  - `grep` — search for symbols/patterns
  - `task_create` / `task_update` — internal step tracking visible in TUI
  - `next_step` — custom tool: signals agent is ready to present next walkthrough step
- Agent produces structured steps (reuses existing `ClaudeStepResponse` type)
- TUI unchanged from user perspective — still n/p navigation, same layout
- Mode flag wired up: `--mode` passed as system prompt variant

**Walk modes (system prompt variants):**
| Mode | Focus |
|------|-------|
| `onboarding` | Architecture, entry points, data flow — "how does this work?" |
| `review` | Recent commits, changed files, what and why — "what changed?" |
| `audit` | Dependencies, error handling, auth paths — "what could go wrong?" |
| `security` | Input validation, secrets, auth, OWASP patterns |

**Definition of Done:**
- [ ] Walk agent reads ≥3 files per step on repos with >5 modules (not just 1–2)
- [ ] `--mode review` produces a walkthrough focused on `git diff` context
- [ ] `--mode audit` surfaces dependency count, error-unhandled paths
- [ ] User can interrupt mid-walk (Ctrl-C) without hang — agent respects cancellation
- [ ] Session log written to `~/.config/gist/sessions/<timestamp>.json`
- [ ] Legacy `--no-meerkat` path still works

---

### Phase 3 — Session Persistence & Memory

**Goal:** Walkthroughs are resumable. Knowledge from past walks accumulates and informs future ones.

**Scope:**
- Add `meerkat-store` crate
- Sessions saved to `~/.config/gist/sessions/` automatically
- `gist codewalk --resume <session-id>` to continue a walk
- `gist codewalk --list-sessions` to see past walks
- Memory enabled (`enable_memory`) — HNSW index of past walkthrough summaries
  - When starting a new walk on a familiar repo, relevant past context surfaces automatically
- Auto-compaction kicks in when session exceeds configurable token threshold

**Definition of Done:**
- [ ] Session interrupted and resumed at correct step
- [ ] Second walk on same repo shows "I've seen this codebase before" context
- [ ] Sessions list shows repo path, date, model, mode, step count
- [ ] Compaction fires without user-visible interruption on sessions >50 turns
- [ ] `gist codewalk --purge-sessions` clears all stored sessions

---

### Phase 4 — Sub-agents & Advanced Modes

**Goal:** Parallel analysis. Specialized agents for heavy-lift use cases (security audit, large monorepos).

**Scope:**
- Recon agent can spawn sub-agents for parallel module analysis (`enable_subagents`)
- New mode: `deep-audit` — spawns one sub-agent per top-level module, results merged
- Budget controls exposed in config:
  ```toml
  [codewalk]
  max_tokens = 100000
  max_tool_calls = 200
  max_wall_seconds = 300
  max_subagents = 4
  ```
- Export improvements: sub-agent findings collated into final report
- Optional: `--output report.md` produces structured audit report

**Definition of Done:**
- [ ] `deep-audit` on a 10-module repo produces per-module summaries
- [ ] Budget exhaustion exits gracefully with partial results, not a panic
- [ ] Concurrent sub-agents respect `max_subagents` limit
- [ ] Report export includes source file references (file:line format)

---

## Cross-Cutting Constraints

These apply to every phase:

1. **No breaking changes to existing CLI surface** — all new behavior behind flags or defaults that preserve current behavior
2. **Config-driven model** — never hardcode a model string after Phase 0
3. **Feature flag discipline** — `--features meerkat` gates all new code until Phase 2 is declared stable, then flipped to default-on
4. **Binary size budget** — track with `cargo bloat` after each phase; alert if >5MB increase
5. **OpenRouter compatibility** — all agent features must work via OpenRouter, not just direct Anthropic/OpenAI

---

## Open Questions (to resolve before/during Phase 0)

1. Does GLM-5-Turbo emit well-formed tool-call JSON? If not, which model is the minimum viable walk agent?
2. Meerkat GitHub activity — last commit date, open issues, maintenance posture. Check before Phase 1.
3. License compatibility — Meerkat license vs. this project's license.
4. Is `meerkat-store` stable enough for Phase 3, or do we need to roll our own session JSON?

---

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-25 | Use Meerkat as agent harness, not build from scratch | Saves harness work; Rust-native; modular opt-in crates |
| 2026-03-25 | Feature-flag all Meerkat code until Phase 2 stable | Preserve existing behavior; allow rollback |
| 2026-03-25 | Recon agent separate from walk agent | Separation of concerns; different tool sets; cheaper model for recon |
| 2026-03-25 | Implement custom `OpenRouterChatClient` wrapping Chat Completions API | Meerkat's built-in `OpenAiClient` uses `/v1/responses`, not `/v1/chat/completions`; OpenRouter only supports Chat Completions; custom `LlmClient` impl bridges the gap |
| 2026-03-25 | Phase 0 complete — GLM-5-Turbo tool calls work on OpenRouter | Live spike confirmed: model emitted well-formed tool-call JSON, `read_file` round-trip succeeded, +1MB binary delta (32→33MB). Proceed to Phase 1. |

---

## What This Is Not

- Not a replacement for the TUI — Meerkat runs behind it, not instead of it
- Not a general-purpose agent framework exposed to users
- Not dependent on Anthropic specifically — must work with any OpenRouter model
