//! Strata's code-block language ids, as its document layout normalises them (sourceview's
//! language ids: `python3`, `c-sharp`, `typescript-jsx`…), to the tokens C10a's grammar
//! lookup takes (a file extension or a grammar name). Anything else passes through, so a
//! fence's own token (`rs`, `bash`) works too. Every id Strata gives finds a grammar except
//! `vala`, which two-face's set lacks: those blocks draw plain.

/// The grammar token for `id`.
pub fn grammar_token(id: &str) -> &str {
    match id {
        "c-sharp" => "cs",
        "docker" => "Dockerfile",
        "js" => "js",
        "kotlin" => "kt",
        "latex" => "tex",
        "makefile" => "Makefile",
        "markdown" => "md",
        "powershell" => "ps1",
        "python3" => "py",
        "ruby" => "rb",
        "rust" => "rs",
        "sh" => "sh",
        "typescript" => "ts",
        "typescript-jsx" => "tsx",
        "yaml" => "yaml",
        other => other,
    }
}

/// Strata's ids, for the test that each one resolves (or is deliberately plain).
#[cfg(test)]
pub(crate) const STRATA_IDS: &[&str] = &[
    "sh",
    "c",
    "cpp",
    "c-sharp",
    "css",
    "dart",
    "diff",
    "docker",
    "go",
    "html",
    "java",
    "js",
    "json",
    "jsx",
    "kotlin",
    "lua",
    "makefile",
    "markdown",
    "latex",
    "php",
    "powershell",
    "python3",
    "ruby",
    "rust",
    "sql",
    "swift",
    "toml",
    "typescript",
    "typescript-jsx",
    "vala",
    "xml",
    "yaml",
];
