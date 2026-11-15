//! The one live-stream seam a transcription engine drives: whatever a
//! capture feeds samples to and polls interim text from, then finalizes.
//!
//! The engine's worker owns the stream's lifecycle (begin, feeds, finish,
//! whole-clip fallback); the runtime behind the trait only answers those
//! calls. The GGUF stream ([`transcribe_cpp::Stream`]) implements it in
//! [`crate::transcription::engine::streaming`], and Google Transcribe's Live
//! session in [`crate::transcription::gemini::live`].

use crate::transcription::engine::Partial;

/// A live stream as the engine's worker drives it.
pub(crate) trait StreamFeed {
    /// Feeds one block of the clip's 16 kHz mono samples.
    fn feed(&mut self, samples: &[f32]) -> Result<(), String>;

    /// Finalizes the stream and answers its whole text.
    fn finalize(&mut self) -> Result<String, String>;

    /// The stream's text so far, for live text.
    fn partial(&self) -> Partial;

    /// Where the stream's text starts in the audio it heard, in milliseconds,
    /// once it has finalized; `None` when the model gives no timing.
    fn text_start_ms(&self) -> Option<u64> {
        None
    }

    /// Where the stream's text ends in the audio it heard, in milliseconds,
    /// once it has finalized; `None` when the model gives no timing.
    fn text_end_ms(&self) -> Option<u64> {
        None
    }

    /// The distinct language tags the model's raw decode carries, in order of
    /// first appearance, once it has finalized; empty when it emits none.
    fn language_tags(&self) -> Vec<String> {
        Vec::new()
    }
}
