# Google Transcribe

Google Transcribe (Labs › Google Transcribe, off by default) transcribes with
Google's Gemini speech models in the cloud, using the person's own Gemini API
key. It is a runtime beside the local GGUF models and Apple Speech, not a
download: the picker offers **Google Transcribe (Gemini)** while the toggle is
on and a key answers, and a saved choice of it stays saved while it can't run.
While Voinc listens, it keeps running its usual fallback
(`custom_models::runnable`); starting from launch, a Linux pending listen
holds at Model required and a save that makes the model installed — the key,
its only install — starts it, since Google Transcribe never prepares.
**Your audio goes to Google.** The Labs card's description and the picker row
say so.

```ts
Labs > Google Transcribe (off)                       // labs.google_transcribe
├── Toggle on -> the card shows an "API key" field (drafted while typed,
│   saved with Apply; `google_transcribe.api_key`, or the environment's
│   VOINC_GOOGLE_API_KEY, read again at every run) and, while on, a
│   "Skip live streaming" switch (`google_transcribe.batch_only`)
├── Skip live streaming (on) -> no live session at all: each dictation
│   transcribes the whole clip by batch after it ends; nothing shows while
│   it records
├── Picker -> "Google Transcribe (Gemini)" · Cloud · every language · streams;
│   size reads "Runs in Google's cloud"; no download ever starts
├── Dictate (hold) -> the capture rides one Gemini Live WebSocket session:
│   setup (model, language hint, custom vocabulary, SMART mode), then 16 kHz
│   mono PCM fed as base64; interim text replaces the words so far and each
│   turn's final text appends
│   ├── Any stream failure — refused setup, a stalled or capped connection, a
│   │   missed finish — is an ordinary stream error, and the whole clip
│   │   decides, transcribed by batch as every runtime's fallback does
│   └── Ten minutes after it opened the session stops a little early (9:30)
│       so the last turn finalizes; a longer capture's stream fails and the
│       whole clip decides
├── Finish (release) -> the stream finalizes with 0.4 s of trailing silence
│   (a GGUF stream does the same, so a last word still lands); the complete
│   clip is authoritative, as with every runtime
├── Meetings -> each pause-cut chunk is transcribed by batch, word-timed;
│   macOS loads its own transcriber, Linux and Windows share the engine
└── Labs › Transcribe files -> `voinc transcribe <file>` runs batch over the
    file; clips longer than 30 minutes are split at that limit client-side,
    because audio past it is silently truncated while the answer still
    reports completed
```

## The key

The key is the person's: the saved setting first (`voinc settings set
google_transcribe.api_key …`, or the field on the Labs card, serialized as
`null` while unset), else the `VOINC_GOOGLE_API_KEY` environment variable.
Either is trimmed; an empty one is no key. Google Transcribe is prepared —
nothing to download — while its key answers, and `voinc model use
gemini_transcribe` selects it under the same rule. A request with no key fails
before anything is sent, named for a person; a refused key, a rate limit, and
Google's HTML error pages are named for a person too. Requests and
WebSockets go out like every other Voinc request: the platform's TLS and
roots, and the proxy the computer uses — a SOCKS proxy refuses the live
session (the batch request still goes out).

## Batch and Live

Batch is one REST call per clip over the Interactions API: the clip as inline
base64 mono WAV, `store: false` so nothing is kept in Google's interactions
history on Voinc's behalf. A dictation's processed text is wanted as plain
text (`SMART`); a meeting chunk or a timed file asks for verbatim text with
word timestamps, and the answer's `word_info` annotations carry each word and
its start and end in decimal seconds, read as milliseconds. The deadline is
the clip's length plus slack, clamped to a minute and five.

Live is one WebSocket per streaming dictation capture, driven by the engine's
own stream machinery: the engine feeds it while the capture runs, a finish
finalizes, and every rule the GGUF streams follow — deadlines, falling back
to the whole clip, nothing pasting before the final text — applies unchanged,
because it is the same code with a different transport under it. Google's
Live API does not give word timestamps; interim text replaces and turn text
appends, so the fallback's whole clip supplies anything the session dropped.

## Batch only

The Labs card's **Skip live streaming** switch (`google_transcribe.batch_only`,
off by default) leaves the live session out of dictation entirely: the engine
opens no stream, so every capture finishes as a whole clip transcribed by
batch, and nothing shows while it records. The choice is read again at every
capture, like the key, and applies without a model reload. Where the live
session cannot connect — a SOCKS proxy refuses it, and the batch request
still goes out — this spares every capture the refused attempt before its
fallback. Meetings, `voinc transcribe`, and the whole-clip fallback already
run batch and are unaffected; only the dictation capture's stream is.

## Vocabulary

The saved vocabulary list rides along as the request's `customVocabulary`
(at most 1000 terms), read at the start of each run so a settings change
applies to the next one — only while Labs › Vocabulary biasing is on and the
boost is not off, exactly the gate Parakeet's bias keeps
(`vocabulary::transcription_terms`, which also carries a focused mode's own
terms while that dictation records).

## Free tier

The free tier keeps some content to improve Google products and bounds
requests per minute and per day (the day resets at midnight Pacific); the
Live API allows a few sessions at once and caps one at ten minutes. Every
request Voinc sends carries `store: false`. A rate limit is named for a
person, not shown as a raw status.

## Checks

`missing_key_is_reported_without_a_request`,
`a_refused_key_is_named_for_a_person`, `a_rate_limit_says_what_to_do`, and
`an_html_error_page_still_names_a_reason` in
[gemini.rs](../../src/transcription/gemini.rs);
`wav_bytes_is_a_well_formed_mono_wav`, `offsets_carry_milliseconds`,
`text_prefers_the_named_output_text`, `text_joins_step_contents`,
`a_word_annotation_is_a_timed_word`, `a_non_completed_status_is_a_server_failure`,
and `timeout_bounds_hold` in
[batch.rs](../../src/transcription/gemini/batch.rs) — every one against
fixture JSON and in-memory sockets; no request ever left the machine.
`gemini_transcribe_is_offered_only_with_its_lab_and_key` in
[models.rs](../../src/transcription/models.rs) holds the picker's rule, and
the engine's stream tests
(`a_stream_that_dropped_a_phrase_after_a_pause_falls_back_to_the_whole_clip`
and the rest of
[engine.rs](../../src/transcription/engine.rs)) drive the same
`StreamFeed` seam the Live session rides with a fake transport.
`batch_only_leaves_no_stream_options` in
[gemini.rs](../../src/transcription/gemini.rs) holds the Skip live streaming
gate, and `google_transcribe_batch_only_round_trips` in
[app_settings.rs](../../src/app_settings.rs) its setting.
`a_saved_gemini_choice_waits_pending_until_its_key_arrives` in
[linux/app.rs](../../src/linux/app.rs) holds the launch gate's pending state
and the key-as-install rule.

Observed on a real Linux desktop (GNOME Wayland, x86_64): selecting
`gemini_transcribe` with a key through `voinc model use` switches at once, a
saved choice without a key survives and holds the pending listen at Model
required, and saving the key starts the pending listener without a
preparation. Not yet observed, and not covered by any check: a real request
to Google.
The Live session's state machine (its reads between writes, tunnel
timeouts, and turn assembly) has no unit tests — the engine's stream rules
around it do. No macOS meeting with Gemini selected has been recorded, and
`voinc transcribe` over a real long file (segment splitting) is untested
against Google's truncation behavior.
