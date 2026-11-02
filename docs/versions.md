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

## How a release is cut

1. Work lands on `main` by pull request. CI must be green. Nothing is merged to `main` without a pull request.
2. When the milestone's issues are done, open a pull request named `Release vX.Y.Z` that sets `version` in `Cargo.toml` (`[workspace.package]`) and `web/package.json` to the same number, and updates any doc that names the version. Run `scripts/check.sh` and `cd web && npm run e2e` first. CI gives a pull request that only changes versions the light pipeline (no `e2e` or `dsp-wasm` job, see the CI section of [dev.md](dev.md#ci)), so the local e2e run is the one that counts.
3. After it is merged, tag the merge commit: `git tag -a vX.Y.Z -m "Vesper vX.Y.Z" && git push origin vX.Y.Z`.
4. Create the GitHub release from the tag (`gh release create vX.Y.Z --generate-notes`). The notes are the merged pull requests and the issues they close. There is no CHANGELOG file.
5. Deploy from the tag, never from a branch: `git archive vX.Y.Z` into the stack, then follow [deploy.md](deploy.md). Before it, tag the running image (`docker tag vesper:latest vesper:previous`) and, if the release adds a migration, back up `data/`.
6. Check `/health`, sign in, and join a call from a second device. Say in the release notes what was not verified.

## Rolling back

Use `vesper:previous` and the compose file of the old tag (see "Upgrade" in [deploy.md](deploy.md)). A MINOR release only adds to the database, so the old image can still read it. After a MAJOR release the old image may not: restore the backup.

## Pre-release builds

Branch builds are not versions. A build from a pull request is for testing only and is never deployed to the live instance without an explicit decision.

## Not an agent's decision

The version number of a release, the license, the changelog and making the repository public are the owner's calls. An agent may prepare the release pull request and the tag, and waits for a go.
