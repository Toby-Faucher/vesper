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
8. **Size is watched.** `web/scripts/size-check.mjs` fails the build when, in gzip, the first load JS passes 70 kB, any lazy chunk 35 kB, all JS 150 kB, the first load CSS 12 kB or all CSS 20 kB. What loads before the first click is the number that counts, for JS and for CSS. The wasm and the model are not counted.

## Not goals

Anything that hides a failure. A spinner with no cause. Sound or visual effects on the mic path. Settings nobody asked for.

## Layout and components

Written 2026-11-01, after the visual check of every screen.

**The shell.** One left sidebar, the main pane, and nothing on the right. The sidebar is 248 px wide. Its header holds the
logo and the switcher button (Ctrl+K). Under it is the Rooms list. Each row is 28 px and shows the room name,
an unread count and an amber `@n` pill for mentions. People in voice are listed under their room, with avatar, name, a mute or
deafen icon and the speaking cue. In your own call each row also ends in a connection icon: three bars good, two fair, one in
the warning colour poor, all faint while measuring. Its name says "Connection to alice: good"; the tooltip and the person card
give the figures in words. Your own row with Settings sits at the bottom. The sidebar hides fully with `Ctrl+\`. It never
shrinks to a row of icons. The main pane has a 48 px header with the room name, Pins, Search and Join voice. Join voice is the amber primary only when you are in no call. While you are in a
call, a small call bar floats at the bottom centre of the main pane. The pane leaves room for it, so it covers nothing.
Settings and Admin use the same shell: a list of sections on the left of the pane and one page of controls.

**The tokens rule.** A component uses role names only (`bg-app`, `text-fg-2`, `bg-accent`, `text-danger`). Raw steps and
hex values live only in [`web/src/styles/tokens.css`](../web/src/styles/tokens.css). Sizes follow the 4 px scale. Text uses six
sizes (12, 13, 14, 16, 20 and 24 px). Names, headings and unread rooms use the strong weight (600). Buttons, labels and
table heads use 500, which Segoe UI and Liberation Sans draw as regular. The rest is 400. Counts, times and stats use
tabular figures.

**The colour rules.**
- The accent is amber (the logo colour) unless the person picks another in Settings > Appearance. Below, "amber" means
  the accent.
- Amber means one of five things: the primary action, focus, a selection marker, a mention, or the unread line. A view has
  one amber fill at most. Text on an amber fill is near-black.
- Warnings use their own orange plus an icon and words. They never use amber.
- A person's colour shows only on their avatar and their speaking ring. Names, text, row borders and messages stay grey.
  There are no role colours.
- Room names have no `#` in front.
- Colour is never the only cue. Speaking also shows a static bar icon, and muted shows an icon.

**The accent choice.** Per account: `accounts.ui_accent`, sent in `UserInfo`, so it follows the person to every device and
paints from `/api/me` before the shell renders (the last value is kept in `vesper.accent` for the sign in page). It is one
of 8 presets (amber, rose, orchid, violet, indigo, sky, teal, lime) or a custom `#rrggbb`. The server checks only the form; a
custom colour must pass the contrast rules in `web/src/lib/ui/accent.ts` (near-black text on the fill and on its hover 4.5:1,
the fill 3:1 on the page, sidebar and panels, link text 4.5:1 on the page and on the soft fill). Settings refuses one that does not,
shows the ratio and offers the nearest allowed colour, and a stored one that fails paints amber. `roles()` in `accent.ts` derives every
accent role (`--accent`, `--accent-hover`, `--accent-fg`, `--accent-subtle`, `--accent-line`, `--focus`) from the one fill and
sets them on `<html>`; `accent.test.ts` checks each preset. The logo and the tab icon follow the accent: `tint()` gives a colour
matrix that moves amber to the fill and keeps every grey, applied to the logo by the `#logo-tint` SVG filter in `index.html`
(set before the first paint, so the logo never shows amber first) and to the 32 px favicon on a canvas (`favicon.ts`, lazy,
the PNG fetched as a file, a `blob:` URL). Amber leaves both files untouched. The avatar colours, the notification icon and the speaking ring do not
change. There is no coral: it sat between the danger red and the warning orange.

**The switch point.** The look is chosen in one place: `data-style` on `<html>`, set by `web/src/lib/ui/style.ts` before the app
mounts. A new style is a new block in `tokens.css` with the same role names, plus one entry in `STYLES`. Components never read
`data-style`.

## Tokens

Colours, type, radii and motion live in [`web/src/styles/tokens.css`](../web/src/styles/tokens.css), one block per style. The style is
chosen in one place: `data-style` on `<html>`, set by `web/src/lib/ui/style.ts` before the app mounts. Components never read it.
`web/src/app.css` maps the roles to Tailwind names (`bg-app`, `text-fg-3`, `bg-accent`, `text-danger` ...) and turns the default
Tailwind palette off, so a class like `bg-neutral-800` does not exist. `web/src/lib/ui/no-raw-colours.test.ts` fails on a hex value,
`rgb()`, `hsl()`, `oklch()` or a palette class outside `tokens.css`, and `tokens.test.ts` checks the contrast of each text and UI pair.

To see every role, the type scale, the radii and the focus ring, run `npm --prefix web run dev` and open `/_gallery` (dev server only).

## Motion

Every animation goes through [`web/src/lib/ui/motion.ts`](../web/src/lib/ui/motion.ts) as a named preset, played by anime.js's WAAPI
build (`waapi.animate` only). Only `opacity` and `transform` move, except the jump flash, which fades a background colour.
anime.js is not in the first load: it loads right after start, and until it arrives every preset is instant (the jump flash
is the instant change it has under reduced motion).

| Preset | Pose | Used by |
| --- | --- | --- |
| `pop` | scale 0.97, 160 ms | popovers, menus, dialogs including the switcher and search (open, and an empty box with their fill fading out on close), the mention list, the drop overlay; the CSS tooltip does the same pose in CSS |
| `rise` | 6 px up, 220 ms (under the timeline's 8 px bottom margin, so it keeps following) | the call bar, a new message, the timeline pills |
| `swap` | 4 px up, 160 ms | the timeline when another room opens, a settings or admin section, the connection bar |
| `row` | 8 px from the left, 160 ms | a person joining or leaving voice |
| `bump` | scale 0.6 with a small overshoot, 160 ms | a reaction coming, going or changing its count; a sidebar unread count |

Enter eases out (`--ease-out`, `--ease-pop` for `bump`); exit runs the pose backwards in 120 ms, easing in. The selected room's fill
glides to the next room (`glide`). Buttons sink a little while pressed and hover changes colour: both plain CSS on the same tokens.
A message row's highlight (hover, keyboard focus, editing, a mention) is a CSS fade of its background: in over 160 ms, out over
220 ms, so moving between rows crossfades; the focus ring itself shows at once. The jump flash fades in, holds and fades out
(`flash`). Hiding the sidebar slides it out to the left and the main pane follows its edge; showing it slides both back
(`slide`). The grid takes its final columns once at the start and only transforms move after that; the leaving sidebar is
inert at once and hidden when the slide ends. The saved hidden state at load does not slide.

Rules: under `prefers-reduced-motion` every preset is an 80 ms opacity fade, and the glide, the jump flash, the row highlight and
the sidebar are instant. Nothing
animates on a speaking change; the ring toggles instantly (a fade once cost about 60 percent more CPU in a call). History, a room open and
"Load older" do not animate; only live arrivals do, three at most. The rooms, counts and people that come with the first hello appear
at once (`settle()`), and at most three transitions start in one task. A dialog or popover closes at once, so focus moves on; the fading
box is empty (no text twice in the page) and hidden from screen readers. A leaving element is inert at once, so it takes no focus,
click or second Enter. Nothing with `role="status"` animates its text. Typing never animates: the composer's text and caret stay put.

## Components

The primitives live in [`web/src/lib/ui/c/`](../web/src/lib/ui/c/): buttons, fields, switch, range, dialog, popover, menu, avatar,
badge, status line, empty state. Dialogs are native `<dialog>` with `showModal()`; popovers and menus use the Popover API and
`place()` in `web/src/lib/ui/place.ts`. The CSS tooltip (`data-tip`) goes through `place()` too
(`web/src/lib/ui/tip.ts`), so it flips or shifts to stay on screen. Per-user colour is an avatar slot from `web/src/lib/ui/avatar.ts`. An avatar shows the person's picture when they set one (Settings > Account), else their initials on the slot colour; the speaking ring and the mute glyph draw the same around either. Icons are Lucide paths in
`web/src/lib/ui/icons.ts`. `/_gallery?primitives` shows each one in its states.

## Checked

The tokens (contrast in `tokens.test.ts`, colours only through roles in `no-raw-colours.test.ts`), errors shown only as table sentences
(`no-raw-errors.test.ts`), and the browser tests in `web/e2e/ui.mjs`: the keyboard paths of the primitives, a keyboard-only walk with
a focus ring on every Tab stop, reduced motion and layout shift. Headless Chromium only.

## Not done yet

A linter or formatter for the web code, a screen reader pass, and the keyboard and focus paths in Firefox and Safari.
