# Google Transcribe

Google Transcribe (Labs › Google Transcribe, off by default) transcribes with
Google's Gemini speech models in the cloud, using the person's own Gemini API
key. It is a runtime beside the local GGUF models and Apple Speech, not a
download: the picker offers **Google Transcribe (Gemini)** while the toggle is
on — even before a key arrives, the row names the key instead of hiding the
model — and a saved choice of it stays saved while it can't run.
While Voinc listens, it keeps running its usual fallback
(`custom_models::runnable`); starting from launch, a Linux pending listen
holds at Model required and a save that makes the model installed — the key,
its only install — starts it, since Google Transcribe never prepares.
**Your audio goes to Google.** The Labs card's description and the picker row
say so.

```ts
Labs > Google Transcribe (off)                       // labs.google_transcribe
├── Toggle on -> the card shows an "API key" field (drafted while typed and
│   committed once typing pauses — the shared draft save every text field
│   uses, so the first key saves through the field and a cleared field
│   clears it; the field shows dots, the key lives in the OS keychain and
│   stays out of settings.json unless the keychain is unavailable, and the
│   environment's VOINC_GOOGLE_API_KEY answers first at every run) and,
│   while on, a "Skip live streaming" switch (`google_transcribe.batch_only`)
├── Skip live streaming (on) -> no live session at all: each dictation
│   transcribes the whole clip by batch after it ends; nothing shows while
│   it records
├── Picker -> "Google Transcribe (Gemini)" · Cloud · every language · streams;
│   size reads "Runs in Google's cloud"; no download ever starts. While the
│   toggle is on but no key answers, the row is still listed — it names the
│   key in amber ("Add your Gemini API key in Labs › Google Transcribe")
│   and takes no action button
├── General › Speech model (Google Transcribe selected, no key) -> the row's
│   description is that same key note, in place of the download offer; the
│   picker value button stays
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

The key is the person's, and it lives in the OS keychain — the login keychain
on macOS, Credential Manager on Windows, the Secret Service of the session bus
on Linux (`transcription::gemini::key_store`, over zbus and Rust crypto so no
system dbus library is needed) — never in settings.json: the saved setting
(`google_transcribe.api_key`) is only the in-memory carrier, re-seeded from
the keychain at every load and left out of every save. `voinc settings get`
and `list` show whether one is saved ("saved in your system's keychain"),
never the key itself. While the keychain is unavailable the key stays in the
file instead — a save is never blocked by it — and a later load or save moves
it in. A key found in an older file moves into the keychain once
(`google-key-in-keychain` migration). At request time the
`VOINC_GOOGLE_API_KEY` environment variable answers first, else the saved
key. The field on the Labs card shows dots (masked) and is drafted while
typed and committed once typing pauses — the shared draft save every text
field uses — so a pasted first key saves through the field and a cleared
field clears it; either is trimmed, and an empty one is no key. Google
Transcribe is prepared — nothing to download — while its key answers, and
`voinc model use gemini_transcribe` selects it under the same rule. A request
with no key fails before anything is sent, named for a person; a refused
key, a rate limit, and Google's HTML error pages are named for a person too.
Requests and WebSockets go out like every other Voinc request: the platform's
TLS and roots, and the proxy the computer uses — a SOCKS proxy refuses the
live session (the batch request still goes out).

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
Google sends every live event — `setupComplete` included — as a binary
WebSocket frame whose payload is the events' JSON, and accepts the same
JSON in text frames going out; the session parses both frame types the
same way (`handle_event`). This live model ends its turns with
`generationComplete` rather than `turnComplete`, arriving right after the
utterance's finalized `inputTranscription`; a turn whose text only ever
arrived as speculation is committed by the turn end, and `generationComplete`
after the stream end settles the session with the whole capture's text. The
quiet rule — settling 500 ms after Google's last event — remains the
fallback when no turn end arrives. Measured on the wire (2026-11-16, probe):
the last utterance's commit lands ~0.9–1.5 s after `audioStreamEnd` and
Google never closes the socket, so the finalize waits up to 2.5 s — a 1.2 s
wait missed every capture whose last utterance was still being transcribed
at the release and fell back to the whole clip by batch.

## Batch only

The Labs card's **Skip live streaming** switch (`google_transcribe.batch_only`,
off by default) leaves the live session out of dictation entirely: the engine
opens no stream, so every capture finishes as a whole clip transcribed by
batch, and nothing shows while it records. The choice is read again at every
capture, like the key, and applies without a model reload: saving it asks the
engine to re-read whether it can stream now (`Command::PublishStreams`), so
the next capture follows the new choice instead of the last command's.
Where the live
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

## In the logs

The Gemini paths say so in `process.log`: loading the runtime logs
`loaded Google Transcribe; transcription goes to Google` (with the batch
model and language), and every batch request logs `transcribed with Google`
or `Google Transcribe request failed` with the host, its duration, and its
byte counts — never any text — so whether a dictation went to Google reads
straight from the logs. A saved model that cannot run — its key is gone, or
its lab is off — is named where the listener falls back
(`custom_models::runnable`): `the saved transcription model cannot run;
running the default until it can`.

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
`gemini_transcribe_is_listed_while_its_lab_is_on_even_without_its_key` there
holds the keyless listing: the row is listed while its lab is on and no key
answers, read through `labs::google_transcribe_in(&settings)` so the render
consults the window's own settings rather than the applied state (the
goldens `rune-picker-google-light` and `rune-google-settings-light`, scenes
`picker-google` and `google-settings`, pin both renders). The engine's stream tests
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
and the key-as-install rule. `the_labs_api_key_draft_saves_and_clears` in
[rune_ui/tests.rs](../../src/rune_ui/tests.rs) holds the card's field itself:
seeded at the store's open (an unopened store projects nothing), a pasted
first key commits through the shared draft save, a later open reseeds from
the saved key, and clearing the field clears it. The key's storage is held by
`the_gemini_key_moves_into_the_keychain_once`, `a_saved_gemini_key_stays_out_of_the_file`,
`an_unavailable_keychain_keeps_the_gemini_key_in_the_file`, and
`a_migrated_file_is_not_rewritten_by_later_loads` in
[persistence/tests.rs](../../src/app_settings/persistence/tests.rs);
`the_gemini_key_is_never_printed` in
[control/settings.rs](../../src/control/settings.rs) holds the redaction; and
[key_store.rs](../../src/transcription/gemini/key_store.rs) unit-tests the
store access itself — prime, sync on change, clear, holds, the unavailable
store keeping the key in the file, and an empty key being no key — every one
against an in-memory recorder: no test ever opens the real keychain.

Observed on a real Linux desktop (GNOME Wayland, x86_64): selecting
`gemini_transcribe` with a key through `voinc model use` switches at once, a
saved choice without a key survives and holds the pending listen at Model
required, and saving the key starts the pending listener without a
preparation. A real request to Google is observed too (2026-11-16, same
desktop): dictating with Google Transcribe selected transcribes the capture
on the live session — the log reads `transcribed Linux dictation
source="stream"`, the stream finalizes in a few hundred milliseconds where
the batch fallback took whole seconds, and the pasted text is coherent. The
binary-frame rule was found first with a throwaway probe client against the
real endpoint (every live event rides one; the session parsed only text
frames, so setup never completed and every capture fell back to batch); the
probe stays out of the tree.
Still not observed: a macOS meeting with Gemini selected, `voinc
transcribe` over a real long file (segment splitting against Google's
truncation behavior), and the free tier's rate limit. The Live session's
state machine (its reads between writes, tunnel timeouts, and turn
assembly) has no unit tests — the engine's stream rules around it do.
