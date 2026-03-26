# Gist CLI

A fast, local-first code snippet manager with an interactive terminal UI, AI-powered tagging, and an AI-driven repository walkthrough tool (CodeWalk).

---

## Features

- **Store & organize** any text snippet in a local SQLite database
- **Syntax-highlighted** content view (Rust, Python, JS, SQL, Bash, and more)
- **Interactive TUI** — two-panel layout, Vim-style navigation, live fuzzy search
- **AI tagging** — auto-generates tags via OpenRouter (or any OpenAI-compatible API)
- **Import/Export** for backup and migration
- **CodeWalk** — AI-powered repository walkthrough with five focus modes, session persistence, and a parallel deep-audit engine

---

## Installation

### Requirements

- Rust stable (1.70+)
- SQLite3
- Optional: OpenRouter API key for AI features

### Build

```bash
git clone https://github.com/socrates8300/gist.git
cd gist

# Standard build
cargo build --release

# With Meerkat agent support (required for CodeWalk recon + deep-audit)
cargo build --release --features meerkat

cargo install --path .
```

---

## Snippet Management

### Add

```bash
# Open editor to write a new snippet
gist add

# With explicit tags
gist add --tags "rust,async,example"

# From an existing file
gist add --file /path/to/code.rs
```

If `--tags` is omitted, tags are generated automatically by the AI model.

### View and search

```bash
# View a snippet by ID (syntax-highlighted)
gist view 1

# Search across content and tags
gist search "async trait"

# Search tags only
gist search "rust" --tags-only

# List all snippets (default: 20, sorted by creation time)
gist list

# Custom limit and sort order (options: created, id, tags)
gist list --limit 50 --sort-by tags
```

### Edit and delete

```bash
# Edit content and regenerate tags
gist update 1

# Update tags only
gist update 1 --tags "new,tags"

# Delete with confirmation prompt
gist delete 1

# Delete without prompt
gist delete 1 --force
```

### Import / Export

```bash
# Export all snippets to JSON
gist export --output snippets.json

# Import from JSON
gist import --input snippets.json
```

### Database maintenance

```bash
gist optimize
```

---

## Interactive TUI

```bash
gist ui
```

Two-panel layout: snippet list on the left, full content on the right.

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `↑` / `↓`, `j` / `k` | Move selection |
| `PgUp` / `PgDn` | Page up/down |
| `Home` / `End` | Jump to first/last |
| `Tab` | Switch between panels |
| `a` | Add new snippet |
| `e` | Edit selected snippet |
| `d` | Delete selected snippet |
| `y` | Copy content to clipboard |
| `t` | Edit tags |
| `r` | Refresh list |
| `s`, `/` | Enter search mode |
| `Enter` | Execute search |
| `Esc` | Exit search/help/cancel |
| `?` | Toggle help screen |
| `q` | Quit |

---

## CodeWalk — AI Repository Walkthrough

CodeWalk guides an AI agent through a codebase, producing a structured walkthrough you navigate step by step in the TUI.

```bash
gist codewalk [OPTIONS] [PATH]
```

`PATH` defaults to `.` (current directory).

### Walk modes

| Mode | Flag | Focus |
|------|------|-------|
| Onboarding | `--mode onboarding` | Architecture, entry points, data flow — "How does this work?" |
| Review | `--mode review` | Recent changes, commit context — "What changed?" |
| Audit | `--mode audit` | Tech debt, error handling, code quality — "What could fail?" |
| Security | `--mode security` | Input validation, auth, secrets, OWASP patterns — "What are the risks?" |
| Deep Audit | `--mode deep-audit` | Parallel per-module sub-agents with budget controls — structured audit report |

### Common flags

```bash
# Specify what to explore
gist codewalk --scope "Trace the auth flow" --mode security .

# Choose a model (overrides config)
gist codewalk --model "openai/gpt-4o" .

# Export session to Markdown when done
gist codewalk --output report.md .

# Custom system prompt file
gist codewalk --prompt my-prompt.txt .

# Load pre-existing tech debt notes
gist codewalk --notes debt.md .
```

### Session management

Sessions are saved automatically to `~/.config/gist/sessions/` after each step.

```bash
# List saved sessions
gist codewalk --list-sessions

# Resume a previous session by ID
gist codewalk --resume <session-id>

# Delete all saved sessions
gist codewalk --purge-sessions
```

### Deep Audit mode

`--mode deep-audit` runs a parallel sub-agent audit across all key modules identified by the recon agent. Each sub-agent gets its own budget slice and produces findings, risks, and `file:line` references. Results are pre-loaded into the TUI before it starts.

Budget limits (configurable in `~/.config/gist/config.toml`):

| Setting | Default | Description |
|---------|---------|-------------|
| `max_tokens` | 100,000 | Total token budget across all sub-agents |
| `max_tool_calls` | 200 | Maximum tool calls across all sub-agents |
| `max_wall_seconds` | 300 | Wall-clock time limit (seconds) |
| `max_subagents` | 4 | Maximum concurrent sub-agents |

The `--output` flag in deep-audit mode produces a structured Markdown report with:
- Executive summary (module count, finding count, risk count)
- Per-module findings, risks, and file references
- Consolidated risk register
- All file:line references
- Git HEAD SHA and elapsed time

### Meerkat agent flags (requires `--features meerkat`)

```bash
# Skip the recon agent (use legacy single-prompt behavior)
gist codewalk --no-meerkat .

# Run the raw Meerkat integration spike (diagnostic)
gist codewalk --meerkat-spike .
```

### CodeWalk TUI controls

| Key | Action |
|-----|--------|
| `n`, `→` | Next step |
| `p`, `←` | Previous step |
| `Tab` | Switch focus between code panel and explanation panel |
| `j` / `k`, `↑` / `↓` | Scroll focused panel |
| `Ctrl-d` / `Ctrl-u` | Half-page down/up in focused panel |
| `J` / `K` | Scroll code panel (regardless of focus) |
| `d` | Trigger a deep dive on a suggested topic |
| `t` | Add a tech debt note for the current file |
| `T` | Toggle tech debt notes panel |
| `e` | Export session to Markdown |
| `/` | Search within current file |
| `?` | Toggle help screen |
| `q` | Quit CodeWalk |

---

## Configuration

Config is stored at `~/.config/gist/config.toml` and created automatically on first run.

```bash
# View current config
gist config --show

# Set preferred editor
gist config --editor "code -w"

# Set theme (dark | light | system)
gist config --theme dark

# Enable/disable AI auto-tagging
gist config --auto-tags false

# Set OpenRouter API key
gist config --api-key "sk-or-..."

# Set AI model for tagging
gist config --ai-model "openai/gpt-4o-mini"

# Set API base URL (defaults to OpenRouter)
gist config --ai-base-url "https://openrouter.ai/api/v1"
```

### Full config.toml reference

```toml
editor = ""                          # empty = auto-detect (nvim > vim > nano)
default_tags = ["snippet"]
theme = "Dark"                       # Dark | Light | System
auto_generate_tags = true
tag_api_key = "sk-or-..."           # OpenRouter key
ai_model = "glm-5-turbo"
ai_base_url = "https://api.z.ai/api/coding/paas/v4"
anthropic_api_key = ""              # Optional: direct Anthropic key for CodeWalk

[codewalk]
enable_memory = true
compaction_threshold = 50           # Conversation turns before compaction
session_retention_days = 30
max_tokens = 100000                 # Deep-audit token budget
max_tool_calls = 200                # Deep-audit tool call budget
max_wall_seconds = 300              # Deep-audit time limit
max_subagents = 4                   # Deep-audit concurrency
```

---

## AI Setup

CodeWalk and auto-tagging use [z.ai](https://z.ai) by default (GLM Coding Plan).

1. Sign up at [z.ai](https://z.ai) and generate an API key from the [API Keys page](https://z.ai/manage-apikey/apikey-list)
2. Set it via config:
   ```bash
   gist config --api-key "your-z.ai-key"
   ```
3. The defaults are already set correctly — no other configuration needed:
   ```bash
   gist config --ai-model "glm-5-turbo"
   gist config --ai-base-url "https://api.z.ai/api/coding/paas/v4"
   ```

Any OpenAI-compatible provider also works — just update `--ai-base-url` and `--ai-model` accordingly.

If no key is set, tagging falls back to default tags. CodeWalk requires an API key.

---

## Data Storage

| Path | Contents |
|------|----------|
| `~/.config/gist/gists.db` | SQLite snippet database |
| `~/.config/gist/config.toml` | Application configuration |
| `~/.config/gist/sessions/` | CodeWalk session files (JSON) |

The database schema:

```sql
CREATE TABLE IF NOT EXISTS gists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content    TEXT NOT NULL,
    tags       TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

---

## Tech Stack

| Library | Purpose |
|---------|---------|
| [clap](https://crates.io/crates/clap) | CLI argument parsing |
| [rusqlite](https://crates.io/crates/rusqlite) | SQLite storage |
| [ratatui](https://crates.io/crates/ratatui) + [crossterm](https://crates.io/crates/crossterm) | Terminal UI |
| [tokio](https://crates.io/crates/tokio) | Async runtime |
| [reqwest](https://crates.io/crates/reqwest) | HTTP client |
| [serde](https://crates.io/crates/serde) + [toml](https://crates.io/crates/toml) | Config serialization |
| [syntect](https://crates.io/crates/syntect) | Syntax highlighting |
| [chrono](https://crates.io/crates/chrono) | Timestamps |
| [dirs](https://crates.io/crates/dirs) | Standard config paths |
| [tempfile](https://crates.io/crates/tempfile) | Editor temp files |
| [meerkat](https://crates.io/crates/meerkat) *(optional)* | Agent framework for CodeWalk recon + deep-audit |
| [glob](https://crates.io/crates/glob) *(optional)* | File pattern matching for recon agent |

---

## Development

```bash
# Run tests (base)
cargo test

# Run tests with Meerkat agent support
cargo test --features meerkat

# Format
cargo fmt

# Lint
cargo clippy
```

---

## Contributing

- Open issues for bugs or feature requests
- Fork and submit PRs
- Follow `cargo fmt` formatting
- Add tests for new features
- Update this README for user-visible changes

---

## License

[MIT](./LICENSE)
