//! The web's code block colours (`web/src/lib/highlight.ts`, `langOf` in `markdown.ts`): a small tokenizer per language,
//! coarse on purpose (comments, strings, numbers, keywords, types and tags). The web scans one regex of named groups
//! with lookarounds, which Rust's `regex` has not got; here each group is its own regex anchored at the scan position,
//! tried in the web's order, with the lookarounds as checks on the characters around the match. Same tokens.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

/// A token's kind: the `--syn-*` colour it takes, or `Plain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Kw,
    Str,
    Com,
    Num,
    Ty,
    /// A word: a keyword, a type (a capital then a small letter, in languages with types) or plain.
    Word,
}

/// The highlighter's name for a fence language (`langOf`), or None when it has none.
pub fn lang_of(lang: Option<&str>) -> Option<&'static str> {
    Some(match lang?.to_lowercase().as_str() {
        "rust" | "rs" => "rust",
        "js" | "javascript" | "jsx" | "mjs" | "ts" | "typescript" | "tsx" => "js",
        "json" => "json",
        "sh" | "bash" | "shell" | "zsh" | "console" => "sh",
        "py" | "python" => "py",
        "html" | "xml" | "svg" | "svelte" => "html",
        "css" => "css",
        _ => return None,
    })
}

type Before = fn(Option<char>) -> bool;
type After = fn(&str) -> bool;

struct Group {
    kind: Kind,
    re: Regex,
    /// The lookbehind, on the character before the match: tried first, so a scan inside a long word stays linear.
    before: Option<Before>,
    /// The lookahead, on the text after the match.
    after: Option<After>,
}

struct Spec {
    groups: Vec<Group>,
    kw: Vec<&'static str>,
    types: bool,
}

const W: &str = "[A-Za-z0-9_]"; // JS's \w is ASCII
const STR2: &str = r#""(?:[^"\\\n]|\\.)*"?"#;
const STR1: &str = r"'(?:[^'\\\n]|\\.)*'?";
const NUM: &str = r"[0-9][A-Za-z0-9_.]*"; // \b\d: the scan never stops inside a word that a word group would take
const BLOCK: &str = r"/\*[\s\S]*?(?:\*/|$)";
const ID: &str = r"[A-Za-z_$][A-Za-z0-9_$]*";

fn group(kind: Kind, pat: &str) -> Group {
    Group {
        kind,
        re: Regex::new(&format!(r"\A(?:{pat})")).expect("a valid pattern"),
        before: None,
        after: None,
    }
}

impl Group {
    fn before(mut self, f: Before) -> Group {
        self.before = Some(f);
        self
    }
    fn after(mut self, f: After) -> Group {
        self.after = Some(f);
        self
    }
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn spec(lang: &str) -> Spec {
    use Kind::*;
    let words = |s: &'static str| s.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>();
    match lang {
        "rust" => Spec {
            groups: vec![
                group(Com, &format!(r"//[^\n]*|{BLOCK}")),
                group(Str, &format!(r"{STR2}|'(?:\\.|[^\\'\n])'")),
                group(Num, NUM),
                // a macro: \b[a-z_]\w*!(?!=)
                group(Ty, &format!(r"[a-z_]{W}*!")).after(|a| !a.starts_with('=')),
                group(Word, ID),
            ],
            kw: words(
                "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
            ),
            types: true,
        },
        "js" => Spec {
            groups: vec![
                group(Com, &format!(r"//[^\n]*|{BLOCK}")),
                group(Str, &format!(r"{STR2}|{STR1}|`(?:[^`\\]|\\.)*`?")),
                group(Num, NUM),
                group(Word, ID),
            ],
            kw: words(
                "as async await break case catch class const continue debugger default delete do else enum export extends false finally for from function if import in instanceof interface let new null of return static super switch this throw true try type typeof undefined var void while yield",
            ),
            types: true,
        },
        "json" => Spec {
            groups: vec![
                // a key: a string followed by a colon
                group(Ty, STR2).after(|a| a.trim_start().starts_with(':')),
                group(Str, STR2),
                group(Num, r"-?[0-9][0-9.eE+-]*"),
                group(Word, ID),
            ],
            kw: words("true false null"),
            types: false,
        },
        "sh" => Spec {
            groups: vec![
                // a comment only after a space, ; | & or at the start
                group(Com, r"#[^\n]*")
                    .before(|b| b.is_none_or(|c| c.is_whitespace() || ";|&".contains(c))),
                group(Str, &format!(r"{STR2}|'[^']*'?")),
                group(Ty, r"\$\{?[A-Za-z0-9_@#?*!-]+\}?"),
                group(Num, NUM),
                group(Word, r"[A-Za-z_][A-Za-z0-9_-]*"),
            ],
            kw: words(
                "if then else elif fi for in do done case esac while until function return local export readonly set unset exit echo cd sudo",
            ),
            types: false,
        },
        "py" => Spec {
            groups: vec![
                group(Com, r"#[^\n]*"),
                group(
                    Str,
                    &format!(
                        r#"[rbfu]{{0,2}}(?:"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)|{STR2}|{STR1})"#
                    ),
                ),
                group(Ty, r"@[A-Za-z0-9_.]+"),
                group(Num, NUM),
                group(Word, ID),
            ],
            kw: words(
                "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
            ),
            types: true,
        },
        "html" => Spec {
            groups: vec![
                group(Com, r"<!--[\s\S]*?(?:-->|$)"),
                group(Kw, r"</?[A-Za-z0-9_:-]+|/?>"),
                // an attribute name: not inside a name, followed by =
                group(Ty, r"[A-Za-z0-9_:-]+")
                    .before(|b| !b.is_some_and(|c| word_char(c) || c == ':' || c == '-'))
                    .after(|a| a.starts_with('=')),
                group(Str, &format!("{STR2}|{STR1}")),
            ],
            kw: vec![],
            types: false,
        },
        "css" => Spec {
            groups: vec![
                group(Com, BLOCK),
                group(Str, &format!("{STR2}|{STR1}")),
                // a property: not inside a name, then a colon that is not a pseudo-element's ::
                group(Ty, r"[A-Za-z0-9_-]+")
                    .before(|b| !b.is_some_and(|c| word_char(c) || c == '-'))
                    .after(|a| {
                        let rest = a.trim_start();
                        rest.starts_with(':') && rest[1..].chars().next().is_some_and(|c| c != ':')
                    }),
                group(Kw, r"@[A-Za-z0-9_-]+|!important"),
                group(Num, r"#[0-9a-fA-F]{3,8}(?-u:\b)|-?[0-9][A-Za-z0-9_.%]*"),
            ],
            kw: vec![],
            types: false,
        },
        _ => Spec {
            groups: vec![],
            kw: vec![],
            types: false,
        },
    }
}

fn specs() -> &'static HashMap<&'static str, Spec> {
    static S: OnceLock<HashMap<&'static str, Spec>> = OnceLock::new();
    S.get_or_init(|| {
        ["rust", "js", "json", "sh", "py", "html", "css"]
            .into_iter()
            .map(|l| (l, spec(l)))
            .collect()
    })
}

/// `code` as (kind, text) tokens that join back to exactly `code`. `lang` is a name from [`lang_of`]; an unknown one
/// gives one plain token.
pub fn highlight(code: &str, lang: &str) -> Vec<(Kind, String)> {
    let mut out: Vec<(Kind, String)> = vec![];
    let mut push = |k: Kind, v: &str| {
        if v.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.0 == k => last.1.push_str(v),
            _ => out.push((k, v.to_string())),
        }
    };
    let Some(s) = specs().get(lang) else {
        push(Kind::Plain, code);
        return out;
    };
    let (mut at, mut plain) = (0, 0);
    while at < code.len() {
        let before = code[..at].chars().next_back();
        let found = s.groups.iter().find_map(|g| {
            if !g.before.is_none_or(|f| f(before)) {
                return None;
            }
            let m = g.re.find(&code[at..])?;
            let end = at + m.end();
            (m.end() > 0 && g.after.is_none_or(|f| f(&code[end..]))).then_some((g.kind, end))
        });
        let Some((kind, end)) = found else {
            at += code[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        push(Kind::Plain, &code[plain..at]);
        let v = &code[at..end];
        let kind = match kind {
            Kind::Word if s.kw.contains(&v) => Kind::Kw,
            Kind::Word if s.types && starts_like_a_type(v) => Kind::Ty,
            Kind::Word => Kind::Plain,
            k => k,
        };
        push(kind, v);
        (at, plain) = (end, end);
    }
    push(Kind::Plain, &code[plain..]);
    out
}

fn starts_like_a_type(v: &str) -> bool {
    let mut c = v.chars();
    c.next().is_some_and(|c| c.is_ascii_uppercase())
        && c.next().is_some_and(|c| c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(t: Vec<(Kind, String)>) -> Vec<String> {
        t.into_iter()
            .filter(|(k, _)| *k != Kind::Plain)
            .map(|(k, v)| {
                let k = match k {
                    Kind::Kw => "kw",
                    Kind::Str => "str",
                    Kind::Com => "com",
                    Kind::Num => "num",
                    _ => "ty",
                };
                format!("{k}:{v}")
            })
            .collect()
    }

    // The cases of web/src/lib/highlight.test.ts.
    #[test]
    fn tokens_join_back_in_every_language() {
        let src = "fn main() { let s = \"a\\\"b\"; // c\n /* d */ 42 `t` 'x' # e <a href=\"x\"> @d $V \"\"\"q\"\"\" é1\n";
        for l in ["rust", "js", "json", "sh", "py", "html", "css", "nope"] {
            let joined: String = highlight(src, l).into_iter().map(|t| t.1).collect();
            assert_eq!(joined, src, "{l}");
        }
        assert_eq!(highlight("x", "nope"), [(Kind::Plain, "x".to_string())]);
    }

    #[test]
    fn finds_the_main_kinds() {
        let k = |c: &str, l: &str| kinds(highlight(c, l));
        assert_eq!(
            k("let x = \"a\"; // hi\nconst N = 3;", "js"),
            ["kw:let", "str:\"a\"", "com:// hi", "kw:const", "num:3"]
        );
        assert_eq!(
            k("fn f() -> Option<u8> { println!(\"x\") }", "rust"),
            ["kw:fn", "ty:Option", "ty:println!", "str:\"x\""]
        );
        assert!(k("a != b", "rust").is_empty());
        assert_eq!(
            k("{\"k\": [1, true, null]}", "json"),
            ["ty:\"k\"", "num:1", "kw:true", "kw:null"]
        );
        assert_eq!(
            k("echo \"$HOME\" # c\na#b", "sh"),
            ["kw:echo", "str:\"$HOME\"", "com:# c"]
        );
        assert_eq!(
            k("@dec\ndef f(): return None # c", "py"),
            ["ty:@dec", "kw:def", "kw:return", "kw:None", "com:# c"]
        );
        assert_eq!(
            k("<a href=\"x\">t</a><!-- c -->", "html"),
            [
                "kw:<a",
                "ty:href",
                "str:\"x\"",
                "kw:>",
                "kw:</a>",
                "com:<!-- c -->"
            ]
        );
        assert_eq!(
            k("a { color: #fff; } /* c */", "css"),
            ["ty:color", "num:#fff", "com:/* c */"]
        );
        assert_eq!(lang_of(Some("TS")), Some("js"));
        assert_eq!(lang_of(Some("constructor")), None);
        assert_eq!(lang_of(None), None);
    }

    #[test]
    fn is_quick_on_hostile_code() {
        // The server's limit is 4000 characters.
        for s in ["/*", "\"", "'''", "<!--", "\\", "a", "#"] {
            let s = s.repeat(4000 / s.len());
            for l in ["rust", "js", "json", "sh", "py", "html", "css"] {
                let t = std::time::Instant::now();
                let n: usize = highlight(&s, l).iter().map(|x| x.1.len()).sum();
                assert_eq!(n, s.len());
                assert!(
                    t.elapsed().as_millis() < 500,
                    "{l} {}",
                    &s[..2.min(s.len())]
                );
            }
        }
    }
}
