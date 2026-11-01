# 0004. No Content-Security-Policy yet

Status: accepted, revisit before accounts, 2026-10-30

Context: The design calls for a strict CSP, HttpOnly SameSite cookies and CSRF protection. Today the server serves a static app, holds no cookies and renders no user content.

Decision: Ship v1 voice without a CSP header. Add a strict one in the accounts plan, before any user-supplied text or upload is rendered.

Why: A CSP needs to know the final set of scripts, workers, WASM and connect targets. The AudioWorklet, the DSP worker and `wasm-unsafe-eval` are still changing. A policy written now would be rewritten.

Consequences: An XSS bug today could act as the signed-in user (the session cookie is HttpOnly, so it cannot read the cookie itself). (Before [0007](0007-accounts-and-sessions.md) it could read the shared join key.) The attack surface is small (no user HTML), but the gap is real. Set a deadline: CSP lands with the first feature that stores or shows user text. It must allow `worker-src 'self'`, `script-src 'wasm-unsafe-eval'` and the WebSocket origin.
