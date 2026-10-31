# Security

Vesper is a personal project run for one friend group. There is no bug bounty and no support contract.

## Reporting a problem

Report a vulnerability privately, to the repository owner directly. If the repository has GitHub private reporting turned on, use the "Report a vulnerability" button on the Security tab.
Please do not open a public issue for something exploitable. You will get an answer when the owner can read it, usually within a week.

## What to know

- The only access control today is one shared join key (`VESPER_DEV_KEY`). Anyone who has it can join. See [decision 0003](docs/decisions/0003-shared-join-key.md).
- Put HTTPS in front. The key travels in the first WebSocket message. [docs/deploy.md](docs/deploy.md) shows a Caddy setup.
- There is no Content-Security-Policy yet ([decision 0004](docs/decisions/0004-no-csp-yet.md)).
- Known gaps are listed in [docs/known-issues.md](docs/known-issues.md).
- Audio is forwarded by the server without decoding. It is encrypted between each browser and the server (WebRTC DTLS-SRTP), not end to end.
- Never commit the join key, `.env` files, keys or certificates. `scripts/check-secrets.sh` looks for them in tracked files, and CI runs it.
- Dependencies are checked weekly with `cargo deny` and `npm audit` (`.github/workflows/advisories.yml`).
