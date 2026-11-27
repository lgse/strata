// SPDX-License-Identifier: MIT

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
