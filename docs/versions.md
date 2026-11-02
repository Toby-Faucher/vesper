# Versions and releases

Vesper uses `vMAJOR.MINOR.PATCH`. One number covers the whole app: the server, the web app and the wire protocol ship together.

| Part | Meaning |
|---|---|
| MAJOR | A change that breaks people or data: a database migration an older image cannot read, a changed wire message, a removed setting. Needs a backup and a note on how to roll back. |
| MINOR | New features and visible changes. Migrations must only add things. |
| PATCH | Bug fixes and docs only. No migration. |

## Where we are

- **v1.0**: accounts, invites, rooms, voice presence, text chat, search, uploads and the new UI (PR #18, merged and deployed on 2026-11-01).
- **v1.1**: the first round of critique issues: logo, markdown with a live composer preview, profile pictures and bios, call sounds, a noise gate and soft limiter, motion with anime.js, visual fixes (milestone `v1.1`, PRs #32 to #42). It adds one database migration (0005, additive).
- **v1.1.1**: fixes silence after someone leaves and rejoins a call in Firefox (issue #45, PR #46). No migration.
- **v1.2**: click a name to open a profile popup, a choice of accent colour per account, smoother motion (message highlight, popups, the sliding sidebar). It adds one database migration (0006, additive).
- **v1.2.1**: join no longer fails when the saved microphone is gone (issue #68); the logo and tab icon follow the accent colour (issue #69). No migration.
- **v1.2.2**: reliability and CI: a person card removed while open no longer throws, flaky end-to-end steps fixed, the end-to-end harness cleans up its servers, and a lighter CI pipeline for version-bump pull requests. No migration.
- **v1.3**: account recovery without email (owner-issued reset links and `vesper-server reset-password`), the `vesper-server backup` command, archiving a room, force-mute enforced by the server, a connection quality icon per person and an opt-in "Even out voices" switch, and Web Push for mentions and "started voice" with an install manifest. Adds migrations 0007 to 0009. An older image cannot start on the migrated database: roll back with the backup taken before the upgrade.
- **v1.3.1**: fixes: a search hit inside a link, a second logo on the welcome page, tooltips that kept an old position, a Firefox error when the open room was archived or made private (#82), flaky end-to-end tests, and connection timeouts on the server (header, idle and body). Needs a one-line Caddy change on existing deployments (see deploy.md). No migration.
- **v1.4**: open tabs see "Vesper was updated" and a Reload button after an upgrade (the version handshake), an automatic backup before each migration, a Server page in Admin, unread mentions on the installed app's badge, precompressed web assets, a smaller denoiser download and a burst-loss lab (RED was measured and does not help), less JavaScript on first load, the browser's noise suppression while the denoiser loads, a Retry microphone button, a short fade when the audio runs dry, and your own call history in Settings. Adds migration 0010. An older image cannot start on the migrated database: roll back with the backup taken before the migration.
- **v1.4.1**: fixes: nested lists in messages (#36), the person card no longer covers the composer when it grows (#86), two flaky signaling tests made deterministic (#95, #113), a microphone settings row in the diagnostics report, a fail-safe when a swapped microphone stays silent, and a clearer message when the microphone test cannot play through the speaker. No migration.

## How a release is cut

1. Work lands on `main` by pull request. CI must be green. Nothing is merged to `main` without a pull request.
2. When the milestone's issues are done, open a pull request named `Release vX.Y.Z` that sets `version` in `Cargo.toml` (`[workspace.package]`) and `web/package.json` to the same number, and updates any doc that names the version. Run `scripts/check.sh` and `cd web && npm run e2e` first. CI gives a pull request that only changes versions the light pipeline (no `e2e` or `dsp-wasm` job, see the CI section of [dev.md](dev.md#ci)), so the local e2e run is the one that counts.
3. After it is merged, tag the merge commit: `git tag -a vX.Y.Z -m "Vesper vX.Y.Z" && git push origin vX.Y.Z`.
4. Create the GitHub release from the tag (`gh release create vX.Y.Z --generate-notes`). The notes are the merged pull requests and the issues they close. There is no CHANGELOG file.
5. Deploy from the tag, never from a branch: `git archive vX.Y.Z` into the stack, then follow [deploy.md](deploy.md). Before it, tag the running image (`docker tag vesper:latest vesper:previous`). If the release adds a migration, the server backs the database up to `data/backups/pre-schema-<NNNN>-<UTC time>.db` (`<NNNN>`: the schema the copy holds) before it migrates (from v1.4); a manual `vesper-server backup` as well does no harm.
6. Check `/health`, sign in, and join a call from a second device. Say in the release notes what was not verified.

## Rolling back

Use `vesper:previous` and the compose file of the old tag (see "Upgrade" in [deploy.md](deploy.md)). An older image refuses to start on a database a newer one migrated, even when the migrations only add (it stops with "migration N was previously applied but is missing"). So after a release with a migration, MINOR or MAJOR, roll back to the old image together with the backup taken before the upgrade: the `data/backups/pre-schema-<NNNN>-*.db` copy with the newest time among those whose `<NNNN>` is at most the old image's schema (written by the new image before it migrated; not simply the newest file, see [deploy.md](deploy.md#upgrade-and-rollback)), restored as `data/vesper.db` with the stack stopped ([deploy.md](deploy.md#upgrade-and-rollback)). Everything written after the upgrade is lost. Not verified on the live instance: tests start an old-schema database, and an upgrade that fails part way five times, and check the copy the recipe picks opens with the old migrations, but no older image was started on a restored copy.

Open tabs: every `hello` and `/api/me` carries the server's version and build id (`Me.server`). A tab that sees a build id other than the one of its first `hello` stops its socket, blocks sending and shows "Vesper was updated. Reload to continue." with a Reload button. It never reloads by itself, so a mismatch can not loop.

## Pre-release builds

Branch builds are not versions. A build from a pull request is for testing only and is never deployed to the live instance without an explicit decision.

## Not an agent's decision

The version number of a release, the license, the changelog and making the repository public are the owner's calls. An agent may prepare the release pull request and the tag, and waits for a go.
