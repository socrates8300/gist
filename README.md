# 🌟 **Gist CLI**

A modern, AI-powered command-line **and interactive terminal app** for managing text snippets ("gists"):

- save  
- search  
- edit  
- **auto-tag with OpenRouter AI**

all in a fast, minimal local database.

---

## 🚀 Features

- **Store & organize** any text snippet in SQLite
- **Edit** with your preferred editor (Neovim, Vim, or any editor of choice)
- **Rich terminal UI:**  
  - browse  
  - Vim & arrow key navigation  
  - inline content view  
  - live **fuzzy search** filter
- **AI-powered tagging** using [OpenRouter](https://openrouter.ai)
- **Search by text or tags**
- View by ID or list with previews
- Fully local first, fast, no cloud lock-in
- Works standalone or as your snippet vault with optional AI
- **Import/Export** functionality for backup and migration
- **Configuration** system for customizing behavior

---

## 🛠️ Installation

```bash
git clone https://github.com/yourusername/gist-cli.git
cd gist-cli

cargo build --release
cargo install --path .
```

Then you can run the CLI commands or launch the TUI:

```bash
gist ui
```

---

## ⚡ Usage

```bash
# Add a new snippet (opens in your configured editor)
gist add

# Add a snippet with specified tags
gist add --tags "rust,code,example"

# Add a snippet from a file
gist add --file /path/to/code.rs

# Update existing by ID
gist update 1

# Update tags for a snippet
gist update 1 --tags "new,tags"

# Search snippets by query text/tags
gist search "rust async traits"

# Search only in tags
gist search "rust" --tags-only

# View full snippet content
gist view 1

# Delete a snippet
gist delete 1

# List all snippets with preview lines
gist list

# List with custom sorting and limit
gist list --sort-by tags --limit 10

# Launch interactive TUI (keyboard help built-in)
gist ui

# Export all snippets to a file
gist export --output snippets.json

# Import snippets from file
gist import --input snippets.json

# Configure application settings
gist config --editor "code -w" --theme dark --auto-tags true

# Optimize database
gist optimize
```

---

## 🤖 AI Tagging Setup

**Optional but recommended** for generating meaningful tags:

1. Sign up at [OpenRouter.ai](https://openrouter.ai)  
2. Generate your API key  
3. Set environment variable:

```bash
export OPENROUTER_API_KEY=your_api_key_here
```

Or configure through the CLI:

```bash
gist config --api-key "your_api_key_here"
```

- If unset/unreachable, snippets will use intelligent fallback tagging
- Tagging runs asynchronously and won't block your note flow
- Tag generation can be disabled with `gist config --auto-tags false`

---

## 💾 Data Storage

- All snippets stored **locally** in an SQLite database
- **Primary key:** standard SQLite auto-increment integer (integer type strictly stores numeric values ensuring data integrity [[neon.tech](https://neon.tech/docs/data-types/integer)], [sqlite.org](https://www.sqlite.org/lang_createtable.html))
- Fast, atomic reads/writes without server setup
- Supports fuzzy search on both **content and tags**
- Database optimization commands available

### Example DB schema:

```sql
CREATE TABLE IF NOT EXISTS gists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content    TEXT NOT NULL,
    tags       TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

---

## 📱 Terminal UI

The interactive TUI mode offers a powerful interface for managing your snippets:

- Two-panel layout with list and content views
- Vim-style navigation (j/k) and arrow keys
- Tab key to switch between panels
- Live search filtering
- Tag editing directly in the UI
- Clipboard integration
- Full keyboard shortcut help screen (press ?)

### Keyboard Shortcuts:

```
Navigation:
  ↑/↓, j/k     - Move selection up/down
  PgUp/PgDown  - Move by page
  Home/End     - Jump to start/end
  Tab          - Switch between list and content panels

Actions:
  a            - Add new snippet
  e            - Edit selected snippet
  d            - Delete selected snippet (with confirmation)
  y            - Copy snippet content to clipboard
  t            - Edit tags for the selected snippet
  r            - Refresh snippet list

Search:
  s, /         - Start search mode
  Esc          - Exit search/help mode or cancel action
  Enter        - Execute search

UI:
  ?            - Toggle help screen
  q            - Quit (with confirmation if changes)
```

---

## ⚙️ Configuration

Customize your Gist CLI experience with the configuration system:

```bash
# View current configuration
gist config --show

# Set preferred editor
gist config --editor "code -w"

# Choose theme
gist config --theme light  # Options: dark, light, system

# Enable/disable auto-tag generation
gist config --auto-tags false

# Set API key for tag generation
gist config --api-key "your_api_key"
```

The configuration is stored in `~/.config/gist/config.json` and is loaded automatically when the application starts.

---

## 🔧 Implementation notes

- Rust with performance and safety
- **SQLite** for embedded zero-config storage
- Async **Tokio** + **Reqwest** for non-blocking network
- Native terminal UI via **Ratatui** with Vim-like navigation (arrows+`j/k`)
- Thread-based operations for responsive UI
- Uses **temporary files** with your editor (configurable)
- Precise error handling and safe concurrency
- Supports any API-compatible AI provider with minor changes
- Cross-platform design for Windows, macOS, and Linux

---

## 📦 Dependencies

- [clap](https://crates.io/crates/clap) — CLI argument parser
- [rusqlite](https://crates.io/crates/rusqlite) — SQLite DB
- [reqwest](https://crates.io/crates/reqwest) — async HTTP
- [tokio](https://crates.io/crates/tokio) — async runtime
- [serde](https://crates.io/crates/serde) — JSON handling
- [tempfile](https://crates.io/crates/tempfile) — secure temp files
- [ratatui](https://crates.io/crates/ratatui) — terminal UI
- [crossterm](https://crates.io/crates/crossterm) — terminal control
- [clipboard](https://crates.io/crates/clipboard) — clipboard access
- [colored](https://crates.io/crates/colored) — terminal colors
- [chrono](https://crates.io/crates/chrono) — date/time handling
- [dirs](https://crates.io/crates/dirs) — standard directories

---

## 🖥️ Development

### Requirements

- Latest stable **Rust** (>1.56)
- SQLite3 installed
- Optional: OpenRouter API key for tagging

### Build steps

```bash
cargo build --release
```

**Run CLI:**

```bash
target/release/gist ...commands...
```

---

## 🤝 Contributing

Contributions, feature ideas, and PRs **welcome!**

Please:

- Open issues for bugs
- Fork and PR on GitHub
- Write clear commit messages
- Follow Rust formatting (`cargo fmt`)
- Add tests for new features
- Update documentation for changes

---

## 📝 License

[MIT](./LICENSE)

---

### Enjoy a **fast, smart, developer-oriented snippet manager** – with blazing terminal UI and AI smarts!
