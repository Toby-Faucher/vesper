//! The web app's Markdown subset (`web/src/lib/markdown.ts`), parsed into a tree the views draw as styled text. No
//! markup is ever read as anything but text. One left-to-right pass per line with a delimiter stack: linear time.
//! Keep the two in step; the tests are the web's.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Md {
    Text(String),
    Br,
    Hr,
    B(Vec<Md>),
    I(Vec<Md>),
    S(Vec<Md>),
    Quote(Vec<Md>),
    Li(Vec<Md>),
    /// Level 1 to 3.
    H(u8, Vec<Md>),
    Ul(Vec<Md>),
    Ol(u32, Vec<Md>),
    Code(String),
    Pre(String, Option<String>),
    /// The parsed href, which is also the text shown.
    Link(String),
    Mention(String),
}

const MAX_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Bold,
    Star,
    Under,
    Tilde,
}

impl Marker {
    fn text(self) -> &'static str {
        match self {
            Marker::Bold => "**",
            Marker::Star => "*",
            Marker::Under => "_",
            Marker::Tilde => "~~",
        }
    }

    fn node(self, c: Vec<Md>) -> Md {
        match self {
            Marker::Bold => Md::B(c),
            Marker::Star | Marker::Under => Md::I(c),
            Marker::Tilde => Md::S(c),
        }
    }
}

struct Frame {
    marker: Marker,
    c: Vec<Md>,
}

fn ws(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

fn word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn name_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The parsed href if `s` is an http or https URL without credentials. ASCII only: other characters are
/// percent-encoded and hosts punycoded, so the text shown is the link.
pub fn safe_url(s: &str) -> Option<String> {
    let u = url::Url::parse(s).ok()?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return None;
    }
    Some(u.into())
}

fn push_text(out: &mut Vec<Md>, v: &str) {
    if v.is_empty() {
        return;
    }
    if let Some(Md::Text(last)) = out.last_mut() {
        last.push_str(v);
    } else {
        out.push(Md::Text(v.into()));
    }
}

fn push_all(out: &mut Vec<Md>, nodes: Vec<Md>) {
    for n in nodes {
        match n {
            Md::Text(v) => push_text(out, &v),
            n => out.push(n),
        }
    }
}

/// Where a bare link that starts at `i` ends: at white space, less trailing punctuation.
fn link_end(s: &[char], i: usize) -> usize {
    let mut j = i;
    while j < s.len() && !s[j].is_whitespace() {
        j += 1;
    }
    while j > i && ".,;:!?)*_~".contains(s[j - 1]) {
        j -= 1;
    }
    j
}

fn top<'a>(root: &'a mut Vec<Md>, stack: &'a mut [Frame]) -> &'a mut Vec<Md> {
    match stack.last_mut() {
        Some(f) => &mut f.c,
        None => root,
    }
}

/// Inline Markdown of one line (no newlines).
fn inline(src: &str) -> Vec<Md> {
    let s: Vec<char> = src.chars().collect();
    let at = |i: usize| s.get(i).copied();
    let text = |a: usize, b: usize| s[a..b].iter().collect::<String>();
    let mut root = vec![];
    let mut stack: Vec<Frame> = vec![];
    let mut open = [0usize; 4];
    // An unclosed frame becomes its marker as text followed by its content.
    let unwind = |root: &mut Vec<Md>, stack: &mut Vec<Frame>, open: &mut [usize; 4]| {
        let f = stack.pop().expect("a frame");
        open[f.marker as usize] -= 1;
        let t = top(root, stack);
        push_text(t, f.marker.text());
        push_all(t, f.c);
    };
    let (mut i, mut start) = (0, 0); // start of the pending plain text
    while i < s.len() {
        let ch = s[i];
        let prev = i.checked_sub(1).and_then(at);
        let mut emit = |n: Md, next: usize, i: &mut usize, start: &mut usize| {
            let t = top(&mut root, &mut stack);
            push_text(t, &text(*start, *i));
            t.push(n);
            (*i, *start) = (next, next);
        };
        if matches!(ch, '*' | '_' | '~') {
            let marker = match (ch, at(i + 1)) {
                ('~', Some('~')) => Marker::Tilde,
                ('~', _) => {
                    i += 1;
                    continue;
                }
                ('*', Some('*')) => Marker::Bold,
                ('*', _) => Marker::Star,
                _ => Marker::Under,
            };
            let len = marker.text().len();
            let next = at(i + len);
            if open[marker as usize] > 0 && !ws(prev) && !(marker == Marker::Under && word(next)) {
                push_text(top(&mut root, &mut stack), &text(start, i));
                while stack.last().expect("open").marker != marker {
                    unwind(&mut root, &mut stack, &mut open);
                }
                let f = stack.pop().expect("open");
                open[marker as usize] -= 1;
                let t = top(&mut root, &mut stack);
                if f.c.is_empty() {
                    push_text(t, &marker.text().repeat(2));
                } else {
                    t.push(marker.node(f.c));
                }
                i += len;
                start = i;
            } else if !ws(next)
                && !(marker == Marker::Under && word(prev))
                && stack.len() < MAX_DEPTH
            {
                push_text(top(&mut root, &mut stack), &text(start, i));
                stack.push(Frame { marker, c: vec![] });
                open[marker as usize] += 1;
                i += len;
                start = i;
            } else {
                i += len;
            }
        } else if ch == '`' {
            // With no closing backtick, no later backtick exists either, so this scan runs once per line at most.
            match s[i + 1..].iter().position(|&c| c == '`') {
                Some(0) => i += 2,
                Some(p) => {
                    let j = i + 1 + p;
                    emit(Md::Code(text(i + 1, j)), j + 1, &mut i, &mut start);
                }
                None => i += 1,
            }
        } else if ch == 'h' && !word(prev) && {
            let rest: String = s[i..s.len().min(i + 8)].iter().collect();
            rest.starts_with("http://") || rest.starts_with("https://")
        } {
            let j = link_end(&s, i);
            match safe_url(&text(i, j)) {
                Some(href) => emit(Md::Link(href), j, &mut i, &mut start),
                None => i = j,
            }
        } else if ch == '@' && !name_char(prev) && prev != Some('@') {
            let mut j = i + 1;
            while name_char(at(j)) {
                j += 1;
            }
            if (3..=24).contains(&(j - i - 1)) {
                emit(Md::Mention(text(i + 1, j)), j, &mut i, &mut start);
            } else {
                i = j;
            }
        } else {
            i += 1;
        }
    }
    push_text(top(&mut root, &mut stack), &text(start, s.len()));
    while !stack.is_empty() {
        unwind(&mut root, &mut stack, &mut open);
    }
    root
}

/// The closing fence line for a fence at line `k`. An unclosed fence means no later line starts with ``` either, so
/// the scan runs once per message at most.
fn fence_end(lines: &[&str], k: usize) -> Option<usize> {
    if !lines[k].starts_with("```") {
        return None;
    }
    (k + 1..lines.len()).find(|&e| lines[e].starts_with("```"))
}

/// One word after ``` is the language.
fn is_lang(s: &str) -> bool {
    (1..=20).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "+#._-".contains(c))
}

/// `^(#{1,6}) +(?=\S)`: the level and the length of the prefix.
fn heading(line: &str) -> Option<(usize, usize)> {
    let n = line.bytes().take_while(|&b| b == b'#').count();
    let sp = line[n..].bytes().take_while(|&b| b == b' ').count();
    let next = line[n + sp..].chars().next();
    ((1..=6).contains(&n) && sp > 0 && !ws(next)).then_some((n, n + sp))
}

/// `^ {0,3}([-*_])(?: *\1){2,} *$`: a rule.
fn is_hr(line: &str) -> bool {
    let lead = line.bytes().take_while(|&b| b == b' ').count();
    let rest = &line[lead..];
    let Some(c) = rest.chars().next() else {
        return false;
    };
    lead <= 3
        && matches!(c, '-' | '*' | '_')
        && rest.chars().all(|x| x == c || x == ' ')
        && rest.chars().filter(|&x| x == c).count() >= 3
}

/// `^( *)([-*+]|(\d{1,9})[.)]) +(?=\S)`: a list item's indent, its number (none for a bullet) and prefix length.
fn item(line: &str) -> Option<(usize, Option<u32>, usize)> {
    let ind = line.bytes().take_while(|&b| b == b' ').count();
    let rest = &line[ind..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (num, mark) = match rest.bytes().next()? {
        b'-' | b'*' | b'+' => (None, 1),
        _ if (1..=9).contains(&digits)
            && matches!(rest.as_bytes().get(digits), Some(b'.' | b')')) =>
        {
            (rest[..digits].parse().ok(), digits + 1)
        }
        _ => return None,
    };
    let sp = rest[mark..].bytes().take_while(|&b| b == b' ').count();
    let next = rest[mark + sp..].chars().next();
    (sp > 0 && !ws(next)).then_some((ind, num, ind + mark + sp))
}

struct OpenList {
    l: Vec<Md>,
    ind: usize,
    /// The first number of a numbered list.
    start: Option<u32>,
}

/// Puts the innermost open list into its parent: the last item of the list around it, or `out`.
fn close(open: &mut Vec<OpenList>, out: &mut Vec<Md>) {
    let o = open.pop().expect("an open list");
    let node = match o.start {
        Some(n) => Md::Ol(n, o.l),
        None => Md::Ul(o.l),
    };
    match open.last_mut().and_then(|p| p.l.last_mut()) {
        Some(Md::Li(c)) => c.push(node),
        _ => out.push(node),
    }
}

/// Parses a message body. Unknown syntax, unclosed markers and raw HTML stay text.
pub fn parse(src: &str) -> Vec<Md> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out = vec![];
    let mut prev_inline = false;
    let mut k = 0;
    while k < lines.len() {
        let line = lines[k];
        if let Some(e) = fence_end(&lines, k) {
            // Text on a fence line (an info string that is not one word, or a tail after the closing fence) is
            // shown, so nothing is hidden. One word is the language, the block's label.
            let (head, tail) = (&line[3..], &lines[e][3..]);
            let lang = is_lang(head).then(|| head.to_string());
            if !head.is_empty() && lang.is_none() {
                if prev_inline {
                    out.push(Md::Br);
                }
                push_all(&mut out, inline(head));
            }
            out.push(Md::Pre(lines[k + 1..e].join("\n"), lang));
            push_all(&mut out, inline(tail));
            k = e + 1;
            prev_inline = !tail.is_empty();
            continue;
        }
        if line.starts_with("> ") {
            let mut c = vec![];
            while k < lines.len() && lines[k].starts_with("> ") {
                if !c.is_empty() {
                    c.push(Md::Br);
                }
                push_all(&mut c, inline(&lines[k][2..]));
                k += 1;
            }
            out.push(Md::Quote(c));
            prev_inline = false;
            continue;
        }
        if let Some((n, len)) = heading(line) {
            out.push(Md::H(n.min(3) as u8, inline(&line[len..])));
            (k, prev_inline) = (k + 1, false);
            continue;
        }
        if is_hr(line) {
            out.push(Md::Hr);
            (k, prev_inline) = (k + 1, false);
            continue;
        }
        if item(line).is_some() {
            // A run of items is one or more lists. An item indented deeper than the one above opens a list inside it
            // (up to MAX_DEPTH levels); a change of kind (bullets or numbers) at one level starts a sibling list there.
            let mut open: Vec<OpenList> = vec![];
            while k < lines.len() && !is_hr(lines[k]) {
                let Some((ind, start, len)) = item(lines[k]) else {
                    break;
                };
                while open.last().is_some_and(|t| ind < t.ind) {
                    close(&mut open, &mut out);
                }
                let deeper = |t: &OpenList, depth: usize| ind > t.ind && depth < MAX_DEPTH;
                match open.last() {
                    Some(t) if t.start.is_some() != start.is_some() && !deeper(t, open.len()) => {
                        let ind = t.ind;
                        close(&mut open, &mut out);
                        open.push(OpenList {
                            l: vec![],
                            ind,
                            start,
                        });
                    }
                    Some(t) if !deeper(t, open.len()) => {} // the same list
                    _ => open.push(OpenList {
                        l: vec![],
                        ind,
                        start,
                    }),
                }
                let o = open.last_mut().expect("an open list");
                o.l.push(Md::Li(inline(&lines[k][len..])));
                k += 1;
            }
            while !open.is_empty() {
                close(&mut open, &mut out);
            }
            prev_inline = false;
            continue;
        }
        if prev_inline {
            out.push(Md::Br);
        }
        push_all(&mut out, inline(line));
        prev_inline = true;
        k += 1;
    }
    out
}

/// The text a reader sees, on one line: for reply quotes and pins.
pub fn plain(nodes: &[Md]) -> String {
    fn walk(list: &[Md], out: &mut String) {
        for n in list {
            match n {
                Md::Text(v) | Md::Code(v) | Md::Link(v) => out.push_str(v),
                Md::Pre(v, _) => {
                    out.push(' ');
                    out.push_str(v);
                    out.push(' ');
                }
                Md::Br | Md::Hr => out.push(' '),
                Md::Mention(u) => {
                    out.push('@');
                    out.push_str(u);
                }
                Md::Quote(c) | Md::H(_, c) | Md::Li(c) => {
                    out.push(' ');
                    walk(c, out);
                    out.push(' ');
                }
                Md::B(c) | Md::I(c) | Md::S(c) | Md::Ul(c) | Md::Ol(_, c) => walk(c, out),
            }
        }
    }
    let mut s = String::new();
    walk(nodes, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    //! The cases of `web/src/lib/markdown.test.ts` (less `decorate` and `langOf`, which the desktop app has not got).
    use super::*;
    use Md::*;

    fn t(v: &str) -> Md {
        Text(v.into())
    }
    fn li(v: &str, sub: Vec<Md>) -> Md {
        let mut c = vec![t(v)];
        c.extend(sub);
        Li(c)
    }
    fn links(nodes: &[Md]) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|n| match n {
                Link(h) => vec![h.clone()],
                B(c) | I(c) | S(c) | Quote(c) | Li(c) | H(_, c) | Ul(c) | Ol(_, c) => links(c),
                _ => vec![],
            })
            .collect()
    }
    fn kids(n: &Md) -> &[Md] {
        match n {
            B(c) | I(c) | S(c) | Quote(c) | Li(c) | H(_, c) | Ul(c) | Ol(_, c) => c,
            _ => &[],
        }
    }

    #[test]
    fn markdown_is_inert() {
        assert_eq!(
            parse("<script>alert(1)</script>"),
            [t("<script>alert(1)</script>")]
        );
        assert_eq!(
            parse("<img src=x onerror=alert(1)>"),
            [t("<img src=x onerror=alert(1)>")]
        );
        assert!(links(&parse("javascript:alert(1)")).is_empty());
        assert_eq!(
            parse("https://x.test/a?b=<c>"),
            [Link("https://x.test/a?b=%3Cc%3E".into())]
        );
    }

    #[test]
    fn only_http_and_https_links_without_credentials_are_links() {
        for s in [
            "javascript:alert(1)",
            "data:text/html,x",
            "vbscript:x",
            "//evil.test/x",
            "https://user:pw@evil.test/",
            "https://user@evil.test/",
            "ftp://x.test/",
            "mailto:a@b.test",
        ] {
            assert_eq!(safe_url(s), None, "{s}");
        }
        for s in [
            "jAvAsCrIpT:alert(1)",
            "xhttps://a.test",
            "[a](javascript:alert(1))",
            "https://user:pw@evil.test/",
        ] {
            assert!(links(&parse(s)).is_empty(), "{s}");
        }
        assert_eq!(
            safe_url("https://a.test/x").as_deref(),
            Some("https://a.test/x")
        );
        // No [text](url) links: the URL itself shows, so the text cannot hide where it goes.
        assert_eq!(
            parse("[bank](https://evil.test)"),
            [t("[bank]("), Link("https://evil.test/".into()), t(")")]
        );
    }

    #[test]
    fn links_show_the_real_host() {
        assert_eq!(
            parse("https://bank.test/\u{202e}gnp.exe"),
            [Link("https://bank.test/%E2%80%AEgnp.exe".into())]
        );
        let idn = links(&parse("https://\u{430}pple.test/"));
        assert_eq!(idn, ["https://xn--pple-43d.test/"]);
        let zw = links(&parse("https://a.test/x\u{200b}y"));
        for h in zw.iter().chain(&idn) {
            assert!(h.bytes().all(|b| (0x21..=0x7e).contains(&b)), "{h}");
        }
    }

    #[test]
    fn text_on_a_code_fence_line_is_shown() {
        assert_eq!(
            parse("```js hi @bob\nx\n```after\nnext"),
            [
                t("js hi "),
                Mention("bob".into()),
                Pre("x".into(), None),
                t("after"),
                Br,
                t("next")
            ]
        );
        assert_eq!(
            parse("a\n```js\nx\n```"),
            [t("a"), Pre("x".into(), Some("js".into()))]
        );
    }

    #[test]
    fn markdown_basic() {
        assert_eq!(
            parse("**b** *i* _j_ ~~s~~ `c<d>`"),
            [
                B(vec![t("b")]),
                t(" "),
                I(vec![t("i")]),
                t(" "),
                I(vec![t("j")]),
                t(" "),
                S(vec![t("s")]),
                t(" "),
                Code("c<d>".into())
            ]
        );
        assert_eq!(
            parse("a\n```\n<b>**x**</b>\n```\nb"),
            [t("a"), Pre("<b>**x**</b>".into(), None), t("b")]
        );
        assert_eq!(
            parse("> one **two**\n> > three\nafter"),
            [
                Quote(vec![t("one "), B(vec![t("two")]), Br, t("> three")]),
                t("after")
            ]
        );
        assert_eq!(
            parse("hi @Bob_1, @ab @@x a@bcd"),
            [t("hi "), Mention("Bob_1".into()), t(", @ab @@x a@bcd")]
        );
        assert_eq!(
            parse("see https://a.test/x?y=1). ok"),
            [t("see "), Link("https://a.test/x?y=1".into()), t("). ok")]
        );
        assert_eq!(parse("a\nb\n\nc"), [t("a"), Br, t("b"), Br, Br, t("c")]);
        assert_eq!(
            parse("**a *b* c**"),
            [B(vec![t("a "), I(vec![t("b")]), t(" c")])]
        );
        assert_eq!(parse("snake_case_name"), [t("snake_case_name")]);
    }

    #[test]
    fn markdown_unclosed_is_text() {
        assert_eq!(parse("**a"), [t("**a")]);
        assert_eq!(parse("a **b *c"), [t("a **b *c")]);
        assert_eq!(parse("`a"), [t("`a")]);
        assert_eq!(parse("```\nx"), [t("```"), Br, t("x")]);
        assert_eq!(parse("2 * 3 * 4"), [t("2 * 3 * 4")]);
        assert_eq!(parse("a ****"), [t("a ****")]);
        assert_eq!(parse("****"), [Hr]); // a line of three or more is a rule, as CommonMark
        assert_eq!(parse(">no space"), [t(">no space")]);
    }

    #[test]
    fn nesting_stops_at_depth_4() {
        fn depth(n: &[Md]) -> usize {
            n.iter()
                .map(|x| match x {
                    B(c) | I(c) | S(c) => 1 + depth(c),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }
        assert_eq!(depth(&parse("*a *b *c *d *e e* d* c* b* a*")), 4);
    }

    #[test]
    fn markdown_is_linear() {
        let big = 100_000;
        let cases = [
            "*".repeat(big),
            "_".repeat(big),
            "~".repeat(big),
            "`".repeat(big),
            "**a".repeat(big / 3),
            "*a ".repeat(big / 3),
            "> ".repeat(big / 2),
            ">\n".repeat(big / 2),
            "```\n".repeat(big / 4),
            "https://".repeat(big / 8),
            "http://a.test/".repeat(big / 14),
            "@".repeat(big),
            "@abc".repeat(big / 4),
            "<script>".repeat(big / 8),
            "**a *b ~~c _d ".repeat(big / 14),
            "a".repeat(big),
            "#".repeat(big),
            "# a\n".repeat(big / 4),
            "- ".repeat(big / 2),
            "- a\n".repeat(big / 4),
            "1. a\n- b\n".repeat(big / 10),
            "-".repeat(big),
            "- - ".repeat(big / 4) + "x",
            "1".repeat(big) + ". a",
            "```js\n".repeat(big / 6),
            "- a\n    1. b\n".repeat(big / 12),
            " ".repeat(big) + "- a",
            " - a\n  - b\n   - c\n    - d\n".repeat(big / 32),
        ];
        for s in cases {
            let t = std::time::Instant::now();
            let out = parse(&s);
            // The web allows 250 ms in an optimised build; this is a debug build.
            assert!(
                t.elapsed() < std::time::Duration::from_secs(2),
                "{}",
                &s[..16]
            );
            assert!(!out.is_empty());
        }
    }

    #[test]
    fn markdown_blocks() {
        assert_eq!(
            parse("# One\n## Two **b**\n#### Four\n#no"),
            [
                H(1, vec![t("One")]),
                H(2, vec![t("Two "), B(vec![t("b")])]),
                H(3, vec![t("Four")]),
                t("#no")
            ]
        );
        assert_eq!(
            parse("- a\n* b\n+ c\n3. x\n4) y\nafter"),
            [
                Ul(vec![li("a", vec![]), li("b", vec![]), li("c", vec![])]),
                Ol(3, vec![li("x", vec![]), li("y", vec![])]),
                t("after")
            ]
        );
        assert_eq!(parse("a\n---\n* * *\nb"), [t("a"), Hr, Hr, t("b")]);
        assert_eq!(parse("- - -"), [Hr]);
        for s in ["-a", "- ", "#", "# ", "1.a", "--", "2 * 3"] {
            assert_eq!(parse(s), [t(s)], "{s}");
        }
        assert_eq!(
            parse("```RusT\nfn a() {}\n```"),
            [Pre("fn a() {}".into(), Some("RusT".into()))]
        );
    }

    #[test]
    fn indented_items_nest_under_the_item_above() {
        assert_eq!(
            parse("- a\n  - b\n    1. c\n  - d\n- e"),
            [Ul(vec![
                li(
                    "a",
                    vec![Ul(vec![
                        li("b", vec![Ol(1, vec![li("c", vec![])])]),
                        li("d", vec![])
                    ])]
                ),
                li("e", vec![])
            ])]
        );
        // A change of kind at one level starts a sibling list there; at the top it starts a new list.
        assert_eq!(
            parse("- a\n  - b\n  1. c\n3. d"),
            [
                Ul(vec![li(
                    "a",
                    vec![Ul(vec![li("b", vec![])]), Ol(1, vec![li("c", vec![])])]
                )]),
                Ol(3, vec![li("d", vec![])])
            ]
        );
        // An indented first item is a list; a dedent past every level starts a new one.
        assert_eq!(
            parse("  - a\n- b"),
            [Ul(vec![li("a", vec![])]), Ul(vec![li("b", vec![])])]
        );
        // Nesting stops at depth 4: deeper items stay at the fourth level.
        fn levels(n: &[Md]) -> usize {
            n.iter()
                .map(|x| usize::from(matches!(x, Ul(_))) + levels(kids(x)))
                .max()
                .unwrap_or(0)
        }
        let deep = parse(&["- 1", " - 2", "  - 3", "   - 4", "    - 5"].join("\n"));
        assert_eq!(levels(&deep), 4);
    }

    #[test]
    fn hostile_input_in_blocks_stays_text() {
        assert_eq!(
            parse("# <img src=x onerror=alert(1)>"),
            [H(1, vec![t("<img src=x onerror=alert(1)>")])]
        );
        assert_eq!(
            parse("- [x](javascript:alert(1))"),
            [Ul(vec![li("[x](javascript:alert(1))", vec![])])]
        );
        assert_eq!(
            parse("```<script>\nx\n```"),
            [t("<script>"), Pre("x".into(), None)]
        );
        assert_eq!(
            parse("```js\"onload=x\n</code>\n```"),
            [t("js\"onload=x"), Pre("</code>".into(), None)]
        );
        assert_eq!(
            parse("![img](https://x.test/a.png)"),
            [t("![img]("), Link("https://x.test/a.png".into()), t(")")]
        );
    }

    #[test]
    fn plain_is_the_text_on_one_line() {
        assert_eq!(
            plain(&parse("**hi** @bob\n- a\n- `b`\n```\ncode\n```")),
            "hi @bob a b code"
        );
    }
}
