# Engines

`ROOKEY_BACKEND` picks one:

- `local`, the default: whisper.cpp on this machine, on CUDA, Metal or the CPU, whichever your build has. Needs a model.
- `elevenlabs`: ElevenLabs Scribe. Uploads the clip when you stop: about 1.5 s of waiting for 10 s of speech.
- `elevenlabs-realtime`: streams while you talk and shows the words as they come, about 0.3 s of waiting after you stop.

The ElevenLabs engines need no model, only `ELEVENLABS_API_KEY` in the [keys file](./settings.md).

`rookey setup` downloads the models for `local` from whisper.cpp's page on Hugging Face into the data folder:

| Model | Size | |
|---|---|---|
| Large v3 turbo | 1.5 GB | The default. Close to the largest model in accuracy, several times faster |
| Large v3 turbo, compressed | 547 MB | The same at a third of the size, a little less exact. For less memory |
| Large v3 | 2.9 GB | The largest and the slowest. Wants a GPU |
| Small | 465 MB | Quick without a GPU. More mistakes, above all outside English |
| Base | 141 MB | The quickest, and the least exact |

The settings page lists any ggml model already on disk as well: its own, and those of whisper.cpp and pywhispercpp. `ROOKEY_MODEL` takes the path of any other.

`ROOKEY_LANG` is `auto` (the default), one language like `en`, or several like `en,uk`. The local engine picks the likeliest of the ones you list each time. ElevenLabs takes one language or detects it, so with several it detects.

## What leaves your machine

- With `local`, nothing you say. ElevenLabs gets the audio, your `ROOKEY_WORDS` and any screen terms.
- Screen terms read by `ocr` stay on this machine until they go to ElevenLabs with the audio. `openai` and `anthropic` get the screenshot itself.
- The [update check](./install.md#updates) asks GitHub for the latest release. It is the only call rookey makes without being asked. Models come from Hugging Face when you pick one.
- Transcripts, settings and keys stay on disk. A saved key is never sent back to the settings page, only its last four characters.
