//! The batch Interactions path: a complete clip to `POST /v1beta/interactions`
//! as inline base64 WAV, transcript out.
//!
//! Audio past the API's 30-minute segment limit is silently truncated while
//! the answer still reports `completed`, so a longer clip is split client-side
//! and the segments joined: one transcript the caller sees as one clip.
//! `store` is false, so nothing is kept in Google's interactions history on
//! Voinc's behalf. A smart mode asks Google to punctuate and tidy; a verbatim
//! mode answers word times, which meetings and `voinc transcribe` use.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;

use super::{BATCH_MODEL, Config, GeminiError, INTERACTIONS_URL, MAX_SEGMENT_SAMPLES};
use crate::net;
use crate::transcription::transcript::{Transcript, TranscriptSegment};

/// A request waits at most half its audio again plus 20 s, at least a minute,
/// at most five: a slow answer fails cleanly into the whole-clip fallback
/// instead of holding a paste, and a long file still has room to finish.
fn timeout(samples: usize) -> Duration {
    let seconds = samples as f64 / 16_000.0 * 0.5 + 20.0;
    Duration::from_secs(seconds.clamp(60.0, 300.0) as u64)
}

/// Transcribes a complete clip for its text: smart mode, no timestamps.
pub(crate) fn transcribe_text(samples: &[f32], config: &Config) -> Result<String, GeminiError> {
    if samples.len() <= MAX_SEGMENT_SAMPLES {
        return one_segment(samples, config, false).map(|segment| segment.text);
    }
    let mut text = String::new();
    for chunk in chunks(samples) {
        let segment = one_segment(chunk, config, false)?;
        let piece = segment.text.trim().to_owned();
        if !piece.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&piece);
        }
    }
    Ok(text)
}

/// A complete clip with Google's word times, chunked at the segment limit
/// exactly as [`transcribe_text`] chunks it, so the text matches and each
/// chunk's times sit on the clip's own timeline.
pub(crate) fn transcribe_timed(
    samples: &[f32],
    config: &Config,
) -> Result<Transcript, GeminiError> {
    if samples.len() <= MAX_SEGMENT_SAMPLES {
        return one_segment(samples, config, true);
    }
    let mut text = String::new();
    let mut words: Vec<TranscriptSegment> = Vec::new();
    for (index, chunk) in chunks(samples).enumerate() {
        let offset_ms = index as i64 * MAX_SEGMENT_SAMPLES as i64 * 1000 / 16_000;
        let segment = one_segment(chunk, config, true)?;
        let piece = segment.text.trim().to_owned();
        if !piece.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&piece);
        }
        words.extend(segment.words.into_iter().map(|mut word| {
            word.start_ms += offset_ms;
            word.end_ms += offset_ms;
            word
        }));
    }
    Ok(Transcript {
        text,
        segments: Vec::new(),
        words,
    })
}

/// The clip's pieces, each at most the API's segment limit.
fn chunks(samples: &[f32]) -> impl Iterator<Item = &[f32]> {
    samples.chunks(MAX_SEGMENT_SAMPLES)
}

/// One whole request under [`MAX_SEGMENT_SAMPLES`] samples. A timed request
/// is verbatim with word times; an untimed one is smart.
fn one_segment(samples: &[f32], config: &Config, timed: bool) -> Result<Transcript, GeminiError> {
    let body = serde_json::json!({
        "model": BATCH_MODEL,
        "input": [{
            "type": "audio",
            "data": base64::engine::general_purpose::STANDARD
                .encode(super::wav_bytes(samples)),
            "mime_type": "audio/wav",
        }],
        "generation_config": {"transcription_config": transcription_config(config, timed)},
        "store": false,
    });
    let interaction = send(
        &serde_json::to_vec(&body).unwrap_or_default(),
        config,
        samples,
    )?;
    Ok(interaction.transcript())
}

/// The transcription options one request carries: the language hint, the
/// vocabulary, and the mode. A timed request answers word times; an untimed
/// one asks Google to punctuate and tidy.
fn transcription_config(config: &Config, timed: bool) -> serde_json::Value {
    let mut options = serde_json::Map::new();
    if let Some(language) = &config.language {
        options.insert("language_codes".into(), serde_json::json!([language]));
    }
    if !config.vocabulary.is_empty() {
        options.insert(
            "custom_vocabulary".into(),
            serde_json::json!(config.vocabulary),
        );
    }
    options.insert(
        "mode".into(),
        if timed {
            serde_json::json!({
                "type": "verbatim",
                "timestamp_granularities": ["word"],
            })
        } else {
            serde_json::json!({"type": "smart"})
        },
    );
    serde_json::Value::Object(options)
}

/// One interaction's answer.
#[derive(Deserialize)]
struct Interaction {
    status: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
    output_text: Option<String>,
}

#[derive(Deserialize)]
struct Step {
    #[serde(default)]
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    #[serde(default)]
    annotations: Vec<Annotation>,
}

/// A text content's annotations; only a `word_info` one carries what a
/// timed transcript needs.
#[derive(Deserialize)]
struct Annotation {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    start_offset: Option<String>,
    end_offset: Option<String>,
}

impl Interaction {
    /// The clip's text: the answer's `output_text` when the API names one,
    /// else every text content in step order.
    fn text(&self) -> String {
        if let Some(text) = &self.output_text {
            return text.clone();
        }
        let mut text = String::new();
        for step in &self.steps {
            for content in &step.content {
                if content.kind.as_deref() != Some("text") {
                    continue;
                }
                let Some(piece) = content.text.as_deref() else {
                    continue;
                };
                let piece = piece.trim();
                if piece.is_empty() {
                    continue;
                }
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(piece);
            }
        }
        text
    }

    /// The answer as a transcript: its text plus every word annotation's
    /// time, in order.
    fn transcript(&self) -> Transcript {
        let mut words = Vec::new();
        for step in &self.steps {
            for content in &step.content {
                for annotation in &content.annotations {
                    if annotation.kind.as_deref() != Some("word_info") {
                        continue;
                    }
                    let (Some(start), Some(end)) = (annotation.start_ms(), annotation.end_ms())
                    else {
                        continue;
                    };
                    let Some(word) = annotation.text.as_deref().map(str::to_owned) else {
                        continue;
                    };
                    words.push(TranscriptSegment {
                        start_ms: start,
                        end_ms: end,
                        text: word,
                    });
                }
            }
        }
        words.sort_by_key(|word| word.start_ms);
        Transcript {
            text: self.text(),
            segments: Vec::new(),
            words,
        }
    }

    /// Whether the interaction finished.
    fn completed(&self) -> bool {
        self.status.as_deref().is_none_or(|status| {
            matches!(status, "completed" | "incomplete" | "requires_continuation")
        })
    }
}

impl Annotation {
    /// The word's start, in milliseconds, from a decimal-seconds offset such
    /// as `10.5s`.
    fn start_ms(&self) -> Option<i64> {
        Self::offset_ms(self.start_offset.as_deref())
    }

    fn end_ms(&self) -> Option<i64> {
        Self::offset_ms(self.end_offset.as_deref())
    }

    fn offset_ms(offset: Option<&str>) -> Option<i64> {
        let seconds: f64 = offset?.trim().trim_end_matches('s').parse().ok()?;
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        Some((seconds * 1000.0).round() as i64)
    }
}

/// Posts one request and reads its interaction, naming every failure for a
/// person. The body carries the audio, so the timeout covers a slow upload.
fn send(body: &[u8], config: &Config, samples: &[f32]) -> Result<Interaction, GeminiError> {
    let agent = net::agent(INTERACTIONS_URL, |config| {
        config
            .http_status_as_error(false)
            .timeout_global(Some(timeout(samples.len())))
    });
    let request = ureq::http::Request::builder()
        .method("POST")
        .uri(INTERACTIONS_URL)
        .header("x-goog-api-key", &config.key)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .map_err(|error| GeminiError::Invalid(error.to_string()))?;
    let mut response = agent
        .run(request)
        .map_err(|error| GeminiError::Unreachable(net::reason(&error)))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| GeminiError::Unreachable(net::reason(&error)))?;
    let body = String::from_utf8_lossy(&body);
    if !(200..300).contains(&status) {
        return Err(super::error_for_status(status, &body));
    }
    let interaction: Interaction = serde_json::from_str(body.trim())
        .map_err(|error| GeminiError::Server(format!("an unreadable answer ({error})")))?;
    if !interaction.completed() {
        let status = interaction.status.clone().unwrap_or_default();
        return Err(GeminiError::Server(format!(
            "the transcription did not finish ({status})"
        )));
    }
    Ok(interaction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_carry_milliseconds() {
        assert_eq!(Annotation::offset_ms(Some("10.5s")), Some(10_500));
        assert_eq!(Annotation::offset_ms(Some("30s")), Some(30_000));
        assert_eq!(Annotation::offset_ms(Some("0.01s")), Some(10));
        assert_eq!(Annotation::offset_ms(Some("-1s")), None);
        assert_eq!(Annotation::offset_ms(Some("soon")), None);
        assert_eq!(Annotation::offset_ms(None), None);
    }

    #[test]
    fn text_prefers_the_named_output_text() {
        let interaction: Interaction = serde_json::from_str(
            r#"{"status":"completed","output_text":"hello there",
                "steps":[{"content":[{"type":"text","text":"old"}]}]}"#,
        )
        .unwrap();
        assert_eq!(interaction.text(), "hello there");
    }

    #[test]
    fn text_joins_step_contents() {
        let interaction: Interaction = serde_json::from_str(
            r#"{"status":"completed","steps":[
                {"content":[{"type":"text","text":"Hello"},{"type":"audio"}]},
                {"content":[{"type":"text","text":"  "},{"type":"text","text":"there."}]}]}"#,
        )
        .unwrap();
        assert_eq!(interaction.text(), "Hello there.");
    }

    #[test]
    fn a_word_annotation_is_a_timed_word() {
        let interaction: Interaction = serde_json::from_str(
            r#"{"status":"completed","steps":[{"content":[
                {"type":"text","text":"Hello",
                 "annotations":[{"type":"word_info","text":"Hello",
                                 "start_offset":"0.2s","end_offset":"0.6s"},
                                {"type":"url_citation","url":"https://example.com"}]}]}]}"#,
        )
        .unwrap();
        let transcript = interaction.transcript();
        assert_eq!(transcript.words.len(), 1);
        assert_eq!(transcript.words[0].start_ms, 200);
        assert_eq!(transcript.words[0].end_ms, 600);
        assert_eq!(transcript.words[0].text, "Hello");
    }

    #[test]
    fn a_non_completed_status_is_a_server_failure() {
        let interaction: Interaction =
            serde_json::from_str(r#"{"status":"failed","steps":[]}"#).unwrap();
        assert!(!interaction.completed());
    }

    #[test]
    fn timeout_bounds_hold() {
        // A 30-minute clip.
        assert_eq!(timeout(30 * 60 * 16_000), Duration::from_secs(300));
        // A moment of audio still waits a minute.
        assert_eq!(timeout(16_000), Duration::from_secs(60));
    }
}
