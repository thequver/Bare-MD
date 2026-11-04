//! Syntax highlighting for fenced code blocks.
//!
//! It runs **inside the layout**, on the character ranges the layout already has,
//! so highlighting costs nothing at paint time: a code line becomes several runs
//! (keyword, string, comment …) and the painter draws them like any other styled
//! run. There is no extra pass, no regex engine and no per-frame work.
//!
//! The tokenizer is deliberately small: an identifier / number / string / comment
//! scanner with a per-language keyword table and comment and string delimiters.
//! That is enough for code blocks in a note, and it never mis-colours a page of
//! prose because it only ever runs on lines the parser marked as code.
//!
//! Multi-line state (a block comment opened last line, an unfinished string) is
//! carried across lines by [`State`], so a `/* … */` block that spans ten lines is
//! one comment, not ten broken ones.

/// What a stretch of code is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    /// Plain code text.
    Plain,
    Keyword,
    /// A type or constant: an identifier that starts with a capital.
    Type,
    /// An identifier directly followed by `(`.
    Func,
    Str,
    Num,
    Comment,
    /// A preprocessor line (`#include`, `#define`) in C-family languages.
    Meta,
}

/// Carried from one line of a block to the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct State {
    /// Inside a `/* … */` comment.
    pub block_comment: bool,
    /// Inside a string opened on an earlier line (`""".strip()` etc).
    pub string: Option<char>,
}

/// How one language is recognised: comment markers, quote characters, keywords.
struct Syntax {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    keywords: &'static [&'static str],
    /// `#word` at the start of a line is a preprocessor directive.
    preprocessor: bool,
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "union", "macro_rules",
];
const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield", "match", "case",
];
const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "export", "extends", "finally", "for", "function", "if",
    "import", "in", "instanceof", "let", "new", "of", "return", "static", "super", "switch",
    "this", "throw", "try", "typeof", "var", "void", "while", "yield", "true", "false", "null",
    "undefined", "interface", "type", "implements",
];
const C_KW: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "class", "namespace", "new", "delete",
    "public", "private", "protected", "template", "using", "virtual", "override", "nullptr",
    "true", "false", "bool", "string", "std", "auto",
];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
    "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return",
    "select", "struct", "switch", "type", "var", "nil", "true", "false",
];
const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function",
    "in", "return", "export", "local", "echo", "exit", "set", "source", "read", "cd", "sudo",
    "test",
];
const SQL_KW: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create",
    "table", "alter", "drop", "join", "left", "right", "inner", "outer", "on", "group", "by",
    "order", "having", "limit", "offset", "as", "and", "or", "not", "null", "is", "in", "distinct",
    "union", "all", "case", "when", "then", "else", "end", "primary", "key", "foreign",
    "references", "index", "default", "count", "sum", "avg", "min", "max",
];

fn syntax(lang: &str) -> Option<Syntax> {
    let id = lang.trim().to_ascii_lowercase();
    let id = id.split(['{', ' ', ',']).next().unwrap_or("").to_string();
    let s = match id.as_str() {
        "rust" | "rs" => Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            keywords: RUST_KW,
            preprocessor: false,
        },
        "py" | "python" | "python3" => Syntax {
            line_comments: &["#"],
            block_comment: None,
            quotes: &['"', '\''],
            keywords: PY_KW,
            preprocessor: false,
        },
        "js" | "jsx" | "javascript" | "ts" | "tsx" | "typescript" | "mjs" | "cjs" => Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            keywords: JS_KW,
            preprocessor: false,
        },
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "java" | "cs" | "csharp" | "kt" | "kotlin"
        | "swift" | "dart" | "php" | "scala" => Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            keywords: C_KW,
            preprocessor: true,
        },
        "go" | "golang" => Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '`'],
            keywords: GO_KW,
            preprocessor: false,
        },
        "sh" | "bash" | "zsh" | "shell" | "console" | "fish" => Syntax {
            line_comments: &["#"],
            block_comment: None,
            quotes: &['"', '\''],
            keywords: SH_KW,
            preprocessor: false,
        },
        "sql" => Syntax {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: &['\'', '"'],
            keywords: SQL_KW,
            preprocessor: false,
        },
        "toml" | "ini" | "cfg" | "conf" | "yaml" | "yml" => Syntax {
            line_comments: &["#"],
            block_comment: None,
            quotes: &['"', '\''],
            keywords: &["true", "false", "null", "yes", "no", "on", "off"],
            preprocessor: false,
        },
        "json" | "jsonc" => Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            keywords: &["true", "false", "null"],
            preprocessor: false,
        },
        "css" | "scss" | "less" => Syntax {
            line_comments: &[],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            keywords: &[
                "important", "media", "import", "keyframes", "font-face", "from", "to", "and",
                "not", "only", "screen", "print",
            ],
            preprocessor: false,
        },
        "md" | "markdown" | "txt" | "text" | "log" | "" => return None,
        // An unknown language still gets comments and strings, which is most of
        // the readability win, and no keywords (so nothing is coloured wrongly).
        _ => Syntax {
            line_comments: &["//", "#"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            keywords: &[],
            preprocessor: false,
        },
    };
    Some(s)
}

/// True when `lang` gets highlighted at all.
pub fn supported(lang: &str) -> bool {
    syntax(lang).is_some()
}

/// Classes for one line of `lang`, `chars.len()` entries long.
pub fn line(lang: &str, chars: &[char], st: &mut State) -> Vec<Tok> {
    let mut out = vec![Tok::Plain; chars.len()];
    let Some(syn) = syntax(lang) else { return out };
    let n = chars.len();
    let starts = |i: usize, pat: &str| -> bool {
        let mut k = i;
        for c in pat.chars() {
            if chars.get(k) != Some(&c) {
                return false;
            }
            k += 1;
        }
        true
    };
    let mut i = 0usize;

    // A block comment or a string that was left open on the previous line.
    if st.block_comment {
        if let Some(end) = find_from(chars, i, "*/") {
            for t in out.iter_mut().take(end + 2) {
                *t = Tok::Comment;
            }
            st.block_comment = false;
            i = end + 2;
        } else {
            out.iter_mut().for_each(|t| *t = Tok::Comment);
            return out;
        }
    }
    if let Some(q) = st.string {
        if let Some(end) = chars[i..].iter().position(|c| *c == q).map(|p| p + i) {
            for t in out.iter_mut().take(end + 1) {
                *t = Tok::Str;
            }
            st.string = None;
            i = end + 1;
        } else {
            out.iter_mut().for_each(|t| *t = Tok::Str);
            return out;
        }
    }

    if syn.preprocessor && chars.first() == Some(&'#') {
        // `#include <stdio.h>` / `#define X 1` — the whole line is meta.
        out.iter_mut().for_each(|t| *t = Tok::Meta);
        return out;
    }

    while i < n {
        let c = chars[i];
        // ---- comments ----
        if let Some((open, close)) = syn.block_comment {
            if starts(i, open) {
                if let Some(end) = find_from(chars, i + open.len(), close) {
                    for t in out.iter_mut().take(end + close.len()).skip(i) {
                        *t = Tok::Comment;
                    }
                    i = end + close.len();
                } else {
                    for t in out.iter_mut().skip(i) {
                        *t = Tok::Comment;
                    }
                    st.block_comment = true;
                    i = n;
                }
                continue;
            }
        }
        if syn.line_comments.iter().any(|p| starts(i, p)) {
            for t in out.iter_mut().skip(i) {
                *t = Tok::Comment;
            }
            break;
        }
        // ---- strings ----
        if syn.quotes.contains(&c) {
            let q = c;
            // Triple quotes (python, kotlin) count as one delimiter.
            let triple = starts(i, &format!("{q}{q}{q}"));
            let width = if triple { 3 } else { 1 };
            let mut j = i + width;
            let mut closed = false;
            while j < n {
                if triple {
                    if starts(j, &format!("{q}{q}{q}")) {
                        j += 3;
                        closed = true;
                        break;
                    }
                } else if chars[j] == q {
                    j += 1;
                    closed = true;
                    break;
                } else if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = j.min(n);
            for t in out.iter_mut().take(end).skip(i) {
                *t = Tok::Str;
            }
            if !closed && !triple {
                st.string = Some(q);
            }
            i = end;
            continue;
        }
        // ---- numbers ----
        if c.is_ascii_digit() {
            let mut j = i;
            while j < n && (chars[j].is_ascii_alphanumeric() || matches!(chars[j], '.' | '_' | 'x')) {
                j += 1;
            }
            for t in out.iter_mut().take(j).skip(i) {
                *t = Tok::Num;
            }
            i = j;
            continue;
        }
        // ---- identifiers ----
        if c.is_alphanumeric() || c == '_' {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let mut k = j;
            while k < n && chars[k] == ' ' {
                k += 1;
            }
            let class = if syn.keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&word) && (kw.to_lowercase() == word.to_lowercase())) {
                Tok::Keyword
            } else if chars.get(k) == Some(&'(') {
                Tok::Func
            } else if word.chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false) {
                Tok::Type
            } else {
                Tok::Plain
            };
            if class != Tok::Plain {
                for t in out.iter_mut().take(j).skip(i) {
                    *t = class;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn find_from(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    if pat.is_empty() {
        return Some(from);
    }
    let mut i = from;
    while i + pat.len() <= chars.len() {
        if chars[i..i + pat.len()] == pat[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(lang: &str, src: &str) -> Vec<Tok> {
        let chars: Vec<char> = src.chars().collect();
        let mut st = State::default();
        line(lang, &chars, &mut st)
    }

    #[test]
    fn keywords_strings_and_comments() {
        let c = classes("rust", "let s = \"hi\"; // note");
        assert_eq!(c[0], Tok::Keyword, "let");
        let s = "let s = \"hi\"; // note".chars().collect::<Vec<_>>();
        let q = s.iter().position(|ch| *ch == '"').unwrap();
        assert_eq!(c[q], Tok::Str);
        assert_eq!(c[c.len() - 1], Tok::Comment);
    }

    #[test]
    fn a_block_comment_carries_across_lines() {
        let mut st = State::default();
        let a: Vec<char> = "/* start".chars().collect();
        let ca = line("c", &a, &mut st);
        assert!(ca.iter().all(|t| *t == Tok::Comment));
        assert!(st.block_comment);
        let b: Vec<char> = "still inside".chars().collect();
        let cb = line("c", &b, &mut st);
        assert!(cb.iter().all(|t| *t == Tok::Comment));
        let d: Vec<char> = "*/ int x;".chars().collect();
        let cd = line("c", &d, &mut st);
        assert_eq!(cd[0], Tok::Comment);
        assert!(!st.block_comment);
        assert!(cd.iter().any(|t| *t == Tok::Keyword), "int after the comment");
    }

    #[test]
    fn plain_text_is_not_highlighted() {
        assert!(!supported(""));
        assert!(!supported("markdown"));
        let c = classes("", "some ordinary words");
        assert!(c.iter().all(|t| *t == Tok::Plain));
    }

    #[test]
    fn calls_and_types_are_told_apart() {
        let c = classes("rust", "compute(Value::new())");
        assert_eq!(c[0], Tok::Func, "compute is called");
        let v = "compute(Value::new())".chars().collect::<Vec<_>>();
        let p = v.iter().position(|ch| *ch == 'V').unwrap();
        assert_eq!(c[p], Tok::Type, "Value is capitalised");
    }
}
