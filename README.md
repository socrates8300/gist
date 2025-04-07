# 🌟 **Gist CLI**

A modern, AI-powered command-line **and interactive terminal app** for managing text snippets (“gists”):

- save  
- search  
- edit  
- **auto-tag with OpenRouter AI**

all in a fast, minimal local database.

---

## 🚀 Features

- **Store & organize** any text snippet in SQLite
- **Edit** via [Neovim](https://neovim.io) for a smooth workflow
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
# Add a new snippet (opens in Neovim)
gist add

# Update existing by ID
gist update 1

# Search snippets by query text/tags
gist search "rust async traits"

# Quick lookup by ID (shortcut)
gist search 1

# View full snippet content
gist view 1

# List all snippets with preview lines
gist list

# Launch interactive TUI (keyboard help built-in)
gist ui
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

- If unset/unreachable, snippets will be tagged as `"untagged"`
- Tagging runs asynchronously and won’t block your note flow

---

## 💾 Data Storage

- All snippets stored **locally** in an SQLite database
- **Primary key:** standard SQLite auto-increment integer (integer type strictly stores numeric values ensuring data integrity [[neon.tech](https://neon.tech/docs/data-types/integer)], [sqlite.org](https://www.sqlite.org/lang_createtable.html))
- Fast, atomic reads/writes without server setup
- Supports fuzzy search on both **content and tags**

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

## 🔧 Implementation notes

- Rust with performance and safety
- **SQLite** for embedded zero-config storage
- Async **Tokio** + **Reqwest** for non-blocking network
- Native terminal UI via **Ratatui** with Vim-like navigation (arrows+`j/k`)
- Uses **temporary files** with your editor (Neovim default, configurable)
- Precise error handling and safe concurrency
- Supports any API-compatible AI provider with minor changes

---

## 📦 Dependencies

- [clap](https://crates.io/crates/clap) — CLI argument parser
- [rusqlite](https://crates.io/crates/rusqlite) — SQLite DB
- [reqwest](https://crates.io/crates/reqwest) — async HTTP
- [tokio](https://crates.io/crates/tokio) — async runtime
- [serde](https://crates.io/crates/serde) — JSON handling
- [tempfile](https://crates.io/crates/tempfile) — secure temp files
- [ratatui](https://crates.io/crates/ratatui) — terminal UI

---

## 🖥️ Development

### Requirements

- Latest stable **Rust** (>1.56)
- Neovim (or configure to use `vim`)
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

---

## 📝 License

[MIT](./LICENSE)

---

### Enjoy a **fast, smart, developer-oriented snippet manager** – with blazing terminal UI and AI smarts!

