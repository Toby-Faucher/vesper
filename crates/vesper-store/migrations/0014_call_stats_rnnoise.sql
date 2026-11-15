-- RNNoise as a call mode. Only widens the mode check: every row is copied as it is. SQLite cannot change a CHECK in
-- place, so the table is rebuilt (nothing references it).
CREATE TABLE call_stats_new (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('deepfilter', 'rnnoise', 'browser', 'off')),
    fallback_reason TEXT NOT NULL CHECK (length(fallback_reason) <= 120),
    loss_pct REAL NULL CHECK (loss_pct BETWEEN 0 AND 100),
    jitter_ms REAL NULL CHECK (jitter_ms BETWEEN 0 AND 10000),
    concealed_pct REAL NULL CHECK (concealed_pct BETWEEN 0 AND 100),
    rtt_ms REAL NULL CHECK (rtt_ms BETWEEN 0 AND 60000),
    browser_ns_loading INTEGER NOT NULL CHECK (browser_ns_loading IN (0, 1))
) STRICT;

INSERT INTO call_stats_new SELECT * FROM call_stats;
DROP TABLE call_stats;
ALTER TABLE call_stats_new RENAME TO call_stats;

CREATE INDEX call_stats_account ON call_stats (account_id, id);
CREATE INDEX call_stats_ended ON call_stats (ended_at);
