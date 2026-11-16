//! The Ctrl+K switcher's matching and ordering: a copy of `web/src/lib/ui/switcher.ts` (and `nextUnread` of
//! `keys.ts`); keep them in step. Plain functions, so the order is tested without a window.

use std::collections::HashMap;

use vesper_protocol::Unread;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Room,
    Person,
    Action,
}

/// A row. `id` is the room id, the user id, or the action's name (`settings-voice`). `hint` is a person's "In voice
/// in ...", or an action's shortcut name (`mute`).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
}

pub const MAX_RESULTS: usize = 30;
pub const MAX_RECENT: usize = 10;
const KINDS: [Kind; 3] = [Kind::Room, Kind::Person, Kind::Action];

/// `r `, `p ` and `> ` typed first scope the query to one kind.
pub fn scope_of(q: &str) -> Option<Kind> {
    match q.get(..2).map(str::to_lowercase).as_deref() {
        Some("r ") => Some(Kind::Room),
        Some("p ") => Some(Kind::Person),
        Some("> ") => Some(Kind::Action),
        _ => None,
    }
}

fn is_word_start(l: &[char], i: usize) -> bool {
    i == 0 || !l[i - 1].is_alphanumeric()
}

fn index_of(l: &[char], q: &[char], from: usize) -> Option<usize> {
    (from..=l.len().checked_sub(q.len())?).find(|&i| l[i..i + q.len()] == *q)
}

/// The first place `q` occurs at a word start.
fn starts(l: &[char], q: &[char]) -> Option<usize> {
    let mut at = index_of(l, q, 0);
    while let Some(i) = at {
        if is_word_start(l, i) {
            return Some(i);
        }
        at = index_of(l, q, i + 1);
    }
    None
}

/// How well `query` matches `label`, higher is better, None for no match. Case-insensitive. A prefix beats a match
/// at a word start, which beats a match inside a word, which beats a scattered subsequence; within a band shorter
/// labels and earlier matches win. The subsequence ignores spaces in the query and rewards letters at word starts and
/// in runs.
pub fn score(query: &str, label: &str) -> Option<f64> {
    let q: Vec<char> = query.trim().to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0.);
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let len = l.len() as f64;
    match index_of(&l, &q, 0) {
        Some(0) => return Some(3000. - len),
        Some(at) => {
            return Some(match starts(&l, &q) {
                Some(s) => 2000. - s as f64 - len / 100.,
                None => 1000. - at as f64 - len / 100.,
            });
        }
        None => {}
    }
    // Best placement of each query letter: a letter scores 1, plus 3 at a word start, plus 2 right after the previous
    // letter.
    let want: Vec<char> = q.into_iter().filter(|c| !c.is_whitespace()).collect();
    let ws = |i| if is_word_start(&l, i) { 3. } else { 0. };
    let mut row: Vec<f64> = (0..l.len())
        .map(|i| {
            if l[i] == want[0] {
                1. + ws(i)
            } else {
                f64::NEG_INFINITY
            }
        })
        .collect();
    for &w in &want[1..] {
        let mut before = f64::NEG_INFINITY; // best of row[0 .. i-2]
        row = (0..l.len())
            .map(|i| {
                if i >= 2 {
                    before = before.max(row[i - 2]);
                }
                if l[i] != w || i == 0 {
                    return f64::NEG_INFINITY;
                }
                1. + ws(i) + before.max(row[i - 1] + 2.)
            })
            .collect();
    }
    let best = row.into_iter().fold(f64::NEG_INFINITY, f64::max);
    (best > f64::NEG_INFINITY).then(|| best - len / 100.)
}

/// The list for `q`, grouped by kind (rooms, people, actions; with a query, a group whose best match is higher comes
/// first), at most MAX_RESULTS. The order before matching, and the tie-break after it: rooms with mentions (most
/// first), then other unread rooms (most first), then `recent` (most recent first), then the order given.
pub fn rank(q: &str, items: &[Item], unread: &HashMap<i64, Unread>, recent: &[i64]) -> Vec<Item> {
    let scope = scope_of(q);
    let query = if scope.is_some() { &q[2..] } else { q };
    let prio = |it: &Item| -> i64 {
        if it.kind != Kind::Room {
            return 0;
        }
        let id = it.id.parse::<i64>().unwrap_or(0);
        match unread.get(&id) {
            Some(u) if u.mentions > 0 => 3_000_000 + i64::from(u.mentions),
            Some(u) if u.unread > 0 => 2_000_000 + i64::from(u.unread),
            _ => recent
                .iter()
                .position(|&r| r == id)
                .map_or(0, |r| 1_000_000 - r as i64),
        }
    };
    let kind_at = |k: Kind| KINDS.iter().position(|&x| x == k).unwrap_or(0);
    let mut scored: Vec<(usize, i64, f64, &Item)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| scope.is_none_or(|s| it.kind == s))
        .filter_map(|(i, it)| Some((i, prio(it), score(query, &it.label)?, it)))
        .collect();
    let has_query = !query.trim().is_empty();
    scored.sort_by(|a, b| {
        let first = if has_query {
            b.2.total_cmp(&a.2)
        } else {
            kind_at(a.3.kind).cmp(&kind_at(b.3.kind))
        };
        first.then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0))
    });
    scored.truncate(MAX_RESULTS);
    let mut order: Vec<Kind> = vec![];
    for x in &scored {
        if !order.contains(&x.3.kind) {
            order.push(x.3.kind);
        }
    }
    order
        .into_iter()
        .flat_map(|k| {
            scored
                .iter()
                .filter(move |x| x.3.kind == k)
                .map(|x| x.3.clone())
        })
        .collect()
}

/// `room` first in the recent list, which keeps MAX_RECENT.
pub fn push_recent(recent: &mut Vec<i64>, room: i64) {
    recent.retain(|&r| r != room);
    recent.insert(0, room);
    recent.truncate(MAX_RECENT);
}

/// The room `dir` steps from `current` in the sidebar order (no wrap), or None at either end.
pub fn step_room(ids: &[i64], current: Option<i64>, dir: isize) -> Option<i64> {
    match current.and_then(|c| ids.iter().position(|&i| i == c)) {
        None if dir > 0 => ids.first().copied(),
        None => ids.last().copied(),
        Some(i) => ids.get(i.checked_add_signed(dir)?).copied(),
    }
}

/// The next room with unread messages after `current` in the sidebar order, wrapping, or None when no other has any.
pub fn next_unread(
    ids: &[i64],
    current: Option<i64>,
    unread: &HashMap<i64, Unread>,
) -> Option<i64> {
    let n = ids.len();
    let i = current.and_then(|c| ids.iter().position(|&x| x == c));
    (1..=n)
        .map(|k| ids[(i.unwrap_or(n - 1) + k) % n])
        .find(|&id| Some(id) != current && unread.get(&id).is_some_and(|u| u.unread > 0))
}

#[cfg(test)]
mod tests {
    //! The cases of `web/src/lib/ui/switcher.test.ts` that the Rust copy can share.
    use super::*;

    fn item(kind: Kind, id: &str, label: &str) -> Item {
        Item {
            kind,
            id: id.into(),
            label: label.into(),
            hint: None,
        }
    }
    fn unread(room: i64, unread: i64, mentions: i64) -> (i64, Unread) {
        let u = serde_json::from_value(serde_json::json!({
            "room_id": room, "last_read_id": 0, "unread": unread, "mentions": mentions }))
        .unwrap();
        (room, u)
    }
    fn labels(v: &[Item]) -> Vec<&str> {
        v.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn prefix_beats_word_start_beats_inside_beats_subsequence() {
        let p = score("gen", "general").unwrap();
        let w = score("gen", "the general").unwrap();
        let i = score("gen", "regenerate").unwrap();
        let s = score("gnl", "general").unwrap();
        assert!(p > w && w > i && i > s, "{p} {w} {i} {s}");
        assert_eq!(score("xyz", "general"), None);
        assert_eq!(score("", "general"), Some(0.));
        assert!(score("GEN", "General").is_some(), "case-insensitive");
        assert!(
            score("g n", "general").is_some(),
            "spaces in a subsequence are ignored"
        );
        assert!(score("ab", "a").is_none());
    }

    #[test]
    fn ranking_groups_scopes_and_breaks_ties() {
        let items = [
            item(Kind::Room, "1", "general"),
            item(Kind::Room, "2", "music"),
            item(Kind::Room, "3", "games"),
            item(Kind::Person, "u", "Gemma"),
            item(Kind::Action, "settings-voice", "Settings: Voice"),
        ];
        let un: HashMap<i64, Unread> = [unread(2, 3, 0), unread(3, 1, 1)].into();
        // No query: by kind, mentions then unread then recent first.
        assert_eq!(
            labels(&rank("", &items, &un, &[1])),
            ["games", "music", "general", "Gemma", "Settings: Voice"]
        );
        assert_eq!(
            labels(&rank("", &items, &HashMap::new(), &[3, 1])),
            ["games", "general", "music", "Gemma", "Settings: Voice"]
        );
        // A query: the best match's group first.
        assert_eq!(
            labels(&rank("ge", &items, &un, &[])),
            ["Gemma", "general", "games", "Settings: Voice"]
        );
        assert_eq!(labels(&rank("p g", &items, &un, &[])), ["Gemma"]);
        assert_eq!(labels(&rank("> ", &items, &un, &[])), ["Settings: Voice"]);
        let many: Vec<Item> = (0..40)
            .map(|i| item(Kind::Room, &i.to_string(), &format!("r{i}")))
            .collect();
        assert_eq!(rank("", &many, &un, &[]).len(), MAX_RESULTS);
    }

    #[test]
    fn rooms_step_and_unread_wraps() {
        let ids = [1, 2, 3];
        assert_eq!(step_room(&ids, Some(2), 1), Some(3));
        assert_eq!(step_room(&ids, Some(3), 1), None);
        assert_eq!(step_room(&ids, Some(1), -1), None);
        assert_eq!(step_room(&ids, None, -1), Some(3));
        let un: HashMap<i64, Unread> = [unread(1, 2, 0)].into();
        assert_eq!(next_unread(&ids, Some(2), &un), Some(1));
        assert_eq!(next_unread(&ids, Some(1), &un), None);
        assert_eq!(next_unread(&ids, None, &un), Some(1));
        let mut r = vec![];
        for x in [1, 2, 1] {
            push_recent(&mut r, x);
        }
        assert_eq!(r, [1, 2]);
    }
}
