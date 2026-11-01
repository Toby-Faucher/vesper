//! Full-text search over `messages_fts`. User text never reaches FTS5 syntax: [`fts_query`] turns
//! it into quoted terms. Room visibility and every filter are bound parameters inside the one SQL
//! statement, so a page is never cut short or padded by rows the caller may not see.

use std::collections::HashMap;

use vesper_protocol::{SearchHit, SearchPage, SearchQuery};

use crate::messages::{BY_IDS, MsgRow, json_ids, load};
use crate::{Result, Store};

const MAX_TERMS: usize = 16;
const MAX_TERM_CHARS: usize = 64;
/// Most hits in one page.
pub const MAX_SEARCH: u32 = 50;
const HIT: char = '\u{2}';
const HIT_END: char = '\u{3}';

/// Quoted FTS5 terms, the last one a prefix, or `None` when nothing searchable is left. Splits on
/// whitespace and control characters (a NUL must not end the FTS5 parse early), cuts each term to
/// 64 chars, drops terms with no letter or digit and keeps 16. A last term of one character is
/// matched as a whole word, not a prefix: `a*` merges the doclist of every word starting with `a`,
/// the slow case.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| c.is_whitespace() || c.is_control())
        .map(|t| t.chars().take(MAX_TERM_CHARS).collect::<String>())
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .take(MAX_TERMS)
        .collect();
    let prefix = terms.last()?.chars().count() >= MIN_PREFIX_CHARS;
    let quoted: Vec<String> = terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    Some(quoted.join(" ") + if prefix { "*" } else { "" })
}

/// Shortest last term that is searched as a prefix.
const MIN_PREFIX_CHARS: usize = 2;

/// Newest first, so FTS5 walks its doclist from the top and stops after one page (bm25 order
/// would score every match first: 57 to 80 ms at 100k messages). With no sort, the snippet is
/// only built for the rows returned. `?7` is the id cursor.
#[doc(hidden)]
pub const HITS: &str = "SELECT m.id, snippet(messages_fts, 0, char(2), char(3), '...', 12)
    FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid
    WHERE messages_fts MATCH ?1
      AND m.room_id IN (SELECT value FROM json_each(?2))
      AND m.deleted_at IS NULL
      AND (?3 IS NULL OR m.room_id = ?3)
      AND (?4 IS NULL OR m.author_id = ?4)
      AND (?5 IS NULL OR m.created_at >= ?5)
      AND (?6 IS NULL OR m.created_at <= ?6)
      AND (?7 IS NULL OR messages_fts.rowid < ?7)
    ORDER BY messages_fts.rowid DESC
    LIMIT ?8";

impl Store {
    /// Messages matching `q.q` in `visible_rooms` (the caller decides those), newest first.
    /// `limit` is clamped to 1 to 50. Deleted messages are not in the index.
    pub async fn search(
        &self,
        q: &SearchQuery,
        visible_rooms: &[i64],
        limit: u32,
    ) -> Result<SearchPage> {
        let empty = SearchPage {
            hits: Vec::new(),
            next_before: None,
        };
        let Some(expr) = fts_query(&q.q) else {
            return Ok(empty);
        };
        if visible_rooms.is_empty() {
            return Ok(empty);
        }
        let limit = limit.clamp(1, MAX_SEARCH);
        // One read transaction: both statements see the same snapshot.
        let mut tx = self.pool.begin().await?;
        let mut found: Vec<(i64, String)> = sqlx::query_as(HITS)
            .bind(&expr)
            .bind(json_ids(visible_rooms.iter().copied()))
            .bind(q.room_id)
            .bind(q.author_id.as_deref())
            .bind(q.from)
            .bind(q.to)
            .bind(q.before)
            .bind(limit + 1)
            .fetch_all(&mut *tx)
            .await?;
        let more = found.len() > limit as usize;
        found.truncate(limit as usize);
        let rows: Vec<MsgRow> = sqlx::query_as(BY_IDS)
            .bind(json_ids(found.iter().map(|f| f.0)))
            .fetch_all(&mut *tx)
            .await?;
        let mut messages: HashMap<i64, _> = load(&mut tx, rows)
            .await?
            .into_iter()
            .map(|m| (m.id, m))
            .collect();
        tx.commit().await?;
        let next_before = found.last().map(|f| f.0).filter(|_| more);
        let hits = found
            .into_iter()
            .filter_map(|(id, mut snippet)| {
                let message = messages.remove(&id)?;
                // Markers already in the body would fake a highlight: drop all of them then.
                if message.body.contains([HIT, HIT_END]) {
                    snippet.retain(|c| c != HIT && c != HIT_END);
                }
                Some(SearchHit { message, snippet })
            })
            .collect();
        Ok(SearchPage { hits, next_before })
    }
}
