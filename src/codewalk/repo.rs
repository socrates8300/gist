use ignore::WalkBuilder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// Repository index built from the file tree
pub struct RepoIndex {
    pub tree: String,
    pub languages: Vec<(String, usize)>,
    pub file_count: usize,
    pub root: PathBuf,
    file_cache: HashMap<String, String>,
}

impl RepoIndex {
    /// Build a file tree index of the repository (structure only, no file contents)
    pub fn build(repo_path: &Path) -> io::Result<Self> {
        let repo_path = repo_path.canonicalize()?;
        let mut tree_lines = Vec::new();
        let mut lang_counts: HashMap<String, usize> = HashMap::new();
        let mut file_count: usize = 0;

        let walker = WalkBuilder::new(&repo_path)
            .hidden(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .max_depth(Some(12))
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();

            // Skip the root itself
            if path == repo_path {
                continue;
            }

            let relative = match path.strip_prefix(&repo_path) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let depth = relative.components().count();
            let indent = "  ".repeat(depth.saturating_sub(1));
            let name = relative
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                tree_lines.push(format!("{}{}/", indent, name));
            } else {
                tree_lines.push(format!("{}{}", indent, name));
                file_count += 1;

                // Detect language by extension
                if let Some(ext) = path.extension() {
                    let lang = ext_to_language(ext.to_str().unwrap_or(""));
                    if !lang.is_empty() {
                        *lang_counts.entry(lang.to_string()).or_insert(0) += 1;
                    }
                }
            }

            // Cap tree size to avoid blowing up context
            if tree_lines.len() >= 2000 {
                tree_lines.push("  ... (truncated)".to_string());
                break;
            }
        }

        let mut languages: Vec<(String, usize)> = lang_counts.into_iter().collect();
        languages.sort_by(|a, b| b.1.cmp(&a.1));

        Ok(RepoIndex {
            tree: tree_lines.join("\n"),
            languages,
            file_count,
            root: repo_path,
            file_cache: HashMap::new(),
        })
    }

    /// Read a file on demand, caching the result
    pub fn read_file(&mut self, relative_path: &str) -> io::Result<String> {
        if let Some(cached) = self.file_cache.get(relative_path) {
            return Ok(cached.clone());
        }

        let full_path = self.root.join(relative_path);
        let content = fs::read_to_string(&full_path)?;
        self.file_cache
            .insert(relative_path.to_string(), content.clone());
        Ok(content)
    }

    /// Format a summary string for Claude's initial context
    pub fn summary(&self) -> String {
        let lang_summary: Vec<String> = self
            .languages
            .iter()
            .take(10)
            .map(|(lang, count)| format!("  {} ({} files)", lang, count))
            .collect();

        format!(
            "Repository: {}\nFiles: {}\nLanguages:\n{}\n\nFile tree:\n{}",
            self.root.display(),
            self.file_count,
            lang_summary.join("\n"),
            self.tree
        )
    }
}

fn ext_to_language(ext: &str) -> &str {
    match ext {
        "rs" => "Rust",
        "py" => "Python",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "tsx" => "TypeScript (React)",
        "jsx" => "JavaScript (React)",
        "go" => "Go",
        "java" => "Java",
        "kt" => "Kotlin",
        "swift" => "Swift",
        "c" => "C",
        "cpp" | "cc" | "cxx" => "C++",
        "h" | "hpp" => "C/C++ Header",
        "rb" => "Ruby",
        "php" => "PHP",
        "cs" => "C#",
        "ex" | "exs" => "Elixir",
        "hs" => "Haskell",
        "ml" | "mli" => "OCaml",
        "scala" => "Scala",
        "lua" => "Lua",
        "sh" | "bash" | "zsh" => "Shell",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" => "CSS",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" => "XML",
        "sql" => "SQL",
        "md" | "markdown" => "Markdown",
        "proto" => "Protobuf",
        "zig" => "Zig",
        "nim" => "Nim",
        "dart" => "Dart",
        "r" | "R" => "R",
        "jl" => "Julia",
        _ => "",
    }
}
