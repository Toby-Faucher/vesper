# Security

Vesper is a personal project run for one friend group. There is no bug bounty and no support contract.

## Reporting a problem

Report a vulnerability privately, to the repository owner directly. If the repository has GitHub private reporting turned on, use the "Report a vulnerability" button on the Security tab.
Please do not open a public issue for something exploitable. You will get an answer when the owner can read it, usually within a week.

## What to know

- Access is by account. Accounts come from invite links, passwords are hashed with argon2id, and the session cookie holds a random
  token whose hash is stored. See [decision 0007](docs/decisions/0007-accounts-and-sessions.md).
- Put HTTPS in front: the session cookie travels with every request. [docs/deploy.md](docs/deploy.md) shows a Caddy setup.
- The server sends a strict Content-Security-Policy and the other security headers itself ([decision 0008](docs/decisions/0008-csp.md)).
- Known gaps are listed in [docs/known-issues.md](docs/known-issues.md).
- Audio is forwarded by the server without decoding. It is encrypted between each browser and the server (WebRTC DTLS-SRTP), not end to end.
- Never commit session tokens, invite links, `.env` files, keys or certificates. `scripts/check-secrets.sh` looks for them in tracked files, and CI runs it.
- Dependencies are checked weekly with `cargo deny` and `npm audit` (`.github/workflows/advisories.yml`).
