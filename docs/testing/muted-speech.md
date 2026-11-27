# "You are muted": manual test

The unit tests cover the rule (`web/src/lib/audio/muted-speech.test.ts`, `MutedSpeech` in `desktop/src/voice/mix.rs`), the
web capture tap and the desktop engine with a fake mic. What they cannot show is a real mic, a real room and how it feels.
How it works is in [architecture.md](../architecture.md) (search for "You are muted").

Not verified yet. Record the date, browser or OS, mic and result here when it is run.

## Steps

Two people in one call, A and B, on headphones. Do it on the web app (Chrome, then Firefox) and on the desktop app.

1. A joins with Settings > Voice > Talk mode > "Tell me when I talk while muted" on (the default).
2. A mutes and keeps quiet for 10 s. Nothing shows.
3. A mutes in the middle of a sentence and stops within a second. Nothing shows.
4. A mutes, waits 2 s and talks normally for 2 s. Within about half a second of talking the call bar says "You are muted.
   Unmute to be heard." B hears nothing the whole time, and B's view shows A as muted, not speaking.
5. A keeps talking. The notice goes after about 5 s and does not come back for a minute, even after unmute and mute.
6. A unmutes while the notice shows. It goes at once.
7. Typing, clicks and a cough while muted do not show it. A steady fan or music in the room may; note it if it does.
8. With the call bar made small, the notice opens it to the full bar.
9. A deafens while muted, or a moderator mutes A: talking shows nothing (the moderator's mute shows its own line).
10. A turns the switch off and talks while muted for 10 s: nothing shows. Turned on again, it works without a rejoin.
11. Web only: in `browser` and `off` noise suppression it still works (it reads a copy of the mic there).

## Results

| Date | Client | Mic | Result |
|---|---|---|---|
