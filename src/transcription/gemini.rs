//! Google Transcribe: Gemini's speech-to-text over the Gemini API.
//!
//! Two ways to hear it:
//!
//! - [`batch`] transcribes a complete clip over the Interactions REST API
//!   (inline base64 WAV): dictation fallbacks, meeting chunks, and
//!   `voinc transcribe` files. Long clips are split at Google's 30-minute
//!   segment limit, because audio past it is silently truncated while the
//!   answer still reports `completed`.
//! - [`live`] holds one Live API WebSocket session for a dictation's interim
//!   text. Any stream failure falls back to the whole clip, which [`batch`]
//!   then transcribes.
//!
//! Auth is the person's Gemini API key: the saved setting first, else
//! `VOINC_GOOGLE_API_KEY`. Requests and WebSockets go out like every other
//! Voinc request: the platform's TLS and roots, and the proxy the computer
//! uses. Errors are named for a person.
//!
//! The free tier keeps some content to improve Google products and bounds
//! requests (per minute and per day), the Live API allows a few sessions at
//! once and caps one at 10 minutes; every `store` is false, so nothing is
//! kept in Google's interactions history on Voinc's behalf.

pub(crate) mod batch;
pub(crate) mod live;

use std::sync::{Arc, OnceLock, RwLock};

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::Deserialize;

use crate::transcription::models::{TranscriptionModelId, TranscriptionSelection, validate};
use crate::transcription::stream_feed::StreamFeed;
use crate::transcription::transcript::Transcript;

/// The batch model, as the Interactions API names it.
pub(crate) const BATCH_MODEL: &str = "gemini-3.5-transcribe";
/// The Live API's streaming model, under `models/`.
pub(crate) const LIVE_MODEL: &str = "models/gemini-3.5-transcribe-live";
/// The API's origin; its host is also where the WebSocket connects.
pub(crate) const API_HOST: &str = "generativelanguage.googleapis.com";
/// The Interactions endpoint a whole clip goes to.
pub(crate) const INTERACTIONS_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/interactions";
/// The Live API's WebSocket endpoint, authenticated by an `x-goog-api-key`
/// header on the handshake.
pub(crate) const WS_URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
/// The custom vocabulary a request biases toward, at most 1000 terms.
pub(crate) const MAX_VOCABULARY: usize = 1000;
/// A batch request carries at most 30 minutes of audio. Audio past the limit
/// is silently truncated while the answer still reports `completed`, so a
/// longer clip is split client-side before anything is sent.
pub(crate) const MAX_SEGMENT_SAMPLES: usize = 30 * 60 * 16_000;

/// A live dictation stream hears silence after its clip, as a GGUF stream
/// does: a final turn's last word must land.
pub(crate) const LIVE_TRAILING_SILENCE_SAMPLES: usize = 6_400;

/// The streaming capability `stream_options` reports for this runtime: the
/// engine's stream machinery is what a Live session rides. The options
/// themselves are a GGUF artifact a Live session ignores.
pub(crate) static STREAM_OPTIONS: std::sync::LazyLock<transcribe_cpp::StreamOptions> =
    std::sync::LazyLock::new(transcribe_cpp::StreamOptions::default);

/// The API key the running settings hold, set on every save and load.
static API_KEY: OnceLock<RwLock<Option<Arc<str>>>> = OnceLock::new();

/// The Gemini API key for Google Transcribe: `VOINC_GOOGLE_API_KEY` first,
/// then the saved setting. `None` means Google Transcribe cannot run.
pub(crate) fn api_key() -> Option<String> {
    let from_env = std::env::var("VOINC_GOOGLE_API_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    from_env.or_else(|| {
        API_KEY
            .get_or_init(Default::default)
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|key| key.to_string())
    })
}

/// Records the saved key for [`api_key`]; called on every settings load and
/// save, so a change applies to the next request.
pub(crate) fn set_api_key(key: Option<String>) {
    let key = key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
        .map(Arc::from);
    *API_KEY
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = key;
}

/// What one transcription asks Google for: the key, the language hint, and
/// the vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Config {
    key: String,
    /// The BCP-47 hint the selection names; `None` lets Google detect the
    /// language.
    language: Option<String>,
    /// The saved vocabulary, at most [`MAX_VOCABULARY`] terms.
    vocabulary: Vec<String>,
}

impl Config {
    /// Reads the key and vocabulary as the run starts, so a settings change
    /// applies to the next transcription.
    fn for_selection(selection: &TranscriptionSelection) -> Result<Self> {
        let key = api_key().ok_or_else(|| eyre!("{}", GeminiError::NoKey.to_string()))?;
        let language = (selection.language != crate::transcription::models::AUTO_LANGUAGE)
            .then(|| selection.language.clone());
        // Labs › Vocabulary biasing gates the list, as it does for Parakeet;
        // Google's customVocabulary carries at most MAX_VOCABULARY terms.
        let mut vocabulary = crate::dictation::vocabulary::transcription_terms();
        vocabulary.truncate(MAX_VOCABULARY);
        Ok(Self {
            key,
            language,
            vocabulary,
        })
    }
}

/// Why Google Transcribe could not answer, named for a person.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum GeminiError {
    /// No key is saved or set.
    NoKey,
    /// The key was refused.
    Auth,
    /// The model name was refused: this build predates it, or it left the API.
    UnknownModel,
    /// The request was rejected: audio or options Google wouldn't take.
    Invalid(String),
    /// The free tier's limits were hit.
    RateLimited(String),
    /// Google's service failed.
    Server(String),
    /// The network failed before an answer.
    Unreachable(String),
}

impl GeminiError {
    /// A person-named reason for a failed request.
    fn reason(&self) -> String {
        match self {
            Self::NoKey => "Google Transcribe has no API key. Paste one in Settings › \
                            Labs › Google Transcribe."
                .into(),
            Self::Auth => {
                "Google rejected the API key. Check it in Settings › Labs › Google Transcribe."
                    .into()
            }
            Self::UnknownModel => {
                "Google doesn't know the speech model this build asks for; check for an update."
                    .into()
            }
            Self::Invalid(detail) => format!("Google rejected the audio or options: {detail}"),
            Self::RateLimited(detail) => {
                format!(
                    "Google Transcribe is rate-limited ({detail}); its free tier allows only a \
                     few transcriptions per minute. Try again shortly."
                )
            }
            Self::Server(detail) => format!("Google's transcription service failed: {detail}"),
            Self::Unreachable(detail) => {
                format!("Couldn't reach {API_HOST}: {detail}")
            }
        }
    }

    /// The failure for the engine's whole-clip fallback, named for a person.
    pub(crate) fn fallback_reason(&self) -> String {
        self.reason()
    }
}

impl std::fmt::Display for GeminiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason())
    }
}

impl std::error::Error for GeminiError {}

/// The Google Transcribe runtime a listener or the macOS engine holds: a
/// selection plus what each request reads afresh. There are no weights, so
/// loading is cheap and prewarming does nothing.
pub(crate) struct Gemini {
    selection: TranscriptionSelection,
}

impl Gemini {
    pub(crate) fn load_selection(selection: &TranscriptionSelection) -> Result<Self> {
        validate(selection)?;
        Config::for_selection(selection)?;
        Ok(Self {
            selection: selection.clone(),
        })
    }

    pub(crate) fn selection(&self) -> &TranscriptionSelection {
        &self.selection
    }

    pub(crate) fn model_id(&self) -> TranscriptionModelId {
        self.selection.model
    }

    pub(crate) fn matches_selection(&self, selection: &TranscriptionSelection) -> bool {
        &self.selection == selection
    }

    /// Nothing to warm: every request reads its key and vocabulary afresh.
    pub(crate) fn prewarm(&mut self) -> Result<()> {
        Ok(())
    }

    fn config(&self) -> Result<Config> {
        Config::for_selection(&self.selection).wrap_err("Google Transcribe is not ready")
    }

    /// Transcribes a complete 16 kHz clip.
    pub(crate) fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        let config = self.config()?;
        batch::transcribe_text(samples, &config).map_err(|error| eyre!("{}", error.reason()))
    }

    /// A complete clip with Google's word times, chunked at the 30-minute
    /// segment limit exactly as the untimed run chunks it, so the text
    /// matches.
    pub(crate) fn transcribe_timed(&mut self, samples: &[f32]) -> Result<Transcript> {
        let config = self.config()?;
        batch::transcribe_timed(samples, &config).map_err(|error| eyre!("{}", error.reason()))
    }

    /// One meeting chunk with Google's word times.
    pub(crate) fn transcribe_segments(&mut self, samples: &[f32]) -> Result<Transcript> {
        self.transcribe_timed(samples)
    }

    /// Opens a Live session and drives it. A session Google won't start is a
    /// logged backend error; the engine's queued finish then takes the whole
    /// clip, which `transcribe` answers with a batch request.
    pub(crate) fn with_stream<R>(
        &mut self,
        _stream_options: &transcribe_cpp::StreamOptions,
        cancel: &transcribe_cpp::CancelToken,
        drive: impl FnOnce(&mut dyn StreamFeed) -> R,
    ) -> transcribe_cpp::Result<R> {
        let config = self
            .config()
            .map_err(|error| transcribe_cpp::Error::Backend(format!("{error:#}")))?;
        let mut stream = live::GeminiLiveStream::open(&config, cancel)
            .map_err(|error| transcribe_cpp::Error::Backend(error.fallback_reason()))?;
        Ok(drive(&mut stream))
    }

    /// One word or segment's start and end (ms from the clip's start) and
    /// text, as `transcribe_words` and the capture path read them.
    pub(crate) fn transcribe_words(&mut self, samples: &[f32]) -> Result<(String, Vec<TimedSpan>)> {
        let transcript = self.transcribe_timed(samples)?;
        let spans = if transcript.words.is_empty() {
            transcript.segments.clone()
        } else {
            transcript.words.clone()
        };
        let words = spans
            .into_iter()
            .map(|word| (word.start_ms, word.end_ms, word.text))
            .collect();
        Ok((transcript.text, words))
    }
}

/// A word or segment's start and end (ms from the clip's start) and text.
pub(crate) type TimedSpan = (i64, i64, String);

/// The Google error body a failed request answers.
#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

/// The failure detail a non-2xx answer carries, as a person-named
/// [`GeminiError`].
pub(crate) fn error_for_status(status: u16, body: &str) -> GeminiError {
    let detail = serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    match status {
        401 | 403 => GeminiError::Auth,
        404 => GeminiError::UnknownModel,
        400 => GeminiError::Invalid(detail),
        429 => GeminiError::RateLimited(detail),
        500..=599 => GeminiError::Server(format!("HTTP {status}: {detail}")),
        _ => GeminiError::Server(format!("HTTP {status}: {detail}")),
    }
}

/// Encodes 16 kHz mono f32 samples as the WAV bytes a request carries.
pub(crate) fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&16_000_u32.to_le_bytes());
    bytes.extend_from_slice(&32_000_u32.to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2_u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16_u16.to_le_bytes()); // bits
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&f32_to_i16(*sample).to_le_bytes());
    }
    bytes
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_bytes_is_a_well_formed_mono_wav() {
        let bytes = wav_bytes(&[0.0, 1.0, -1.0, 0.5]);
        let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / i16::MAX as f32)
            .collect();
        assert_eq!(samples.len(), 4);
        assert!(samples[0] < 0.001);
        assert!(samples[1] > 0.99);
        assert!(samples[2] < -0.99);
    }

    #[test]
    fn a_refused_key_is_named_for_a_person() {
        let error = error_for_status(403, r#"{"error":{"message":"API key not valid"}}"#);
        assert!(matches!(error, GeminiError::Auth));
        assert!(error.reason().contains("API key"));
    }

    #[test]
    fn a_rate_limit_says_what_to_do() {
        let error = error_for_status(429, r#"{"error":{"message":"resource exhausted"}}"#);
        let reason = error.reason();
        assert!(reason.contains("rate-limited"), "{reason}");
    }

    #[test]
    fn an_html_error_page_still_names_a_reason() {
        let error = error_for_status(502, "<html>Bad Gateway</html>");
        let reason = error.reason();
        assert!(reason.contains("service failed"), "{reason}");
    }

    #[test]
    fn missing_key_is_reported_without_a_request() {
        assert!(!GeminiError::NoKey.reason().is_empty());
    }
}
