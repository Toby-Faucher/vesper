# 0005. SQLite with sqlx for accounts and text chat

Status: accepted, built in 2026-11-01 plan (confirmed by Toby, 2026-10-30)

Context: Accounts, rooms, messages and uploads need storage for about ten users on one box. Postgres would add a second service to run and back up.

Decision: SQLite in WAL mode, accessed through `sqlx` with plain SQL and migrations. Full-text search uses FTS5. Data lives in one directory (database file and uploads).

Why: One directory means a Proxmox snapshot is a complete backup, and WAL keeps the file crash-consistent under snapshots. A periodic `.backup` copy is a cheap extra. Plain SQL keeps the schema readable.

Consequences: One writer at a time, fine at this size. Moving to Postgres later is a rewrite of queries, not of the app. Keep storage behind its own crate so the server crate does not import sqlx directly. `sqlx` compile-time checks need a prepared query cache committed or `DATABASE_URL` in CI.
