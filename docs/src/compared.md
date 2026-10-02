# Compared

What each one's own site, docs and code said in October 2026.

| | rookey | Wispr Flow | OpenWhispr | Handy |
|---|---|---|---|---|
| Price | $0.22 an hour of speech, paid to ElevenLabs; free on your own machine | 2,000 words a week free; Pro $15/mo, $12/mo yearly | free; its cloud $8/mo | free |
| Open source | Apache-2.0 | no | MIT | MIT |
| Linux | Wayland | no | X11, Wayland | X11, Wayland |
| macOS | Apple silicon | yes | yes | yes |
| Windows | yes | yes | yes | yes |
| Without internet | yes, with a local model | no | yes | yes |
| Account | an ElevenLabs key; none on your machine | required, even free | optional | none |
| Hold to talk | yes | yes | yes | not on Wayland |
| Words as you talk | on Realtime | no | opt-in preview | with streaming models |
| Cleans up text | filler words, your own instruction (ElevenLabs) | filler words, formatting | an LLM | filler words, an LLM |
| Reads the screen | a screenshot: OCR on your machine, or a vision model | app name and screen text, through accessibility | only its assistant | no |
| Your audio goes to | ElevenLabs, or stays on your machine | its cloud, always | its cloud, your provider, or stays | stays on your machine |

## What a month costs

ElevenLabs charges for the length of the audio: $0.22 an hour, $0.39 on Realtime, about 20% more with your words or screen terms, 30% more with an edit instruction. No subscription, and the first 4.5 hours a month (2.5 on Realtime) are free. A month here is 22 working days, at about 150 words a minute.

| Talking a day | rookey | rookey, Realtime | Wispr Flow Pro |
|---|---|---|---|
| 10 minutes, ~1,500 words | $0.81 | $1.43 | $15, or $12 yearly |
| 30 minutes, ~4,500 words | $2.42 | $4.29 | $15, or $12 yearly |
| 1 hour, ~9,000 words | $4.84 | $8.58 | $15, or $12 yearly |

rookey's prices are before the free hours. Wispr Flow Pro costs less only past 2.5 hours of talking every working day, 1.4 on Realtime. Its free plan stops at 2,000 words a week, under an hour of talking a month; ElevenLabs' free hours hold about 40,000 words. On your own machine, rookey costs nothing.
