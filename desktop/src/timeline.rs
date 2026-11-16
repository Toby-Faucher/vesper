//! The pure parts of the web's timeline and composer (`web/src/lib/ui/timeline.ts`, `switcher.ts`, `attachments.ts`):
//! day rows and grouping, the clock in the reader's locale, which messages rise in, the `@` mention being typed and who
//! to offer, the typing line, file sizes.

use std::sync::OnceLock;

use chrono::{Days, Local, TimeZone, Timelike};
use vesper_protocol::{Message, RoomInfo, UserInfo};

/// How the reader's locale writes times and days, as the browser's `Intl` does for the web app: "7:41 PM" and
/// "Mon Oct 27" in en-US, "19:41" and "Mon 27 Oct" in most others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    pub hour12: bool,
    pub month_first: bool,
}

impl Locale {
    /// From a BCP 47 tag ("en-US", "de-DE", "en").
    /// shortcut: a short table of English locales only; other languages that write 12-hour times (Korean, Arabic) get
    /// 24 hours, since their AM/PM words would need a translation table. Grow it when a reader asks.
    pub fn from_tag(tag: &str) -> Locale {
        let mut parts = tag.split(['-', '_']);
        let lang = parts.next().unwrap_or("").to_ascii_lowercase();
        let region = parts
            .find(|p| p.len() == 2)
            .map(str::to_ascii_uppercase)
            .unwrap_or_default();
        let en = lang == "en";
        Locale {
            hour12: en
                && matches!(
                    region.as_str(),
                    "" | "US" | "CA" | "AU" | "NZ" | "IN" | "PH" | "PK" | "BD" | "EG" | "SA" | "MY"
                ),
            month_first: en && matches!(region.as_str(), "" | "US" | "CA" | "PH"),
        }
    }

    /// The OS's locale (the browser's, for the web app), read once.
    pub fn system() -> Locale {
        static L: OnceLock<Locale> = OnceLock::new();
        *L.get_or_init(|| Locale::from_tag(&sys_locale::get_locale().unwrap_or_default()))
    }
}

/// Every clock time reads this (hour, minute) when set: the parity harness pins 7:41 PM like the web scenes
/// (`VESPER_DEV_CLOCK`, debug builds only). Dates are left alone.
pub static FIXED_CLOCK: OnceLock<(u32, u32)> = OnceLock::new();

/// A message's time as the head row shows it: "7:41 PM" (en-US) or "19:41". `short` leaves out AM and PM, for the
/// gutter of a grouped row ("7:41").
pub fn clock(ms: i64, loc: Locale, short: bool) -> String {
    let (h, m) = match FIXED_CLOCK.get() {
        Some(&hm) => hm,
        None => match Local.timestamp_millis_opt(ms).single() {
            Some(t) => (t.hour(), t.minute()),
            None => return String::new(),
        },
    };
    hm(h, m, loc, short)
}

/// `h`:`m` as [`clock`] writes it.
pub fn hm(h: u32, m: u32, loc: Locale, short: bool) -> String {
    if !loc.hour12 {
        return format!("{h:02}:{m:02}");
    }
    let h12 = if h.is_multiple_of(12) { 12 } else { h % 12 };
    let pm = if h < 12 { "AM" } else { "PM" };
    if short {
        format!("{h12}:{m:02}")
    } else {
        format!("{h12}:{m:02} {pm}")
    }
}

/// The messages that rise in after the list changed (`arrivals` in timeline.ts): live arrivals only, three at most.
/// None on a room open (`since` is None), none on an older page (nothing newer than `since`), none for more than three.
pub fn arrivals(since: Option<i64>, ids: &[i64]) -> Vec<i64> {
    let Some(since) = since else { return vec![] };
    let fresh: Vec<i64> = ids.iter().copied().filter(|&id| id > since).collect();
    if fresh.len() <= 3 { fresh } else { vec![] }
}

/// Bodies above about 20 lines collapse behind "Show more" (`isLong`).
pub fn is_long(body: &str) -> bool {
    body.matches('\n').count() > 20 || body.chars().count() > 2000
}

const GROUP_MS: i64 = 5 * 60_000;

/// How one message row starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowHead {
    /// The day row drawn above it ("Today"), on the first message of each local day.
    pub day: Option<String>,
    /// It heads a group: the name and time are shown.
    pub head: bool,
    /// The New row is drawn above it.
    pub new: bool,
}

/// "Today", "Yesterday", "Mon 27 Oct", or "Wed 31 Dec 2025" for another year ("Mon Oct 27" month first).
pub fn day_label<Tz: TimeZone>(ms: i64, now_ms: i64, tz: &Tz, loc: Locale) -> String {
    let (Some(d), Some(now)) = (
        tz.timestamp_millis_opt(ms).single(),
        tz.timestamp_millis_opt(now_ms).single(),
    ) else {
        return String::new();
    };
    let (d, today) = (d.date_naive(), now.date_naive());
    if d == today {
        "Today".into()
    } else if today.checked_sub_days(Days::new(1)) == Some(d) {
        "Yesterday".into()
    } else {
        let day = if loc.month_first {
            "%a %b %-d"
        } else {
            "%a %-d %b"
        };
        let same_year = d.format("%Y").to_string() == today.format("%Y").to_string();
        d.format(&if same_year {
            day.to_string()
        } else {
            format!("{day} %Y")
        })
        .to_string()
    }
}

/// One `RowHead` per message, like the web's `rows`: a day row before the first message of each local day; with
/// `new_after` (a read marker and my id), the New row before the first message after it that is not mine. A message
/// heads a group unless the one before it has the same author, is less than 5 minutes older, and neither is deleted.
/// A reply, a new day and the New row always start a group.
pub fn heads<Tz: TimeZone>(
    messages: &[Message],
    now_ms: i64,
    tz: &Tz,
    loc: Locale,
    new_after: Option<(i64, &str)>,
) -> Vec<RowHead> {
    let day_of = |ms: i64| tz.timestamp_millis_opt(ms).single().map(|t| t.date_naive());
    let mut prev: Option<&Message> = None;
    let mut new_done = new_after.is_none();
    messages
        .iter()
        .map(|m| {
            let mut head = prev.is_none_or(|p| {
                p.author_id != m.author_id
                    || m.created_at - p.created_at >= GROUP_MS
                    || p.deleted
                    || m.deleted
                    || m.reply_to.is_some()
            });
            let day = (prev.is_none_or(|p| day_of(p.created_at) != day_of(m.created_at)))
                .then(|| day_label(m.created_at, now_ms, tz, loc));
            head |= day.is_some();
            let new = !new_done && new_after.is_some_and(|(at, me)| m.id > at && m.author_id != me);
            new_done |= new;
            head |= new;
            prev = Some(m);
            RowHead { day, head, new }
        })
        .collect()
}

/// The `@` mention being typed before byte `caret`: where the `@` is and the text after it. None when the `@` does
/// not start a word (as in an email address), a space comes after it, or it is inside code.
pub fn mention_query(text: &str, caret: usize) -> Option<(usize, String)> {
    let before = text.get(..caret)?;
    let at = before.rfind('@')?;
    let q = &before[at + 1..];
    if q.chars().count() > 32 || q.chars().any(|c| c.is_whitespace() || c == '@') {
        return None;
    }
    if let Some(p) = before[..at].chars().next_back()
        && !(p.is_whitespace() || "([{\"'>".contains(p))
    {
        return None;
    }
    let lines: Vec<&str> = before[..at].split('\n').collect();
    let (last, done) = lines.split_last()?;
    let fences = done.iter().filter(|l| l.starts_with("```")).count();
    if fences % 2 == 1 || last.matches('`').count() % 2 == 1 {
        return None;
    }
    Some((at, q.into()))
}

fn word_start(l: &[char], i: usize) -> bool {
    i == 0 || !l[i - 1].is_alphanumeric()
}

fn find(l: &[char], q: &[char], from: usize) -> Option<usize> {
    (from..(l.len() + 1).saturating_sub(q.len())).find(|&i| l[i..i + q.len()] == *q)
}

/// How well `query` matches `label`, higher is better, None for no match, like the web switcher's `score`. A prefix
/// beats a match at a word start, which beats one inside a word, which beats a scattered subsequence.
pub fn score(query: &str, label: &str) -> Option<f64> {
    let q: Vec<char> = query.trim().to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0.0);
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let len = l.len() as f64;
    if let Some(at) = find(&l, &q, 0) {
        if at == 0 {
            return Some(3000.0 - len);
        }
        let mut i = Some(at);
        while let Some(x) = i {
            if word_start(&l, x) {
                return Some(2000.0 - x as f64 - len / 100.0);
            }
            i = find(&l, &q, x + 1);
        }
        return Some(1000.0 - at as f64 - len / 100.0);
    }
    // The best placement of each letter: 1, plus 3 at a word start, plus 2 right after the previous letter.
    let want: Vec<char> = q.into_iter().filter(|c| !c.is_whitespace()).collect();
    let bonus = |i: usize| 1.0 + if word_start(&l, i) { 3.0 } else { 0.0 };
    let mut row: Vec<f64> = (0..l.len())
        .map(|i| {
            if l[i] == want[0] {
                bonus(i)
            } else {
                f64::NEG_INFINITY
            }
        })
        .collect();
    for &w in &want[1..] {
        let mut before = f64::NEG_INFINITY;
        row = (0..l.len())
            .map(|i| {
                if i >= 2 {
                    before = before.max(row[i - 2]);
                }
                if l[i] != w || i == 0 {
                    return f64::NEG_INFINITY;
                }
                bonus(i) + before.max(row[i - 1] + 2.0)
            })
            .collect();
    }
    let best = row.into_iter().fold(f64::NEG_INFINITY, f64::max);
    (best > f64::NEG_INFINITY).then(|| best - len / 100.0)
}

/// Whether someone can read the room: anyone for an open room; the owner, moderators and members for a restricted one.
pub fn can_see(u: &UserInfo, room: &RoomInfo) -> bool {
    use vesper_protocol::Role;
    !room.restricted
        || matches!(u.role, Role::Owner | Role::Moderator)
        || room.members.contains(&u.user_id)
}

/// People to offer for `@q` in `room`, best match first: only those who can see it, not banned, not me. At most 8.
pub fn mentionable<'a>(
    q: &str,
    users: impl IntoIterator<Item = &'a UserInfo>,
    room: &RoomInfo,
    my_id: &str,
) -> Vec<&'a UserInfo> {
    let mut out: Vec<(&UserInfo, f64)> = users
        .into_iter()
        .filter(|u| !u.banned && u.user_id != my_id && can_see(u, room))
        .filter_map(|u| {
            let s = [score(q, &u.username), score(q, &u.display_name)]
                .into_iter()
                .flatten()
                .reduce(f64::max)?;
            Some((u, s))
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.display_name.cmp(&b.0.display_name))
    });
    out.into_iter().take(8).map(|(u, _)| u).collect()
}

/// "Sam is typing", "Sam and Alex are typing", "Several people are typing", or empty for nobody.
pub fn typing_line(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => format!("{a} is typing"),
        [a, b] => format!("{a} and {b} are typing"),
        _ => "Several people are typing".into(),
    }
}

/// `namesOnly` in attachments.ts: `body` is only these file names joined by ", " in any order, as messages of files were
/// sent before the body could be empty. Such a body is not shown. Each distinct name is tried once.
/// shortcut: no NFC step (the web has one); a name stored in another normal form shows its old body. Upgrade if seen.
pub fn names_only(body: &str, names: &[&str]) -> bool {
    fn rest(s: &str, left: &[&str]) -> bool {
        if left.is_empty() {
            return s.is_empty();
        }
        left.iter().enumerate().any(|(i, n)| {
            if left[..i].contains(n) || !s.starts_with(n) {
                return false;
            }
            let after = &s[n.len()..];
            if left.len() == 1 {
                return after.is_empty();
            }
            let mut others = left.to_vec();
            others.remove(i);
            after.strip_prefix(", ").is_some_and(|a| rest(a, &others))
        })
    }
    !names.is_empty() && rest(body, names)
}

/// "12 KB", "3.4 MB": binary units, as the server counts its upload limit, rounded up.
pub fn file_size(n: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if n < MB {
        format!("{} KB", n.div_ceil(1024).max(1))
    } else {
        format!("{:.1} MB", (n as f64 / MB as f64 * 10.0).ceil() / 10.0)
    }
}

/// The file card's type label (`typeLabel`): images by the type the server found, any other file by the extension
/// of its name ("TXT file"), or empty for a name with no short extension.
pub fn type_label(mime: &str, name: &str) -> String {
    match mime {
        "image/png" => return "PNG image".into(),
        "image/jpeg" => return "JPEG image".into(),
        "image/gif" => return "GIF image".into(),
        "image/webp" => return "WebP image".into(),
        _ => {}
    }
    match name.rsplit_once('.') {
        Some((_, ext))
            if (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!("{} file", ext.to_ascii_uppercase())
        }
        _ => String::new(),
    }
}

/// The largest box an image takes in the timeline.
pub const BOX: (u32, u32) = (360, 280);

/// The box for an image of `w` x `h`: scaled down to fit `BOX`, never up, at least 48 on each side.
pub fn fit(w: u32, h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return BOX;
    }
    let s = (BOX.0 as f64 / w as f64)
        .min(BOX.1 as f64 / h as f64)
        .min(1.0);
    let side = |v: u32| ((v as f64 * s).round() as u32).max(48);
    (side(w), side(h))
}

#[cfg(test)]
mod tests {

    #[test]
    fn names_only_any_order() {
        assert!(names_only("a.mp3, b, c.mp3", &["b, c.mp3", "a.mp3"]));
        assert!(names_only("a.mp3, b, c.mp3", &["c.mp3", "a.mp3", "b"]));
        assert!(names_only("x, x, x", &["x", "x", "x"]));
        assert!(!names_only("x, x", &["x", "x", "x"]));
        assert!(!names_only("see a.mp3", &["a.mp3"]));
        assert!(!names_only("", &[]));
    }
    use super::*;
    use crate::state::tests::{msg, room, user};
    use chrono::{FixedOffset, Utc};
    use vesper_protocol::{ReplyPreview, Role, RoomKind};

    // 2026-11-14 12:00 UTC.
    const NOW: i64 = 1_794_657_600_000;
    const MIN: i64 = 60_000;
    const DAY: i64 = 24 * 60 * MIN;

    fn at(id: i64, author: &str, ms: i64) -> Message {
        Message {
            created_at: ms,
            ..msg(id, 1, author, "x")
        }
    }

    const GB: Locale = Locale {
        hour12: false,
        month_first: false,
    };

    #[test]
    fn day_labels() {
        assert_eq!(day_label(NOW, NOW, &Utc, GB), "Today");
        assert_eq!(day_label(NOW - DAY, NOW, &Utc, GB), "Yesterday");
        assert_eq!(day_label(NOW - 2 * DAY, NOW, &Utc, GB), "Thu 12 Nov");
        assert_eq!(day_label(NOW - 330 * DAY, NOW, &Utc, GB), "Fri 19 Dec 2025");
        let us = Locale::from_tag("en-US");
        assert_eq!(day_label(NOW - 2 * DAY, NOW, &Utc, us), "Thu Nov 12");
        assert_eq!(day_label(NOW - 330 * DAY, NOW, &Utc, us), "Fri Dec 19 2025");
        // 23:30 UTC is already tomorrow at +01:00.
        let plus1 = FixedOffset::east_opt(3600).unwrap();
        assert_eq!(
            day_label(NOW + 11 * 60 * MIN + 30 * MIN, NOW, &plus1, GB),
            "Sun 15 Nov"
        );
    }

    #[test]
    fn locales_and_the_clock() {
        let us = Locale::from_tag("en-US");
        assert!(us.hour12 && us.month_first);
        assert_eq!(Locale::from_tag("en_US.UTF-8"), us);
        assert_eq!(Locale::from_tag("en"), us);
        assert_eq!(Locale::from_tag("en-GB"), GB);
        assert_eq!(Locale::from_tag("de-DE"), GB);
        assert_eq!(Locale::from_tag(""), GB);
        let au = Locale::from_tag("en-AU");
        assert!(au.hour12 && !au.month_first);
        // FIXED_CLOCK is never set in tests: the local time of these instants, whatever the zone.
        let t = Local
            .with_ymd_and_hms(2026, 11, 1, 19, 41, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(clock(t, us, false), "7:41 PM");
        assert_eq!(clock(t, us, true), "7:41");
        assert_eq!(clock(t, GB, false), "19:41");
        let t = Local
            .with_ymd_and_hms(2026, 11, 1, 0, 5, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(clock(t, us, false), "12:05 AM");
        assert_eq!(clock(t, GB, true), "00:05");
        let t = Local
            .with_ymd_and_hms(2026, 11, 1, 12, 5, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(clock(t, us, false), "12:05 PM");
    }

    #[test]
    fn which_messages_rise_in() {
        assert!(arrivals(None, &[1, 2]).is_empty(), "a room open");
        assert_eq!(arrivals(Some(5), &[3, 4, 5, 6]), [6], "live");
        assert!(
            arrivals(Some(5), &[1, 2, 3, 4, 5]).is_empty(),
            "an older page"
        );
        assert_eq!(arrivals(Some(5), &[6, 7, 8]), [6, 7, 8]);
        assert!(
            arrivals(Some(5), &[6, 7, 8, 9]).is_empty(),
            "a page, not live"
        );
        assert!(is_long(&"a\n".repeat(21)) && !is_long(&"a\n".repeat(20)));
        assert!(is_long(&"a".repeat(2001)));
    }

    #[test]
    fn grouping_and_day_rows() {
        let mut reply = at(5, "a", NOW - 9 * MIN);
        reply.reply_to = Some(ReplyPreview {
            id: 1,
            author_id: "b".into(),
            excerpt: "x".into(),
        });
        let mut deleted = at(7, "a", NOW - 7 * MIN);
        deleted.deleted = true;
        let list = [
            at(1, "a", NOW - DAY),      // yesterday: day row, head
            at(2, "a", NOW - 20 * MIN), // today: day row, head
            at(3, "a", NOW - 19 * MIN), // same author, a minute later: grouped
            at(4, "b", NOW - 18 * MIN), // another author: head
            reply,                      // a reply: head
            at(6, "a", NOW - 8 * MIN),  // grouped under the reply
            deleted,                    // deleted: head
            at(8, "a", NOW - 6 * MIN),  // after a deleted one: head
            at(9, "a", NOW),            // 6 minutes later: head
            at(10, "a", NOW + 1),       // grouped
        ];
        let h = heads(&list, NOW, &Utc, GB, None);
        let got: Vec<(bool, Option<&str>)> = h.iter().map(|r| (r.head, r.day.as_deref())).collect();
        assert_eq!(
            got,
            [
                (true, Some("Yesterday")),
                (true, Some("Today")),
                (false, None),
                (true, None),
                (true, None),
                (false, None),
                (true, None),
                (true, None),
                (true, None),
                (false, None),
            ]
        );
        // The New row goes before the first message after the marker that is not mine.
        let h = heads(&list, NOW, &Utc, GB, Some((5, "b")));
        let news: Vec<i64> = list
            .iter()
            .zip(&h)
            .filter(|(_, r)| r.new)
            .map(|(m, _)| m.id)
            .collect();
        assert_eq!(news, [6]);
        assert!(h[5].head, "the New row starts a group");
        let h = heads(&list, NOW, &Utc, GB, Some((5, "a")));
        assert!(h.iter().all(|r| !r.new), "all mine after it");
    }

    #[test]
    fn the_mention_being_typed() {
        let q = |t: &str| mention_query(t, t.len());
        assert_eq!(q("hi @bo"), Some((3, "bo".into())));
        assert_eq!(q("@"), Some((0, String::new())));
        assert_eq!(q("(@al"), Some((1, "al".into())));
        assert_eq!(q("mail a@b"), None, "inside a word");
        assert_eq!(q("@bob "), None, "a space ends it");
        assert_eq!(q("`@bo"), None, "inline code");
        assert_eq!(q("```\n@bo"), None, "a code block");
        assert_eq!(q("```\nx\n```\n@bo"), Some((10, "bo".into())));
        assert_eq!(q(&format!("@{}", "a".repeat(33))), None);
        assert_eq!(mention_query("@bob and more", 4), Some((0, "bob".into())));
    }

    #[test]
    fn who_to_offer_for_a_mention() {
        let mut banned = user("d", "Dora");
        banned.banned = true;
        let mut modr = user("m", "Mona");
        modr.role = Role::Moderator;
        let users = [
            user("a", "Ann"),
            user("b", "Bob"),
            user("bb", "Bobby"),
            user("c", "Abba"),
            banned,
            modr,
        ];
        let open = room(1, "general", RoomKind::Room, &[]);
        let names = |q: &str, r: &RoomInfo| {
            mentionable(q, &users, r, "a")
                .iter()
                .map(|u| u.display_name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("bo", &open),
            ["Bob", "Bobby"],
            "prefix, shorter first"
        );
        assert_eq!(names("b", &open), ["Bob", "Bobby", "Abba"]);
        assert_eq!(names("ann", &open), Vec::<String>::new(), "not me");
        assert_eq!(names("dor", &open), Vec::<String>::new(), "not banned");
        assert_eq!(names("", &open).len(), 4);
        let mut restricted = room(2, "secret", RoomKind::Room, &["a", "b"]);
        restricted.restricted = true;
        assert_eq!(
            names("", &restricted),
            ["Bob", "Mona"],
            "members and moderators"
        );
        assert!(score("bb", "Bobby").is_some(), "inside a word");
        assert!(score("bby", "Bob by").unwrap() < score("bby", "bobby").unwrap());
        assert_eq!(score("xyz", "Bob"), None);
    }

    #[test]
    fn typing_lines_and_sizes() {
        let n = |v: &[&str]| typing_line(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(n(&[]), "");
        assert_eq!(n(&["Sam"]), "Sam is typing");
        assert_eq!(n(&["Sam", "Alex"]), "Sam and Alex are typing");
        assert_eq!(n(&["Sam", "Alex", "Jo"]), "Several people are typing");
        assert_eq!(file_size(1), "1 KB");
        assert_eq!(file_size(1025), "2 KB");
        assert_eq!(file_size(3 * 1024 * 1024 + 1), "3.1 MB");
        assert_eq!(fit(720, 560), (360, 280));
        assert_eq!(fit(100, 50), (100, 50));
        assert_eq!(fit(4000, 10), (360, 48));
        assert_eq!(type_label("image/png", "a.png"), "PNG image");
        assert_eq!(
            type_label("application/octet-stream", "route-notes.pdf"),
            "PDF file"
        );
        assert_eq!(type_label("application/octet-stream", "README"), "");
        assert_eq!(type_label("application/octet-stream", "a.toolong"), "");
    }
}
