# Flockbook brag plan

**What it is:** a reading view for Herdr coding agents: read what each agent did, see which one needs you, reply in one place.
**Angle:** agent terminals are full of noise; Flockbook turns them into a page you can read.
**Tone:** polished, calm, dark (the app's own grey theme, Inter + JetBrains Mono, amber "needs you" dot).
**Format:** landscape 1920×1080, 30 fps, 24 s. Music 120 BPM (beat = 0.5 s), every cut on a beat.

| Time | Scene | On screen |
|---|---|---|
| 0–4 s | Hook | Six agent terminals scroll. "Agent terminals are full of noise." → one pane asks "Do you want to proceed?" → "Which one needs you?" |
| 4–7 s | Reveal | The icon assembles. "Flockbook. A reading view for your Herdr coding agents." |
| 7–10.5 s | Read | The real chat view; tool chips and the agent's answer come in block by block. "Read what each agent did." |
| 10.5–12 s | Needs you | The Needs you list fills, with the agent's question. "See which one needs you." |
| 12–14 s | Approve | Approval card slides up, pointer clicks Allow once. "Answer without opening the terminal." |
| 14–16.5 s | Reply | A message is typed and sent; the answer arrives. "Reply in one place." |
| 16.5–19 s | Resources (open PRs #24, #29) | The Resources panel slides in beside the chat. "Everything the session touched." Tag: Resources · Coming 12/1 |
| 19–20.5 s | Phone page | Phone cards pop in. "And from your phone." |
| 20.5–24 s | Outro | The icon's arrowheads fly into a V. "Your whole flock, on one page." github.com/minchenlee/flockbook · Free · MIT · macOS & Linux · Unofficial, not affiliated with Herdr |

**Source material:** the project's own preview screenshots (from the docs/herdr-app-landing branch, PR #27), animated as cropped pieces; the icon SVG; the flock formation from `site/static/flock.js`. All data in them is the repo's demo data (shop, atlas, fbdemo-coop); the terminal noise in the hook is illustrative.
**Sound:** synthesised in `work/synth.py` (A minor / C major, kick, bass, plucks, pad; effects in key on the same grid).
