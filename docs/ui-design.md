# UI design principles

Principles only, so later work does not invent. Last updated 2026-11-03.

Who it is for: a voice room for a few friends on desktop Chrome or Firefox. Dark by default, with a light scheme in Settings. No mobile layout.

## Principles

1. **Status is honest.** An indicator shows a real signal: connected only when the socket and the peer connection are up, speaking only when the level says so. No optimistic or decorative states.
2. **A fallback is named.** When the denoiser fails and the browser filter takes over, say so in one line with the reason. The `Noise suppression: <mode> (<reason>)` line is the pattern. Never switch quietly.
3. **Errors say what happened and what to do.** "Microphone blocked. Allow it in the address bar." Device messages live in one table (`DEVICE_MESSAGES` in `web/src/lib/devices.ts`). Errors use `role="alert"`. Routine status uses `role="status"`.
4. **Missing data shows a dash and a short reason**, not an error box.
5. **The call screen is cheap.** No animation or effect that costs CPU while audio runs. Motion only on state changes, and none under `prefers-reduced-motion`.
6. **Keyboard works.** Every control is reachable with Tab and shows a visible focus ring (`:focus-visible` in `web/src/app.css`). Inputs without a visible label get an `aria-label`.
7. **Copy is plain.** `scripts/check-wording.sh` checks the strings in `web/src`.
8. **Size is watched.** `web/scripts/size-check.mjs` fails the build when, in gzip, the first load JS passes 70 kB, any lazy chunk 35 kB, all JS 200 kB (raised from 150 on 2026-11-02 and from 170 on 2026-11-14 by Toby: only lazily loaded code adds to it, and the first-load ratchet protects what people wait for), the first load CSS 12 kB or all CSS 24 kB (raised from 20 on 2026-11-14). What loads before the first click is the number that counts, for JS and for CSS. The wasm and the model are not counted.

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
  one amber fill at most. Text on an amber fill is near-black (white in the light scheme).
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
(set before the first paint, so the logo never shows amber first) and to the 32 px favicon on a canvas (`favicon.ts`, lazy:
it fetches the PNG and makes a `blob:` URL). Amber leaves both files untouched. The avatar colours, the notification icon and the speaking ring do not
change. There is no coral: it sat between the danger red and the warning orange.

**The switch point.** The look is chosen in two places, both on `<html>` and both set before the app mounts: `data-style` by
`web/src/lib/ui/style.ts` and `data-scheme` (`dark` or `light`) by `web/src/lib/ui/scheme.ts`. Components never read either.
A new style is a block in `tokens.css` or a lazy file with the same role names, plus one entry in `STYLES`.

**Color scheme.** Settings > Appearance > Color scheme: Dark, Light or System (follows `prefers-color-scheme`, and a change of the
system setting applies at once). Per device, in `localStorage` (`vesper.scheme`), no server change. The default is Dark, so the
look people have stays. `index.html` loads `public/scheme-init.js`, a small blocking script (a file, because the CSP forbids inline script) that sets `data-scheme` and the `theme-color` meta before the first
paint, so there is no flash; `scheme.ts` repeats the rules and keeps the tab colour right after a change. The base block of
`tokens.css` is dark; `html[data-scheme='light']` at the end of it redefines the grey steps, the accent roles, danger, warn, ok,
the avatar slots, code colours and the shadow, and sets `color-scheme: light` (native controls, scrollbars and the select list).
Components need no light rules: they use roles only. Light surfaces are warm white (`#fbfaf8` page, `#f4f2ef` sidebar, `#ffffff`
panels and inputs).

Light accent: a light page cannot take the dark preset fills (amber is 1.9:1 on it). So `roles(fill, 'light')` in `accent.ts`
darkens the fill, in the same hue, until white text on it passes 4.5:1, makes the hover darker, and finds a link colour that passes
4.5:1 on the page and on the soft fill. In light the text on an accent fill is white (`--on-accent`), in dark it stays near-black.
Settings still checks a custom colour in dark only (the stored value is the same on every device); a colour that passes there
always paints in light, which `accent.test.ts` checks for the presets and a grid of colours. The logo and the tab icon keep the
unchanged accent colour: the logo has its own dark ring, so it reads on a light sidebar. This was not checked on screen.

## Styles

Settings > Appearance > Style, per device (`vesper.style`). A style is a lazy CSS file in `web/src/styles/`, loaded by a dynamic
`import()` only when it is chosen, so the first load does not grow (Vesper itself is `tokens.css`, in the first load). `main.ts`
waits for the file (at most 2 s) before it mounts, so the page never shows a style without its tokens. The stored choice and
`data-style` are the same id; an unknown id falls back to Vesper.

| Style | Scheme | Own accent | What it is |
| --- | --- | --- | --- |
| Vesper | both | no (amber) | The default look. |
| Persona | dark | red | Red, black and white. Slanted buttons and chips, a red slash for the current room, Anton headings, a star cursor, tilted dialogs. |
| Cozy | light | leaf green | Warm cream paper, wood-brown text, round shapes, the system rounded face. |
| BBS | dark | yellow | A dial-up board: black, monospace, square corners, a blue selection bar. |
| Aurora | dark | mint | A deep blue night sky, soft corners, a faint static glow behind the page. |

**Why these three.** Brisk has about 48 styles, most of them novelty or built on its game-streaming layout. These were picked for
reading long text and for variety: Cozy is the warm light one (dark brown on cream, 7 to 11:1), BBS is the monospace one (light grey
on black, 12:1 and up, the one a person who lives in a terminal may want), Aurora is a dark with another mood (cool blue instead of
warm grey, mint instead of amber). Each is a few lines of tokens and one or two shape rules (0.5 kB gzip each; Persona is 1.8), with no font
file: Cozy and BBS use the system rounded and monospace faces, so their type changes with the OS. Brisk's frosted glass, moving
light, scanlines and page transitions are not ported. Not picked: Blueprint and Aqua (low contrast text on a pattern or a gradient), Bonfire (too close to Vesper dark).

**Style and scheme.** Each style says in `STYLES` (`style.ts`) what it does with the Color scheme choice: `both` (Vesper: the
choice applies), `dark` or `light` (the style is that by nature). For a fixed style the choice is ignored: Settings greys the
Color scheme group and says "This style is dark only", and `data-scheme` is set to the style's own. The map of fixed styles is
repeated in `public/scheme-init.js` (it must set the right scheme before the first paint); `scheme.test.ts` checks the two agree.

**Style and accent.** Amber is the default accent, and the default is replaced by a style's own colour when the style sets
`own: true` (`data-own-accent` on `<html>`; `applyAccent` then leaves the accent roles to the style's CSS). Any other accent the
person picked still applies, painted by `roles()` as for Vesper. The Persona red is `#e60012` with white text (5.0:1).

**Persona.** Ported from the look of Brisk's Persona style (palette, type, shapes, motion), not its CSS. Tokens first: surfaces,
lines, text, accent roles, the focus ring (white, with a red ring outside it), radii 0, a hard red shadow. Then a few rules: slanted
`.btn` and `.badge` (`clip-path`; a focused button is square so its ring is not cut), the sidebar's slanted red edge, `.row.current`
as a red slash, Anton on `h1`, `h3` and room names, section labels as white tabs, a star cursor (white, red over buttons and links,
the text cursor stays over fields), dialogs with a red edge that tilt in with a spring. Message text is Rubik regular, white on
near-black (about 19:1), unchanged in size. Left out: the page-wipe and calling card animation and the background shards. The motion
that is there (the dialog tilt, the row and button hop) is inside `prefers-reduced-motion: no-preference`.
Fonts are self-hosted in `web/src/styles/fonts/`, latin subset only, loaded with the style: Anton 400, Rubik 400 and 700
(woff2, 19 kB each), SIL Open Font License 1.1 (the licence texts sit next to the files). No CDN.

**Adding a style.**
1. Write `web/src/styles/<id>.css`. One rule, `html[data-style='<id>'] { ... }`, redefines the roles of `tokens.css` that differ
   (roles not listed stay Vesper dark). Set `color-scheme`. Then shape rules, all starting with `html[data-style='<id>']`. Prefer
   tokens (`--radius`, `--font-sans`, `--shadow-float`) to component selectors; where a component needs a hook, add a class, not a rule in the component.
2. Add the entry to `STYLES` with `scheme`, `own` and `load: () => import('../../styles/<id>.css')`. If the scheme is `dark` or
   `light`, add the id to the map in `public/scheme-init.js`.
3. Add the file to `LOOKS` in `tokens.test.ts`: every text pair needs 4.5:1 and every UI pair 3:1 in the style's own colours.
4. Put motion inside `@media (prefers-reduced-motion: no-preference)`. Fonts are self-hosted (check the licence) or a system stack; never a CDN.
5. Run `npm run size` and say what it did to the CSS budget. All CSS was 16.3 kB gzip of 20 before the styles and is 19.5 with the four, and was 20.0 at v1.8.0, when the budget went to 24 kB. The next raise needs the same justified, dated note in `size-check.mjs`. Update this section.

## Tokens

Colours, type, radii and motion live in [`web/src/styles/tokens.css`](../web/src/styles/tokens.css): the base block (Vesper dark) and the
light block. The look is chosen by `data-style` and `data-scheme` on `<html>` (see the switch point above). Components never read them.
`web/src/app.css` maps the roles to Tailwind names (`bg-app`, `text-fg-3`, `bg-accent`, `text-danger` ...) and turns the default
Tailwind palette off, so a class like `bg-neutral-800` does not exist. `web/src/lib/ui/no-raw-colours.test.ts` fails on a hex value,
`rgb()`, `hsl()`, `oklch()` or a palette class outside `tokens.css`, and `tokens.test.ts` checks the contrast of each text and UI pair in every look (dark and light so far).

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
(`web/src/lib/ui/tip.ts`), so it flips or shifts to stay on screen. Per-user colour is an avatar slot from `web/src/lib/ui/avatar.ts`. An avatar shows the person's picture when they set one (Settings > Account), else their initials on the slot colour; the speaking ring (in the accent colour, not the slot colour) and the mute glyph draw the same around either. Icons are Lucide paths in
`web/src/lib/ui/icons.ts`; the screen share ones are in `web/src/lib/share/icons.ts`, in the lazy call chunk. `/_gallery?primitives` shows each one in its states.

## Checked

The tokens (contrast in `tokens.test.ts`, colours only through roles in `no-raw-colours.test.ts`), errors shown only as table sentences
(`no-raw-errors.test.ts`), and the browser tests in `web/e2e/ui.mjs`: the keyboard paths of the primitives, a keyboard-only walk with
a focus ring on every Tab stop, reduced motion and layout shift. Headless Chromium only.

## Not done yet

A linter or formatter for the web code, a screen reader pass, and the keyboard and focus paths in Firefox and Safari.
