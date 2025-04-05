# Gist CLI

A powerful command-line tool for storing, searching, updating, and retrieving text snippets (gists) with AI-powered automatic tagging.

## Features

- Store text snippets in a local SQLite database
- Edit gists using Neovim
- Automatically generate tags using OpenRouter AI
- Search for gists by content or tags
- Quick ID lookup via search command
- List all gists with previews
- View complete gist content

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/gist-cli.git
cd gist-cli

# Build and install
cargo build --release
cargo install --path .
```

## Usage

```bash
# Add a new gist (opens nvim)
gist add

# Update an existing gist by ID
gist update 1

# Search gists by query
gist search "rust async"

# Quick lookup by ID (shorthand for view)
gist search 1

# View a gist by ID
gist view 1

# List all gists
gist list
```

## API Configuration

This application uses the OpenRouter API for generating tags. To use this feature:

1. Get an API key from [OpenRouter](https://openrouter.ai)
2. Set it as an environment variable:

```bash
export OPENROUTER_API_KEY=your_api_key_here
```

If the API key is not set or if there are connectivity issues, gists will be tagged as "untagged".

## Technical Implementation

The application is built with a focus on performance, memory safety, and robust error handling:

- **SQLite Storage**: Persistent, local storage of all gists with metadata
- **Rust Concurrency**: Uses Tokio for asynchronous API requests
- **Error Propagation**: Comprehensive error handling that provides informative feedback
- **Temporary Files**: Secure file handling for editor integration
- **JSON Processing**: Type-safe serialization/deserialization for API communication

## Dependencies

- `clap`: Command-line argument parsing
- `rusqlite`: SQLite database operations
- `reqwest`: HTTP client for API requests
- `tokio`: Asynchronous runtime
- `serde`: JSON serialization/deserialization
- `tempfile`: Secure temporary file handling

## Database Schema

```sql
CREATE TABLE IF NOT EXISTS gists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content    TEXT NOT NULL,
    tags       TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
)
```

## Development

### Prerequisites

- Rust toolchain (1.56.0 or later)
- Neovim text editor
- SQLite

### Building from Source

```bash
cargo build --release
```

The binary will be available at `target/release/gist`.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT License
