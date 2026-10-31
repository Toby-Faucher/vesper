# 0006. Gate later plans on a manual quality checkpoint

Status: accepted, 2026-10-30

Context: The spike sounded good, but it used a Bluetooth headset that does its own noise processing. That proved little about Vesper's denoiser. Building accounts and text on an untested voice path risks wasted work.

Decision: Accounts, text and polish plans do not start until Toby runs the quality checkpoint (wired mic, realistic noise, compared with Discord; see [the checkpoint](../testing/checkpoint.md)) and signs off or lists fixes. Latency numbers come from the loadtest and the latency doc, not from estimates.

Why: Voice quality is the product. Measure first, mark every unmeasured number as not verified, and never claim a number without a run behind it.

Consequences: Some features wait on a human with equipment. VAD gating is a documented deferral: the threshold in the UI is display-only and says so. Deferred minor findings from the review rounds are listed in [known-issues.md](../known-issues.md).
