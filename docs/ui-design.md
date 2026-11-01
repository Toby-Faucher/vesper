# UI design principles

Principles only, so later work does not invent. Last updated 2026-11-01.

Who it is for: a voice room for a few friends on desktop Chrome or Firefox. Dark only. No light theme and no mobile layout.

## Principles

1. **Status is honest.** An indicator shows a real signal: connected only when the socket and the peer connection are up, speaking only when the level says so. No optimistic or decorative states.
2. **A fallback is named.** When the denoiser fails and the browser filter takes over, say so in one line with the reason. The `Noise suppression: <mode> (<reason>)` line is the pattern. Never switch quietly.
3. **Errors say what happened and what to do.** "Microphone blocked. Allow it in the address bar." Device messages live in one table (`DEVICE_MESSAGES` in `web/src/lib/devices.ts`). Errors use `role="alert"`. Routine status uses `role="status"`.
4. **Missing data shows a dash and a short reason**, not an error box.
5. **The call screen is cheap.** No animation or effect that costs CPU while audio runs. Motion only on state changes, and none under `prefers-reduced-motion`.
6. **Keyboard works.** Every control is reachable with Tab and shows a visible focus ring (`:focus-visible` in `web/src/app.css`). Inputs without a visible label get an `aria-label`.
7. **Copy is plain.** `scripts/check-wording.sh` checks the strings in `web/src`.
8. **Size is watched.** `web/scripts/size-check.mjs` fails the build when JS or CSS in `dist/assets` passes its gzip budget. The wasm and the model are not counted.

## Not goals

Anything that hides a failure. A spinner with no cause. Sound or visual effects on the mic path. Settings nobody asked for.

## Tokens

Colours, type, radii and motion live in [`web/src/styles/tokens.css`](../web/src/styles/tokens.css), one block per style. The style is
chosen in one place: `data-style` on `<html>`, set by `web/src/lib/ui/style.ts` before the app mounts. Components never read it.
`web/src/app.css` maps the roles to Tailwind names (`bg-app`, `text-fg-3`, `bg-accent`, `text-danger` ...) and turns the default
Tailwind palette off, so a class like `bg-neutral-800` does not exist. `web/src/lib/ui/no-raw-colours.test.ts` fails on a hex value,
`rgb()`, `hsl()`, `oklch()` or a palette class outside `tokens.css`, and `tokens.test.ts` checks the contrast of each text and UI pair.

To see every role, the type scale, the radii and the focus ring, run `npm --prefix web run dev` and open `/_gallery` (dev server only).

## Not done yet

A linter or formatter for the web code and a screen reader pass.
