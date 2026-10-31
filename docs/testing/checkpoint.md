# Quality checkpoint (manual, Toby signs off)

## 1. Purpose and gate

The spike sounded good with a Bluetooth headset, but that headset did its own noise processing, so the spike proved little about Vesper's denoiser. This checkpoint tests the real pipeline (DeepFilterNet in the browser, Opus over WebRTC) against Discord, with a wired mic, in realistic noise.

**Gate:** the accounts, text and polish plans do not start until Toby fills in section 11 and either signs off or lists fixes. Nothing here is automated; a human with the equipment below runs it.

Time needed: about 2 hours for recording, 30 minutes per listening session.

## 2. Equipment and preconditions

| Item | Requirement |
|---|---|
| Mic | Wired or USB. NOT Bluetooth: a Bluetooth headset mic drops to the low-bandwidth call profile (about 8 to 16 kHz audio) and often runs its own noise processing, which would hide or fake what Vesper does. Settings shows an amber warning if it detects a Bluetooth label. |
| Headphones | Wired, on the speaker side of the machine under test, so the mic does not hear the speakers (echo). |
| Browsers | Latest desktop Chrome and latest desktop Firefox. |
| Weaker laptop | One older or low-power laptop (dual-core, no discrete GPU is ideal) with Chrome. |
| Listeners | At least 3 people (not Toby), headphones on. They can be remote, joined to the call. |
| Discord | Desktop app or browser, Krisp/noise suppression left on its default setting. Write down what the default was in 11.1. |
| Fairness | Same room, same mic, same mic position (mark it with tape), same speaking volume and distance for both apps. Do Vesper and Discord back to back for each scenario. |

Pick one mic and write its model in 11.1. Windows/Mac/Linux system "mic enhancements" must be off for both apps.

### Running Vesper

Run the server and the web app as in [docs/dev.md](../dev.md#run) (build the denoiser first). For friends or a second machine, deploy it as in
[docs/deploy.md](../deploy.md): the server must advertise an address the other machine can reach, and the web app needs HTTPS for their mic.
Variables are in [docs/env.md](../env.md).

Record the git commit and URL in 11.1: `git rev-parse --short HEAD`.

### Where the controls are

- In the room, button **Audio settings** opens Settings:
  - **Noise suppression**: DeepFilterNet (best) / Browser built-in / Off. This is `mode`.
  - **Attenuation limit** (6 to 100 dB), **Post filter** (0 to 0.05), **Input boost** (0 to 12 dB). Only shown in DeepFilterNet mode.
  - **Bitrate** (32 to 510 kbps), **DTX** (applies at next join), **Browser noise suppression on top**, **Auto gain**. Echo cancellation is always on.
  - **Microphone**, **Speaker**, input level meter, **Mic test (hear yourself)**, **Push to talk**.
- In the room, button **Diagnostics**: RTT, jitter, loss, send kbps, codec, ICE, path; an **Audio pipeline** section (state, input and output level, queue, underruns, drops, denoiser time, mic track, capture context); **Copy report**; **Test connection**.
- The line under your name reads `Noise suppression: <mode> (<reason>)`. If the denoiser fails you see the fallback reason there, for example `denoiser too slow for real time`, `denoiser stalled`, `denoiser unavailable: ...`.
- Settings are saved in the browser (localStorage key `vesper.audio`). To reset to defaults: devtools Console, `localStorage.removeItem('vesper.audio')`, then reload.

For every Vesper run use: mode DeepFilterNet, bitrate 64, DTX on, browser noise suppression off, auto gain off, unless the step says otherwise. Write the settings in the clip log.

## 3. Scenarios and script

Four scenarios, each recorded once per app per browser:

| ID | Scenario | How |
|---|---|---|
| fan | Fan running | Desk or box fan on medium, 1 m from the mic. |
| keys | Keyboard while talking | Type steadily on a mechanical or loud keyboard while reading. |
| tv | TV speech in background | TV on a news or talk channel at conversation volume, 2 to 3 m from the mic. Not music. |
| quiet | Quiet speech | Quiet room, speak softly (as if someone is asleep nearby). |

Read this script aloud each time, at an even pace (about 12 seconds):

> 1. Please pass the purple folder to the tall man.
> 2. Six thick thistles stick to the thin stick.
> 3. The quick fox jumped over Thursday's cold fence.
> 4. Seven bags of fresh peaches cost forty dollars.

It has hard starts (p, t, k), sibilants (s, th) and word endings, which is where denoisers clip and smear.

### Capturing the same performance through both apps

Recording the listener's output measures the whole path (mic, denoiser, encoder, network, decoder). Use method A unless you have only one machine.

**Method A: a listener machine records (best).** A second machine joins the call and records what it hears.
1. Machine 2 joins the Vesper room (Chrome). Toby's machine joins with the settings above.
2. On machine 2 (Linux, PipeWire), find the output monitor: `pw-record --list-targets` or `pactl list short sources | grep monitor`. If `pw-record` is missing, install pipewire via your package manager, or use Audacity.
3. Start: `pw-record --target <monitor-name> fan-vesper-chrome-usbmic-1.wav` (Ctrl+C to stop). Audacity alternative: Transport, Recording device set to the monitor/loopback (Linux), "Windows WASAPI loopback" (Windows), or BlackHole (Mac).
4. Toby reads the script. Stop recording. Machine 2 must not play anything else.
5. Repeat with Toby and machine 2 in a Discord call (same noise scenario, same mic, same position).

**Method B: OBS on the listener machine.** Same call setup as method A (Toby on machine 1, a listener joined on machine 2), but record with OBS on machine 2: add an "Audio Output Capture" source (Desktop Audio), Settings, Output, Recording format wav, record, stop. Record the LISTENER side output only, never Toby's own mic.

Always: Machine 2 records only what it hears from the call (not Toby's local mic monitor), and keep levels untouched (no OBS filters, no normalising) so volume differences between apps stay audible. Do NOT normalise later either.

### Naming

`<scenario>-<app>-<browser>-<mic>-<run>.wav`, for example `keys-vesper-firefox-usbmic-1.wav`, `tv-discord-chrome-usbmic-1.wav`. Apps: `vesper`, `discord`. For Discord clips the browser field is the browser the Discord app ran in, or `app` for the desktop client (e.g. `keys-discord-app-usbmic-1.wav`). Keep all clips in one folder, `~/checkpoint/clips/`.

Minimum set: 4 scenarios x 2 apps = 8 clips for Chrome, then repeat the 4 Vesper clips in Firefox, and run the weaker laptop (section 8).

## 4. Blind A/B protocol

1. Put the clips for one scenario and one browser in a folder: one Vesper, one Discord.
2. Randomise which is A and B per scenario. Copy-paste recipe:

```bash
cd ~/checkpoint && mkdir -p blind && : > key.csv
for s in fan keys tv quiet; do
  v=clips/$s-vesper-chrome-usbmic-1.wav; d=clips/$s-discord-app-usbmic-1.wav
  if [ $((RANDOM % 2)) -eq 0 ]; then A=$v; B=$d; else A=$d; B=$v; fi
  cp "$A" blind/$s-A.wav; cp "$B" blind/$s-B.wav
  echo "$s,A=$A,B=$B" >> key.csv
done
```

Adjust the filenames to what you recorded. Toby must not look at `key.csv` until all listeners have finished. (Spreadsheet alternative: `=IF(RAND()<0.5,"Vesper is A","Vesper is B")` per scenario row.)
3. Send each listener the `blind/` folder only. Headphones, same volume, they may replay freely, in any order. Do not tell them which is which or what you changed.
4. Each listener fills one sheet (below) per scenario. Pick for each question A, B, tie.

### Scoring sheet (one per listener per scenario)

Listener: ____  Scenario: ____

| Question | A | B | Tie |
|---|---|---|---|
| Less noise leaking through (fan / keys / TV) | | | |
| More natural voice (less robotic, watery, underwater) | | | |
| Fewer clipped word starts or ends | | | |
| Quiet parts of speech still audible (not eaten) | | | |
| Overall, which would you rather talk with all evening | | | |
| Did either sound robotic or watery? (A / B / neither / both) | | | |

Notes (words that got cut, artifacts you heard, timestamps): ______

### Pass criterion (Toby may change)

Resolve the key, then count per scenario how many listeners (majority of at least 3) prefer or tie Vesper on "overall".

Pass if ALL of:
- Vesper preferred or tied by a majority of listeners in at least 3 of the 4 scenarios.
- No scenario where a majority votes Vesper "robotic or watery".
- No majority reporting clipped words or lost speech in the `quiet` scenario for Vesper.

Edited thresholds: ____________________

## 5. Tuning `attenDb`, `postFilter`, `inputBoostDb`

Defaults live in `web/src/lib/audio/settings.ts`, `DEFAULT_AUDIO_SETTINGS` (now `attenDb: 100`, `postFilter: 0.02`, `inputBoostDb: 0`). Tune in the Settings sliders while the listener machine records. Verified in `capture.ts`: input boost, attenuation limit and post filter all apply live mid-call (the sliders send a `set` message to the worker; input boost changes the gain node), no rejoin or rebuild. Only the final edit of `DEFAULT_AUDIO_SETTINGS` needs a reload, and it only affects browsers with no saved `vesper.audio`. Change ONE knob at a time, record a clip of the fan and quiet scenarios for each value, and listen blind.

Order: input boost, then attenuation limit, then post filter.

| Knob | In plain words | Step | Try |
|---|---|---|---|
| Input boost (`inputBoostDb`) | Gain applied before the denoiser. The model works best with a healthy level; a quiet mic makes it treat speech as noise. | 3 dB | 0, 3, 6, 9, 12. Stop when the input meter clips (amber hint "Too loud: the signal is clipping"). |
| Attenuation limit (`attenDb`) | How much noise it may remove at most. 100 = no limit (strongest). Lower keeps some natural background, which sounds less robotic but leaks more noise. | 10 dB from 100 down | 100, 40, 30, 20. Stop at the first value that stops sounding watery without leaking too much. |
| Post filter (`postFilter`) | Extra suppression of leftover noise in already-quiet spots. Higher is cleaner in pauses, but can make speech edges choppy. | 0.01 | 0, 0.01, 0.02, 0.03, 0.05. |

Procedure per knob: set value, record the fan and quiet clips (use the script), name them `fan-vesper-chrome-usbmic-<knob><value>.wav`, listen blind to the set, pick the best; carry that value forward to the next knob. Keep a table:

| Knob | Value tried | Fan: noise | Quiet: speech kept | Robotic? | Pick |
|---|---|---|---|---|---|
| inputBoostDb | | | | | |

When done, edit `DEFAULT_AUDIO_SETTINGS` in `web/src/lib/audio/settings.ts` and record the values in 11.3. Run `cd web && npm test` (a settings test may pin the defaults; update it with the new values).

## 6. Low-latency model variant

The upstream repo has `DeepFilterNet3_ll_onnx.tar.gz` in `models/` at revision `d375b2d` (the same rev that `scripts/fetch-model.sh` pins). Compare it with the default `DeepFilterNet3_onnx.tar.gz`.

To swap (experiment only, do not commit the binary):
1. In `scripts/fetch-model.sh` change both the URL file name and `OUT` from `DeepFilterNet3_onnx.tar.gz` to `DeepFilterNet3_ll_onnx.tar.gz`. The `SHA256` pins the standard model: run once, copy the printed `sha256 <hash>` line into `SHA256` (it will say mismatch the first time; this is expected, and you are trusting the download from the pinned rev).
2. In `scripts/build-dsp.sh` change `MODEL=` to `crates/vesper-dsp/model/DeepFilterNet3_ll_onnx.tar.gz`. It is installed as `web/public/dsp/model.tar.gz`, which is where the app loads it.
3. Run `scripts/fetch-model.sh && scripts/build-dsp.sh`, hard-reload the app (Ctrl+Shift+R), join and check the status line says `deepfilter` with no fallback reason.
4. If the Rust DSP code assumes the standard model's tensor shapes, the ll model may fail to load (the status line shows `denoiser unavailable: ...`); record that and stop: it then needs code work, not tuning.
5. Native Rust tests/benches still use the standard model (`crates/vesper-dsp/src/lib.rs` test helper hard-codes `DeepFilterNet3_onnx.tar.gz`); the swap affects only the browser build.
6. Revert: `git checkout scripts/` and rebuild.

Compare (same scenarios, same settings):

| | Standard | Low-latency |
|---|---|---|
| Added delay (section 7), ms | | |
| Noise leakage (fan, TV) | | |
| Robotic / watery | | |
| Worker CPU on the weaker laptop | | |
| Verdict | | |

Switch only if the delay gain is noticeable and quality is not worse in a blind listen.

## 7. Delay measurement

Target: the denoiser path adds at most 30 ms over mode=off. The output buffer alone starts at 30 ms (estimated from the code, not measured), so expect to miss this target. Record the real number. DeepFilterNet's own lookahead was never measured and may add on its own, so this number is unknown until now. See docs/testing/latency.md for the acoustic loopback method (end to end, mic to the other person's ears).

Primary method: the acoustic loopback in docs/testing/latency.md (prefer it once it exists). It measures real mic-to-ear delay.

Rough secondary method (use only if latency.md is not there yet; expect +/- 20 ms error, which is as big as the target, so treat it as a sanity check):
1. Two devices: machine A (Toby, joined in the mode under test, USB mic and headphones) and machine B (joined as a listener, with speakers, volume moderate).
2. Put machine A's mic 10 cm from machine B's speaker. Machine A's headphones stay on; nothing else is recorded. The mic must be the only input in the recording.
3. On machine A open Audacity, input = the USB mic, record. Make a short sharp click (clap, or snap your fingers next to the mic) every 2 seconds, 10 times. The recording will show each click twice: first heard directly in the air, then again a bit later as it comes back from machine B's speaker after travelling through the call.
4. Echo cancellation and the denoiser can eat clicks. Check the waveform shows both peaks for each click; if the second is missing, click louder or move the mic closer to the speaker. Do the test in mode=off first to confirm it works.
5. In Audacity zoom in on a click, select from the first peak to the second, and read the length at the bottom (set the selection format to milliseconds). Take the median of 10.
6. Repeat with mode=deepfilter (wait for `Noise suppression: deepfilter` with no reason shown). Added delay = median(deepfilter) minus median(off).

| Run | mode | Median round trip (ms) | Notes |
|---|---|---|---|
| 1 | off | | |
| 2 | deepfilter | | |
| 3 | deepfilter, ll model | | |
| Added by denoiser | (2) minus (1) | | target at most 30 ms |
| Added, ll model | (3) minus (1) | | |

## 8. Browser matrix and weaker laptop

Run the 4 scenarios in each cell (at least `fan` and `quiet` on the laptop).

| Item to record | Chrome desktop | Firefox desktop | Weaker laptop (Chrome) |
|---|---|---|---|
| Browser version | | | |
| Status line stayed `deepfilter` for the whole 12 s script? | | | |
| Any fallback to browser, and the reason text | | | |
| CPU % (see below) | | | |
| Worker real-time factor (see below) | | | |
| Blind result vs Discord (3 of 4 scenarios?) | | | |

- **CPU %**: Chrome Task Manager (Shift+Esc) shows the tab and its "Dedicated Worker" rows. Firefox: `about:processes`. Laptop: `htop` or the OS task manager. Record the steady value during speech.
- **Real-time factor**: the UI does not display it. The worker is forced to fall back to browser mode when frames repeatedly take longer than their budget, so the practical measure is: did it fall back (status line reason `denoiser too slow for real time`) and what is the worker's CPU. Optionally, devtools Performance panel, record 5 s in the call, and look at the worker thread: busy time divided by wall time is the factor (must stay well under 1.0). Write what you used.
- **Firefox (known unverified risk)**: the mic may not deliver 48 kHz, so the denoiser could fall back at every join (status line reason mentioning `Hz` or `denoiser unavailable`). Check on three consecutive joins and record.
- **Chrome (known)**: remote audio only plays if the stream is also attached to a muted audio element (the code does this). Verify you can hear the other tab in Chrome with no extra steps.
- Compare against Discord on the same laptop for CPU if the laptop is slow in a call.

## 9. Unverified items carried from the build

Last updated 2026-10-30. Tick as you check. Use two tabs (or two machines) where a second participant is needed.

- [ ] Two-tab join: a second tab joins; you hear yourself from the other tab; tab names show; leaving removes the entry.
- [ ] Mute: the other side sees `(muted)` and hears nothing; unmute restores.
- [ ] Deafen: you hear nobody; undeafen restores; your own mute state is unaffected.
- [ ] Per-peer volume slider changes loudness and survives a reload and rejoin.
- [ ] Speaking ring (green) shows on the speaker, on both tabs.
- [ ] Tab backgrounding: put the tab in the background for 2 minutes, audio keeps playing both ways; PTT does not stay stuck.
- [ ] Fallback by stalling the worker (Chrome): join in deepfilter, open devtools, Sources, Threads pane (right side), select the worker `dsp.worker`, click Pause. Within about a second the status line should show `browser (denoiser stalled)`, audio keeps flowing. Resume and rejoin to restore.
- [ ] Fallback by CPU starvation: devtools Performance, CPU throttling 6x slowdown (4x if nothing happens), speak; expect `denoiser too slow for real time`.
- [ ] Fallback by missing model: devtools Network, block request URL `*/dsp/model.tar.gz`, join; expect `denoiser unavailable: ...` and working audio.
- [ ] Denied mic permission mid-call: in the site permissions (lock icon) switch the mic to Block while joined; expect an error message, no crash, and ability to rejoin after allowing.
- [ ] Unplug a USB mic mid-call (with that mic picked in Settings, not "System default"): expect the "unplugged" alert; replug and rejoin or switch.
- [ ] Device switching mid-call: change Microphone and Speaker in Settings; audio follows (Firefox may not support Speaker choice: note the message).
- [ ] Push to talk: enable, set a key, hold to talk, release to stop; switch window (alt-tab) while held and confirm the mic cuts off (does not stay open).
- [ ] Connectivity check from a phone on cellular (Wi-Fi off): join, Diagnostics, Test connection, expect OK; note ICE path (relay or direct) and RTT.
- [ ] Copy report button: paste into a text file, check it has user agent, mode, RTT, jitter, loss, codec, ICE state, the audio pipeline lines and no join key.
- [ ] Bitrate change mid-call: lower to 32 kbps while joined (applies now; Send kbps in Diagnostics drops); raise above the joined value (shows the "applies on rejoin" note).
- [ ] DTX: with DTX on, Send kbps drops during silence.

## 10. Order of work

1. Set up (section 2), mark mic position.
2. Chrome: record all 8 clips (section 3), then do the blind test (section 4).
3. Tune (section 5), re-record, update defaults.
4. Low-latency model (section 6), delay (section 7).
5. Firefox and the laptop (section 8).
6. Tick section 9 while doing the above.
7. Fill section 11.

## 11. Results and sign-off

### 11.1 Setup

| Item | Value |
|---|---|
| Date | |
| Git commit / URL tested | |
| Mic model and position | |
| Headphones | |
| Chrome / Firefox versions | |
| Weaker laptop model | |
| Discord noise suppression setting | |
| Listeners | |

### 11.2 Blind A/B results (counts of listeners, Vesper vs Discord vs tie)

| Scenario | Browser | Noise: V/D/T | Natural: V/D/T | Clipped: V/D/T | Quiet kept: V/D/T | Overall: V/D/T | "Robotic" votes |
|---|---|---|---|---|---|---|---|
| fan | Chrome | | | | | | |
| keys | Chrome | | | | | | |
| tv | Chrome | | | | | | |
| quiet | Chrome | | | | | | |
| fan | Firefox | | | | | | |
| keys | Firefox | | | | | | |
| tv | Firefox | | | | | | |
| quiet | Firefox | | | | | | |

Pass criterion met (section 4)? Yes / No. Notes on robotic or clipped speech: ______

### 11.3 Tuned defaults chosen

| Setting | Old | New |
|---|---|---|
| attenDb | 100 | |
| postFilter | 0.02 | |
| inputBoostDb | 0 | |
| Model | DeepFilterNet3 | standard / ll |
| Added delay (ms) | | |

`DEFAULT_AUDIO_SETTINGS` updated in the repo: yes / no (commit `docs: checkpoint results and tuned defaults`).

### 11.4 Fixes needed

| # | Problem | Severity (blocks / should fix / later) |
|---|---|---|
| 1 | | |

### 11.5 Sign-off

- [ ] Signed off: quality is good enough to start the accounts, text and polish plans.
- [ ] Not signed off: fix the items in 11.4 first, then rerun the affected scenarios.

Toby: ______________  Date: ________
