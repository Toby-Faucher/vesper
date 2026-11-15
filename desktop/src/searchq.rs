//! The search box's inline syntax: a copy of `web/src/lib/ui/searchq.ts` (and `marks` of `search.svelte.ts`); keep
//! them in step. `from:sam in:general after:2026-10-01 before:2026-10-31 hello`. A value with a space goes in quotes
//! (`from:"Sam Smith"`). Anything else, other `word:` tokens included, is text for the server, which quotes every
//! term itself. Offsets are byte offsets (the web's are UTF-16 units).

use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use regex::Regex;
use vesper_protocol::{RoomInfo, UserInfo};

/// The longest box the dialog takes, in characters, and the most words sent.
pub const MAX_QUERY: usize = 200;
const MAX_TERMS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    From,
    In,
    After,
    Before,
}

impl Key {
    fn name(self) -> &'static str {
        match self {
            Key::From => "from",
            Key::In => "in",
            Key::After => "after",
            Key::Before => "before",
        }
    }
    fn of(s: &str) -> Option<Key> {
        [Key::From, Key::In, Key::After, Key::Before]
            .into_iter()
            .find(|k| k.name() == s)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub q: String,
    pub room_id: Option<i64>,
    pub author_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// The `from:` and `in:` names found, in that order.
    pub chips: Vec<(Key, String)>,
    pub unknown: Vec<String>,
}

struct Token {
    start: usize,
    end: usize,
    key: Option<Key>,
    value: String,
}

/// A run of quoted text or of anything but spaces and quotes, glued together: `from:"Sam Smith"`, `"two words"`.
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:"[^"]*"?|[^\s"]+)+"#).expect("the token pattern"));

fn unquote(s: &str) -> String {
    s.replace('"', "")
}

fn tokens(input: &str) -> Vec<Token> {
    TOKEN
        .find_iter(input)
        .map(|m| {
            let text = m.as_str();
            let key = text
                .find(':')
                .filter(|&c| c > 0)
                .and_then(|c| Some((Key::of(&text[..c].to_lowercase())?, c)));
            Token {
                start: m.start(),
                end: m.end(),
                key: key.map(|k| k.0),
                value: unquote(key.map_or(text, |(_, c)| &text[c + 1..])),
            }
        })
        .collect()
}

/// Local midnight at the start of a YYYY-MM-DD day (or `today`, `yesterday`), or of the day after it, in unix ms.
fn day<Tz: TimeZone>(v: &str, today: NaiveDate, tz: &Tz, next: bool) -> Option<i64> {
    let d = match v.to_lowercase().as_str() {
        "today" => today,
        "yesterday" => today.pred_opt()?,
        _ => {
            let b = v.as_bytes();
            let shape = b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && b.iter()
                    .enumerate()
                    .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
            if !shape {
                return None;
            }
            NaiveDate::parse_from_str(v, "%Y-%m-%d").ok()?
        }
    };
    let d = if next { d + Duration::days(1) } else { d };
    let midnight = d.and_hms_opt(0, 0, 0)?;
    Some(
        tz.from_local_datetime(&midnight)
            .earliest()?
            .timestamp_millis(),
    )
}

/// The query the server gets for `input`. Dates are whole local days, both included. A repeated key: the last one
/// counts. A key with no value yet is left out. Only the first MAX_QUERY characters and MAX_TERMS words count.
pub fn parse_search<'a, Tz: TimeZone>(
    input: &str,
    users: impl Iterator<Item = &'a UserInfo> + Clone,
    rooms: &[RoomInfo],
    today: NaiveDate,
    tz: &Tz,
) -> Parsed {
    let mut out = Parsed::default();
    let mut text: Vec<String> = vec![];
    let mut chip: Vec<(Key, String)> = vec![];
    let mut bad: Vec<(Key, String)> = vec![];
    let cut: String = input.chars().take(MAX_QUERY).collect();
    for t in tokens(&cut) {
        let Some(key) = t.key else {
            if !t.value.is_empty() {
                text.push(t.value);
            }
            continue;
        };
        let v = t.value.trim();
        if v.is_empty() {
            continue;
        }
        let low = v.to_lowercase();
        bad.retain(|b| b.0 != key);
        chip.retain(|c| c.0 != key);
        match key {
            Key::From => {
                let u = users
                    .clone()
                    .find(|x| x.username.to_lowercase() == low)
                    .or_else(|| users.clone().find(|x| x.display_name.to_lowercase() == low));
                out.author_id = u.map(|u| u.user_id.clone());
                match u {
                    Some(u) => chip.push((key, u.display_name.clone())),
                    None => bad.push((key, format!("No one called {v}."))),
                }
            }
            Key::In => {
                let r = rooms.iter().find(|x| x.name.to_lowercase() == low);
                out.room_id = r.map(|r| r.id);
                match r {
                    Some(r) => chip.push((key, r.name.clone())),
                    None => bad.push((key, format!("No room called {v}."))),
                }
            }
            Key::After | Key::Before => {
                let ms = day(v, today, tz, key == Key::Before);
                if key == Key::After {
                    out.from = ms;
                } else {
                    out.to = ms.map(|m| m - 1);
                }
                match ms {
                    Some(_) => chip.push((key, v.to_string())),
                    None => bad.push((key, format!("{v} is not a date. Use YYYY-MM-DD."))),
                }
            }
        }
    }
    out.q = text
        .join(" ")
        .split_whitespace()
        .take(MAX_TERMS)
        .collect::<Vec<_>>()
        .join(" ");
    for k in [Key::From, Key::In] {
        out.chips.extend(chip.iter().filter(|c| c.0 == k).cloned());
    }
    out.unknown = bad.into_iter().map(|b| b.1).collect();
    if let (Some(f), Some(t)) = (out.from, out.to)
        && f > t
    {
        out.unknown
            .push("The After date is later than the Before date.".into());
    }
    out
}

fn quote(v: &str) -> String {
    if v.contains(|c: char| c.is_whitespace() || c == '"') {
        format!("\"{}\"", unquote(v))
    } else {
        v.to_string()
    }
}

/// `input` with every `key:` token removed, then `key:value` put first when a value is given.
pub fn set_key(input: &str, key: Key, value: Option<&str>) -> String {
    let mut rest = input.to_string();
    for t in tokens(input).iter().rev().filter(|t| t.key == Some(key)) {
        rest.replace_range(t.start..t.end, "");
    }
    let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    match value {
        // A trailing space after a lone filter, ready for the words.
        Some(v) if !v.is_empty() => format!("{}:{} {rest}", key.name(), quote(v)),
        _ => rest,
    }
}

/// The raw value of the last `key:` token (YYYY-MM-DD or '').
pub fn key_value(input: &str, key: Key) -> String {
    tokens(input)
        .into_iter()
        .rfind(|t| t.key == Some(key))
        .map_or(String::new(), |t| t.value)
}

/// A `from:` or `in:` token the caret is in: its key, the text after the colon and its place in `input`.
#[derive(Clone, Debug, PartialEq)]
pub struct At {
    pub key: Key,
    pub q: String,
    pub start: usize,
    pub end: usize,
}

pub fn key_at(input: &str, caret: usize) -> Option<At> {
    let t = tokens(input)
        .into_iter()
        .find(|x| x.start < caret && caret <= x.end)?;
    let key = t.key.filter(|k| matches!(k, Key::From | Key::In))?;
    Some(At {
        key,
        q: t.value,
        start: t.start,
        end: t.end,
    })
}

/// `input` with the token at `at` replaced by `key:value` and a space, and where the caret goes after it.
pub fn complete(input: &str, at: &At, value: &str) -> (String, usize) {
    let put = format!("{}:{} ", at.key.name(), quote(value));
    let after = input[at.end..].trim_start();
    (
        format!("{}{put}{after}", &input[..at.start]),
        at.start + put.len(),
    )
}

/// True when `q` has nothing the server can search (no letter or digit).
pub fn nothing_to_search(q: &str) -> bool {
    !q.chars().any(char::is_alphanumeric)
}

/// True when the last word ends in a single letter or digit, which the server matches as a whole word only.
pub fn short_last(q: &str) -> bool {
    let last = q.split_whitespace().last().unwrap_or("");
    let run = last
        .split(|c: char| !c.is_alphanumeric())
        .rfind(|s| !s.is_empty())
        .unwrap_or("");
    run.chars().count() == 1
}

/// A search snippet as runs of (text, is a match): the \u{2} ... \u{3} markers come out, white space runs read as one
/// space. shortcut: Markdown markers stay as typed (the web drops them with `decorate`, which the desktop has not
/// got); port `decorate` if a snippet's `**` reads badly.
pub fn marks(snippet: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = vec![];
    let (mut on, mut space) = (false, true);
    for ch in snippet.chars() {
        match ch {
            '\u{2}' => on = true,
            '\u{3}' => on = false,
            c => {
                let sp = c.is_whitespace();
                if sp && space {
                    continue;
                }
                space = sp;
                let c = if sp { ' ' } else { c };
                match out.last_mut() {
                    Some(l) if l.1 == on => l.0.push(c),
                    _ => out.push((c.to_string(), on)),
                }
            }
        }
    }
    if let Some(l) = out.last_mut() {
        l.0.truncate(l.0.trim_end().len());
        if l.0.is_empty() {
            out.pop();
        }
    }
    out
}

/// The (year, month, day) of a unix ms instant in `tz`, for tests and the date fields.
pub fn ymd<Tz: TimeZone>(ms: i64, tz: &Tz) -> Option<(i32, u32, u32)> {
    let d = tz.timestamp_millis_opt(ms).single()?.date_naive();
    Some((d.year(), d.month(), d.day()))
}

#[cfg(test)]
mod tests {
    //! The cases of `web/src/lib/ui/searchq.test.ts` that the Rust copy can share.
    use super::*;
    use chrono::Utc;

    fn user(id: &str, username: &str, name: &str) -> UserInfo {
        serde_json::from_value(serde_json::json!({ "user_id": id, "username": username, "display_name": name,
            "accent": "", "role": "member", "banned": false, "bio": "", "avatar": null, "ui_accent": null }))
        .unwrap()
    }
    fn room(id: i64, name: &str) -> RoomInfo {
        serde_json::from_value(
            serde_json::json!({ "kind": "room", "id": id, "name": name, "position": id,
            "restricted": false, "members": [], "archived": false }),
        )
        .unwrap()
    }
    fn parse(s: &str) -> Parsed {
        let users = [user("s", "sam", "Sam Smith"), user("a", "alice", "alice")];
        let rooms = [room(1, "general"), room(2, "game night")];
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        parse_search(s, users.iter(), &rooms, today, &Utc)
    }

    #[test]
    fn keys_words_and_dates() {
        let p = parse(
            r#"from:"Sam Smith" in:"game night" after:2026-10-01 before:2026-10-31 hello  there"#,
        );
        assert_eq!(p.q, "hello there");
        assert_eq!((p.author_id.as_deref(), p.room_id), (Some("s"), Some(2)));
        assert_eq!(ymd(p.from.unwrap(), &Utc), Some((2026, 10, 1)));
        assert_eq!(
            ymd(p.to.unwrap(), &Utc),
            Some((2026, 10, 31)),
            "the last ms of the day"
        );
        assert_eq!(
            p.chips,
            [
                (Key::From, "Sam Smith".into()),
                (Key::In, "game night".into())
            ]
        );
        assert!(p.unknown.is_empty());
        let p = parse("from:bob in:nowhere after:2026-02-30 FROM:alice x");
        assert_eq!(p.author_id.as_deref(), Some("a"), "the last key counts");
        assert_eq!(
            p.unknown,
            [
                "No room called nowhere.",
                "2026-02-30 is not a date. Use YYYY-MM-DD."
            ]
        );
        assert_eq!(parse("from: hi").q, "hi", "a key with no value is left out");
        assert_eq!(
            parse("after:2026-10-31 before:2026-10-01").unknown,
            ["The After date is later than the Before date."]
        );
        assert_eq!(
            ymd(parse("after:yesterday").from.unwrap(), &Utc),
            Some((2026, 10, 14))
        );
        assert_eq!(
            parse("a b c d e f g h i j k l m n").q.split(' ').count(),
            MAX_TERMS
        );
    }

    #[test]
    fn editing_keys() {
        assert_eq!(
            set_key("hi in:general there", Key::In, Some("game night")),
            r#"in:"game night" hi there"#
        );
        assert_eq!(set_key("in:general", Key::In, None), "");
        assert_eq!(set_key("", Key::From, Some("sam")), "from:sam ");
        assert_eq!(
            key_value("after:2026-10-01 after:2026-10-02", Key::After),
            "2026-10-02"
        );
        let at = key_at("hi from:al", 10).unwrap();
        assert_eq!((at.key, at.q.as_str(), at.start), (Key::From, "al", 3));
        assert_eq!(key_at("hi from:al", 2), None);
        assert_eq!(
            complete("hi from:al  x", &at, "alice"),
            ("hi from:alice x".into(), 14)
        );
        assert!(nothing_to_search("!! ?") && !nothing_to_search("a"));
        assert!(short_last("hello w") && !short_last("hello wo") && short_last("x"));
    }

    #[test]
    fn snippet_marks() {
        assert_eq!(
            marks("  say \u{2}hello\u{3}\n\n  world  "),
            [
                ("say ".into(), false),
                ("hello".into(), true),
                (" world".into(), false)
            ]
        );
        assert!(marks("   ").is_empty());
    }
}
