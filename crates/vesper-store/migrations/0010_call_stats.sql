-- Your recent calls (issue #103). Additive only. One row per call an account left, sent by that
-- account's own client and shown only to it. The server keeps the newest 20 per account and none
-- older than 30 days. No room and no other people: only how the call went for you.
CREATE TABLE call_stats (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('deepfilter', 'browser', 'off')),
    fallback_reason TEXT NOT NULL CHECK (length(fallback_reason) <= 120),
    loss_pct REAL NULL CHECK (loss_pct BETWEEN 0 AND 100),
    jitter_ms REAL NULL CHECK (jitter_ms BETWEEN 0 AND 10000),
    concealed_pct REAL NULL CHECK (concealed_pct BETWEEN 0 AND 100),
    rtt_ms REAL NULL CHECK (rtt_ms BETWEEN 0 AND 60000),
    browser_ns_loading INTEGER NOT NULL CHECK (browser_ns_loading IN (0, 1))
) STRICT;

CREATE INDEX call_stats_account ON call_stats (account_id, id);
CREATE INDEX call_stats_ended ON call_stats (ended_at);
