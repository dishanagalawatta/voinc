//! The Live session: one WebSocket per streaming dictation capture.
//!
//! The engine's worker feeds 16 kHz audio while the capture runs and finalizes
//! at its end; interim text shows beside the HUD, and the finalize answers the
//! whole capture's text. Any session failure — refused setup, a stalled or
//! capped connection, a missed finish — is an ordinary stream error, and the
//! engine's queued finish takes the whole clip, which the batch path
//! ([`super::batch`]) transcribes. Nothing is lost but latency.
//!
//! One thread owns the WebSocket: it writes the audio the feeds queue and
//! reads Google's events between writes, with a short read timeout as the
//! loop's pacing. Google's hybrid VAD hears speech starts on its own, and the
//! stream end (`audioStreamEnd`) asks it to finalize the last turn at once.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use tungstenite::Message;

use super::{API_HOST, Config, GeminiError, LIVE_MODEL, WS_URL};
use crate::net;
use crate::transcription::engine::Partial;
use crate::transcription::stream_feed::StreamFeed;

/// One session message carries 100 ms of audio (1024–2048 frames, as the
/// Live transcription guide recommends).
const CHUNK_SAMPLES: usize = 1_600;
/// The chunks in flight to the session thread before a feed drops audio: a
/// stalled connection must never wedge the engine's worker, and the whole
/// clip decides at the finish.
const AUDIO_QUEUE: usize = 64;
/// How long a capture allows the session to connect, authenticate, and
/// complete its setup before the stream is refused.
const CONNECT_BUDGET: Duration = Duration::from_secs(10);
/// How long finalize waits for Google's last turn after the stream ends. The
/// engine's caller gives a stream finish 1.5 s and takes the whole clip when
/// it misses, so this fails first and the engine thread is free to run the
/// whole clip.
/// How long the finalize waits for Google's last turn to settle. On the
/// wire (probe, 2026-11-16): the commit lands ~0.9-1.5 s after
/// `audioStreamEnd`, then the session goes quiet — so the wait must cover
/// the commit plus [`QUIET_AFTER`], or a healthy capture misses the
/// deadline and falls back to the whole clip. Google never closes the
/// socket itself.
const FINALIZE_WAIT: Duration = Duration::from_millis(2_500);
/// Google finalizes the last turn right after the stream ends, so quiet this
/// long after the last event means nothing more is coming.
const QUIET_AFTER: Duration = Duration::from_millis(500);
/// A live transcription session caps at 10 minutes at Google. Voinc stops a
/// little early so the last turn still finalizes; a longer capture's stream
/// fails, and the whole clip decides.
const SESSION_CAP: Duration = Duration::from_secs(9 * 60 + 30);
/// The loop's read timeout: how often it notices an ended capture or a
/// cancelled stream between events.
const READ_PACING: Duration = Duration::from_millis(50);
/// How long a write to Google may take before the session fails; the whole
/// clip decides.
const WRITE_BUDGET: Duration = Duration::from_secs(10);

/// The stream under the TLS session to Google: direct TCP, or a tunnel
/// through the computer's proxy (possibly itself TLS). `ReadWrite` names the
/// two shapes; the standard `Box` impls carry `Read` and `Write` through.
type Underlying = Box<dyn ReadWrite>;

trait ReadWrite: Read + Write + Send {
    /// The TCP socket underneath, when there is one, so a tunnel's timeouts
    /// reach the socket itself.
    fn socket(&self) -> Option<&TcpStream> {
        None
    }
}

impl ReadWrite for TcpStream {
    fn socket(&self) -> Option<&TcpStream> {
        Some(self)
    }
}

impl ReadWrite for native_tls::TlsStream<TcpStream> {
    fn socket(&self) -> Option<&TcpStream> {
        self.get_ref().socket()
    }
}

/// Sets the socket's timeouts, reached through the tunnel's layers.
fn set_timeouts(underlying: &Underlying, read: Duration, write: Duration) {
    let Some(socket) = underlying.socket() else {
        return;
    };
    let _ = socket.set_read_timeout(Some(read));
    let _ = socket.set_write_timeout(Some(write));
}

/// One session's WebSocket: TLS over the stream to Google.
type SessionSocket = tungstenite::WebSocket<native_tls::TlsStream<Underlying>>;

/// One live stream as the engine drives it.
pub(crate) struct GeminiLiveStream {
    /// Audio chunks the session thread writes; disconnected once it ended.
    audio: mpsc::SyncSender<Vec<u8>>,
    /// What the session thread has heard Google say.
    shared: Arc<Shared>,
    /// Set at finalize, so the thread asks Google to finalize the last turn.
    ended: Arc<AtomicBool>,
    /// Set at drop, so a cancelled capture's session closes at once.
    closed: Arc<AtomicBool>,
    /// The engine's cancellation for this stream: the session stops with it.
    cancel: transcribe_cpp::CancelToken,
    /// 16-bit samples not yet a whole 100 ms chunk.
    pending: Vec<u8>,
    /// Chunks dropped to a stalled connection, logged once.
    lost: usize,
}

impl GeminiLiveStream {
    /// Opens a session: connects through the computer's proxy, authenticates
    /// with the key, sends the setup, and waits for Google's `setupComplete`.
    /// Every refusal names why, so the stream is refused cleanly and the
    /// whole clip decides.
    pub(crate) fn open(
        config: &Config,
        cancel: &transcribe_cpp::CancelToken,
    ) -> Result<Self, GeminiError> {
        let shared = Arc::new(Shared::default());
        let websocket = connect(config)?;
        let (audio, queued) = mpsc::sync_channel(AUDIO_QUEUE);
        let ended = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let setup = setup_message(config);
        let worker = {
            let audio = queued;
            let shared = shared.clone();
            let ended = ended.clone();
            let closed = closed.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("gemini-live".into())
                .spawn(move || {
                    // The setup is this thread's first write; the owner wrote
                    // nothing after the handshake, so there is no race.
                    session(websocket, setup, audio, shared, ended, closed, cancel)
                })
                .map_err(|error| GeminiError::Unreachable(error.to_string()))?
        };
        // A session Google won't set up is refused now, so a first dictation
        // learns of a bad key or model from its own capture.
        if !shared.wait_for_setup(CONNECT_BUDGET) {
            // The thread exits within its pacing once `closed` shows.
            closed.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(shared.failure().map_or_else(
                || {
                    GeminiError::Unreachable(
                        "Google's live session did not complete its setup".into(),
                    )
                },
                GeminiError::Unreachable,
            ));
        }
        Ok(Self {
            audio,
            shared,
            ended,
            closed,
            cancel: cancel.clone(),
            pending: Vec::new(),
            lost: 0,
        })
    }
}

impl Drop for GeminiLiveStream {
    fn drop(&mut self) {
        // A cancelled or finished-with capture closes its session at once;
        // the thread never waits on a finalize nobody reads.
        self.closed.store(true, Ordering::Release);
    }
}

impl StreamFeed for GeminiLiveStream {
    fn feed(&mut self, samples: &[f32]) -> Result<(), String> {
        if let Some(failure) = self.shared.failure() {
            return Err(failure);
        }
        self.pending.reserve(samples.len() * 2);
        for &sample in samples {
            self.pending
                .extend_from_slice(&pcm_i16(sample).to_le_bytes());
        }
        while self.pending.len() >= CHUNK_SAMPLES * 2 {
            let chunk: Vec<u8> = self.pending.drain(..CHUNK_SAMPLES * 2).collect();
            if self.audio.try_send(chunk).is_err() {
                self.lost += 1;
                if self.lost == 1 {
                    tracing::warn!(
                        "Google's live session fell behind; audio it missed is covered by the whole clip"
                    );
                }
            }
        }
        Ok(())
    }

    fn finalize(&mut self) -> Result<String, String> {
        // The last sub-chunk is audio too: a capture's end is rarely a whole
        // 100 ms boundary.
        if !self.pending.is_empty() {
            let chunk = std::mem::take(&mut self.pending);
            let _ = self.audio.try_send(chunk);
        }
        self.ended.store(true, Ordering::Release);
        self.shared.wait_for_final(FINALIZE_WAIT, &self.cancel)
    }

    fn partial(&self) -> Partial {
        self.shared.partial()
    }
}

fn pcm_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// The setup one session asks Google for: the transcription model, text
/// answers, and the person's language, vocabulary, and smart formatting.
fn setup_message(config: &Config) -> String {
    let mut transcription = serde_json::Map::new();
    transcription.insert(
        "languageCodes".into(),
        serde_json::json!(config.language.iter().collect::<Vec<_>>()),
    );
    if !config.vocabulary.is_empty() {
        transcription.insert(
            "customVocabulary".into(),
            serde_json::json!(config.vocabulary),
        );
    }
    transcription.insert("mode".into(), serde_json::json!("SMART"));
    serde_json::json!({
        "setup": {
            "model": LIVE_MODEL,
            "generationConfig": {"responseModalities": ["TEXT"]},
            "inputAudioTranscription": transcription,
        }
    })
    .to_string()
}

/// Connects and completes the WebSocket handshake, inside [`CONNECT_BUDGET`].
fn connect(config: &Config) -> Result<SessionSocket, GeminiError> {
    let deadline = Instant::now() + CONNECT_BUDGET;
    let tcp = tcp_stream(&deadline)?;
    let left = || {
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1))
    };
    let connector = native_tls::TlsConnector::builder()
        .build()
        .map_err(|error| GeminiError::Unreachable(format!("TLS failed ({error})")))?;
    // A handshake interrupted (the timeouts make WouldBlock likely) is a
    // failure here: the budget is the session's one chance.
    let tls = match connector.connect(API_HOST, tcp) {
        Ok(tls) => tls,
        Err(native_tls::HandshakeError::Failure(error)) => {
            return Err(GeminiError::Unreachable(format!("TLS failed ({error})")));
        }
        Err(_) => {
            return Err(GeminiError::Unreachable("TLS failed".into()));
        }
    };
    set_timeouts(tls.get_ref(), left(), left());
    let request = ureq::http::Request::builder()
        .method("GET")
        .uri(WS_URL)
        .header("Host", API_HOST)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .header("x-goog-api-key", &config.key)
        .body(())
        .map_err(|error| GeminiError::Invalid(error.to_string()))?;
    match tungstenite::client(request, tls) {
        Ok((websocket, _)) => Ok(websocket),
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            let status = response.status().as_u16();
            let body = response
                .body()
                .as_deref()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            Err(super::error_for_status(status, &body))
        }
        Err(_) => Err(GeminiError::Unreachable(
            "Google's live session could not be established".into(),
        )),
    }
}

/// The socket to Google: through the computer's proxy when it is a CONNECT
/// proxy, direct otherwise. A SOCKS proxy cannot carry the handshake here;
/// its batch path still can, so the stream is refused and the clip decides.
fn tcp_stream(deadline: &Instant) -> Result<Underlying, GeminiError> {
    let left = || deadline.saturating_duration_since(Instant::now());
    let Some(proxy) = net::proxy_for(WS_URL) else {
        let address = first_address(API_HOST, 443)?;
        let socket = TcpStream::connect_timeout(&address, left())
            .map_err(|error| GeminiError::Unreachable(unreachable(&error)))?;
        return Ok(Box::new(socket));
    };
    match proxy.protocol() {
        ureq::ProxyProtocol::Http | ureq::ProxyProtocol::Https => {
            let address = first_address(proxy.host(), proxy.port())?;
            let socket = TcpStream::connect_timeout(&address, left()).map_err(|error| {
                GeminiError::Unreachable(format!("the proxy refused ({error})"))
            })?;
            let _ = socket.set_read_timeout(Some(left()));
            let _ = socket.set_write_timeout(Some(left()));
            // A CONNECT proxy over HTTPS wants its own TLS before the tunnel.
            let socket: Underlying = if proxy.protocol() == ureq::ProxyProtocol::Https {
                let connector = native_tls::TlsConnector::builder()
                    .build()
                    .map_err(|error| GeminiError::Unreachable(format!("TLS failed ({error})")))?;
                let tls = match connector.connect(proxy.host(), socket) {
                    Ok(tls) => tls,
                    Err(native_tls::HandshakeError::Failure(error)) => {
                        return Err(GeminiError::Unreachable(format!(
                            "the proxy refused ({error})"
                        )));
                    }
                    Err(_) => {
                        return Err(GeminiError::Unreachable("the proxy refused".into()));
                    }
                };
                Box::new(tls)
            } else {
                Box::new(socket)
            };
            let socket = tunnel(socket, &proxy, deadline)?;
            Ok(socket)
        }
        _ => Err(GeminiError::Unreachable(
            "the computer's proxy is of a kind the live stream can't use; its batch \
             transcription still works"
                .into(),
        )),
    }
}

/// Why a connection to Google failed, named for a person.
fn unreachable(error: &std::io::Error) -> String {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        "the connection timed out".into()
    } else {
        format!("the connection failed ({error})")
    }
}

fn first_address(host: &str, port: u16) -> Result<std::net::SocketAddr, GeminiError> {
    use std::net::ToSocketAddrs as _;
    (host, port)
        .to_socket_addrs()
        .map_err(|error| GeminiError::Unreachable(unreachable(&error)))?
        .next()
        .ok_or_else(|| GeminiError::Unreachable("the name couldn't be looked up".into()))
}

/// Asks a CONNECT proxy to carry the stream to Google, and waits for its
/// assent.
fn tunnel(
    mut socket: Underlying,
    proxy: &ureq::Proxy,
    deadline: &Instant,
) -> Result<Underlying, GeminiError> {
    let mut request = format!("CONNECT {API_HOST}:443 HTTP/1.1\r\nHost: {API_HOST}:443\r\n");
    if let (Some(user), Some(password)) = (proxy.username(), proxy.password()) {
        let joined = format!("{user}:{password}");
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(joined)
        ));
    }
    request.push_str("\r\n");
    socket
        .write_all(request.as_bytes())
        .map_err(|error| GeminiError::Unreachable(format!("the proxy refused ({error})")))?;
    let mut response = vec![0_u8; 4096];
    let mut filled = 0;
    let answer = loop {
        let read = socket
            .read(&mut response[filled..])
            .map_err(|error| GeminiError::Unreachable(format!("the proxy refused ({error})")))?;
        if read == 0 {
            return Err(GeminiError::Unreachable("the proxy refused".into()));
        }
        filled += read;
        if let Some(position) = response[..filled]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        {
            break String::from_utf8_lossy(&response[..position]).into_owned();
        }
        if filled == response.len() || Instant::now() >= *deadline {
            return Err(GeminiError::Unreachable("the proxy refused".into()));
        }
    };
    if !answer.contains(" 2") {
        return Err(GeminiError::Unreachable(format!(
            "the proxy refused ({})",
            answer.lines().next().unwrap_or_default()
        )));
    }
    Ok(socket)
}

/// What the session thread has heard Google say, shared with the stream.
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Default)]
struct State {
    /// Google accepted the setup.
    setup: bool,
    /// Every finalized turn, in order, joined.
    turns: String,
    /// The current utterance's speculative text.
    interim: String,
    /// When Google last spoke: the quiet rule ends a finalize without a
    /// turn end.
    last_event: Option<Instant>,
    /// The session's answer once it has one: its text, or why it failed.
    result: Option<Result<String, String>>,
}

impl Shared {
    /// Waits until Google accepts the setup, the session fails, or `budget`
    /// passes.
    fn wait_for_setup(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if state.setup || state.result.is_some() {
                return state.setup;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (guard, _timeout) = self
                .ready
                .wait_timeout(state, left.min(READ_PACING))
                .unwrap_or_else(|error| error.into_inner());
            state = guard;
        }
    }

    /// Waits for the session's answer: its text, or why it failed. The
    /// session thread settles on its own [`FINALIZE_WAIT`]; this is the
    /// owner's safety net.
    fn wait_for_final(
        &self,
        budget: Duration,
        cancel: &transcribe_cpp::CancelToken,
    ) -> Result<String, String> {
        let deadline = Instant::now() + budget;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(result) = &state.result {
                return result.clone();
            }
            if cancel.is_cancelled() {
                return Err("the live stream was cancelled".into());
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                // Google's own turn end already settled the last utterance:
                // its text is the answer, and nothing is coming for the rest.
                if state
                    .last_event
                    .is_none_or(|at| at.elapsed() >= QUIET_AFTER)
                {
                    return Ok(state.answer());
                }
                return Err("Google's live session missed its finish deadline".into());
            };
            let (guard, _timeout) = self
                .ready
                .wait_timeout(state, left.min(READ_PACING))
                .unwrap_or_else(|error| error.into_inner());
            state = guard;
        }
    }

    /// The stream's text so far: finalized turns, then the current
    /// utterance's speculative text.
    fn partial(&self) -> Partial {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Partial {
            stable: state.turns.clone(),
            tentative: state.interim.clone(),
        }
    }

    /// Why the session failed, once it has.
    fn failure(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .result
            .as_ref()
            .and_then(|result| result.as_ref().err().cloned())
    }
}

impl State {
    /// The session's whole text: committed turns, then the utterance still
    /// speculative. After the stream end that speculation is the only text
    /// for an utterance Google never committed, and dropping it would lose
    /// what was said.
    fn answer(&self) -> String {
        let mut text = self.turns.clone();
        let piece = self.interim.trim();
        if !piece.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
        text
    }

    /// Google finalized one utterance: its text joins the turns.
    fn hear(&mut self, text: &str, shared: &Shared) {
        let piece = text.trim();
        if piece.is_empty() {
            return;
        }
        if !self.turns.is_empty() {
            self.turns.push(' ');
        }
        self.turns.push_str(piece);
        self.interim.clear();
        self.spoke(shared);
    }

    /// Google's speculative text for the utterance in progress, replacing
    /// whatever it speculated before.
    fn speculate(&mut self, text: &str, shared: &Shared) {
        self.interim = text.to_owned();
        self.spoke(shared);
    }

    /// A turn completed. The utterance's text normally arrived finalized
    /// (`inputTranscription`) first, but a turn whose text only ever came as
    /// speculation still happened: commit it before the next turn's
    /// speculation replaces it.
    fn turn_complete(&mut self, shared: &Shared) {
        let pending = std::mem::take(&mut self.interim);
        let piece = pending.trim();
        if !piece.is_empty() {
            if !self.turns.is_empty() {
                self.turns.push(' ');
            }
            self.turns.push_str(piece);
        }
        self.spoke(shared);
    }

    fn spoke(&mut self, shared: &Shared) {
        self.last_event = Some(Instant::now());
        shared.ready.notify_all();
    }

    /// The session answers: its text, or why it failed.
    fn complete(&mut self, result: Result<String, String>, shared: &Shared) {
        if self.result.is_none() {
            self.result = Some(result);
        }
        shared.ready.notify_all();
    }
}

/// Answers the session's failure once and for all.
fn fail(shared: &Shared, reason: String) {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .complete(Err(reason), shared);
}

/// Answers the session with its text so far.
fn settle(shared: &Shared) {
    let text = {
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.answer()
    };
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .complete(Ok(text), shared);
}

/// Writes one audio chunk: 16-bit PCM at 16 kHz, base64 in a `realtimeInput`.
fn write_audio(websocket: &mut SessionSocket, pcm: &[u8]) -> tungstenite::Result<()> {
    let data = base64::engine::general_purpose::STANDARD.encode(pcm);
    let message = serde_json::json!({
        "realtimeInput": {
            "audio": {"data": data, "mimeType": "audio/pcm;rate=16000"},
        }
    })
    .to_string();
    websocket.write(Message::text(message))?;
    websocket.flush()
}

/// Handles one event from Google, answering whether the session is over.
fn handle(
    websocket: &mut SessionSocket,
    message: Message,
    shared: &Shared,
    sent_end: bool,
) -> bool {
    match message {
        Message::Text(text) => handle_event(text.as_str(), shared, sent_end),
        // Google's live events ride binary frames whose payload is the same
        // JSON a text frame would carry; both parse. The first one,
        // `setupComplete`, is what arms the session at all.
        Message::Binary(bytes) => {
            handle_event(String::from_utf8_lossy(&bytes).as_ref(), shared, sent_end)
        }
        // The server closed: whatever turns it finalized are the answer.
        Message::Close(_) => {
            if sent_end {
                settle(shared);
            } else {
                fail(shared, "Google closed the live session".into());
            }
            true
        }
        // A ping's queued pong goes out with the next flush.
        Message::Ping(_) => {
            let _ = websocket.flush();
            false
        }
        _ => false,
    }
}

/// Applies one event's JSON to the session, answering whether it is over.
/// The socket is untouched: events only read and set state.
fn handle_event(text: &str, shared: &Shared, sent_end: bool) -> bool {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    if event.get("setupComplete").is_some() {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.setup = true;
        shared.ready.notify_all();
        return false;
    }
    if let Some(content) = event.get("serverContent") {
        if let Some(text) = content
            .get("interimInputTranscription")
            .and_then(|interim| interim.get("text"))
            .and_then(|text| text.as_str())
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.speculate(text, shared);
        }
        if let Some(text) = content
            .get("inputTranscription")
            .and_then(|finalized| finalized.get("text"))
            .and_then(|text| text.as_str())
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.hear(text, shared);
        }
        if content.get("turnComplete").is_some() || content.get("generationComplete").is_some() {
            // This live model ends its turns with `generationComplete` rather
            // than `turnComplete` (observed on the wire): the same turn end
            // under two names. Either way the turn is over.
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.turn_complete(shared);
            if sent_end {
                // The last turn is over: the whole capture's text is
                // the answer.
                drop(state);
                settle(shared);
                return true;
            }
        }
    }
    if let Some(error) = event
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
    {
        fail(
            shared,
            format!("Google's live session failed: {}", error.trim()),
        );
        return true;
    }
    false
}

/// The session thread: writes the audio queued to it, reads Google's events
/// between writes, and answers once its final turn settles.
fn session(
    mut websocket: SessionSocket,
    setup: String,
    audio: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Shared>,
    ended: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    cancel: transcribe_cpp::CancelToken,
) {
    // The connect budget's short timeouts are over: the loop's pacing reads,
    // and a stalled write fails the session within its budget.
    set_timeouts(websocket.get_ref().get_ref(), READ_PACING, WRITE_BUDGET);
    let begun = Instant::now();
    if let Err(error) = websocket.write(Message::text(setup)) {
        fail(&shared, format!("the live session failed: {error}"));
        return;
    }
    let _ = websocket.flush();
    let mut sent_end = false;
    let mut finalize_at: Option<Instant> = None;
    loop {
        if closed.load(Ordering::Acquire) || cancel.is_cancelled() {
            return;
        }
        // The audio queued so far, then whatever else arrives before the
        // next read.
        while let Ok(pcm) = audio.try_recv() {
            if let Err(error) = write_audio(&mut websocket, &pcm) {
                fail(&shared, format!("the live session failed: {error}"));
                return;
            }
        }
        if ended.load(Ordering::Acquire) && !sent_end {
            let end = serde_json::json!({"realtimeInput": {"audioStreamEnd": true}}).to_string();
            if let Err(error) = websocket.write(Message::text(end)) {
                fail(&shared, format!("the live session failed: {error}"));
                return;
            }
            let _ = websocket.flush();
            sent_end = true;
            finalize_at = Some(Instant::now());
        }
        // Google's events, with the loop's pacing when nothing comes.
        match websocket.read() {
            Ok(message) => {
                if handle(&mut websocket, message, &shared, sent_end) {
                    return;
                }
            }
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // The pacing passed; the loop notices ends and cancels.
            }
            Err(tungstenite::Error::ConnectionClosed) => {
                if sent_end {
                    // The server closed after the stream end; whatever turns
                    // it finalized are the answer.
                    settle(&shared);
                } else {
                    fail(&shared, "Google closed the live session".into());
                }
                return;
            }
            Err(error) => {
                fail(&shared, format!("the live session failed: {error}"));
                return;
            }
        }
        if sent_end {
            let elapsed = finalize_at.is_some_and(|at| at.elapsed() >= FINALIZE_WAIT);
            if elapsed {
                fail(
                    &shared,
                    "Google's live session missed its finish deadline".into(),
                );
                return;
            }
            let quiet = {
                let state = shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                state.result.is_some()
                    || state
                        .last_event
                        .is_some_and(|at| at.elapsed() >= QUIET_AFTER)
            };
            if quiet {
                settle(&shared);
                return;
            }
        } else if begun.elapsed() >= SESSION_CAP {
            fail(
                &shared,
                "the live session reached Google's limit for one capture".into(),
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Google's live session arms and streams over binary WebSocket frames
    /// whose payloads are the events' JSON (observed on the wire). The first
    /// binary frame, `setupComplete`, is what arms the session at all; a
    /// text frame parses just the same.
    #[test]
    fn the_live_events_parse_from_binary_frames() {
        let shared = Shared::default();
        let frame = |json: &str, shared: &Shared| {
            let bytes = tungstenite::Bytes::from(json.to_string());
            handle_event(String::from_utf8_lossy(&bytes).as_ref(), shared, false)
        };
        frame("{\"setupComplete\":{}}", &shared);
        assert!(
            shared.wait_for_setup(Duration::ZERO),
            "the binary setupComplete arms the session"
        );
        frame(
            "{\"serverContent\":{\"interimInputTranscription\":{\"text\":\"The latest\"}}}",
            &shared,
        );
        frame(
            "{\"serverContent\":{\"inputTranscription\":{\"text\":\"The latest tech build is ready to test.\"}}}",
            &shared,
        );
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(state.turns, "The latest tech build is ready to test.");
        assert_eq!(state.interim, "", "a finalized turn clears the interim");
        assert!(state.setup);
    }

    #[test]
    fn the_same_events_parse_from_text_frames() {
        let shared = Shared::default();
        assert!(!handle_event("{\"setupComplete\":{}}", &shared, false));
        assert!(
            shared.wait_for_setup(Duration::ZERO),
            "the text setupComplete arms the session too"
        );
    }

    #[test]
    fn an_error_event_ends_the_session_with_its_reason() {
        let shared = Shared::default();
        let json = r#"{"error":{"code":400,"message":"unexpected model name format"}}"#;
        assert!(
            handle_event(json, &shared, false),
            "an error ends the session"
        );
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let failure = state
            .result
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .expect("the session failed with the reason");
        assert!(
            failure.contains("unexpected model name format"),
            "{failure}"
        );
    }

    /// Google's live model ends its turns with `generationComplete` rather
    /// than `turnComplete` (observed on the wire, right after the finalized
    /// `inputTranscription`): after the stream end it settles the session
    /// with the whole capture's text.
    #[test]
    fn a_generation_complete_settles_the_stream_after_the_end() {
        let shared = Shared::default();
        let frame = |json: &str, shared: &Shared, sent_end: bool| {
            let bytes = tungstenite::Bytes::from(json.to_string());
            handle_event(String::from_utf8_lossy(&bytes).as_ref(), shared, sent_end)
        };
        frame(
            "{\"serverContent\":{\"inputTranscription\":{\"text\":\"The latest tech build is ready to test.\"}}}",
            &shared,
            true,
        );
        assert!(frame(
            "{\"serverContent\":{\"generationComplete\":true}}",
            &shared,
            true,
        ));
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let answer = state
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .expect("the turn end settled the session");
        assert_eq!(answer, "The latest tech build is ready to test.");
    }

    /// The settled answer keeps an utterance Google speculated but never
    /// committed: after the stream end that speculation is the only text for
    /// it, and dropping it would lose what was said.
    #[test]
    fn the_settled_answer_keeps_a_speculated_utterance() {
        let shared = Shared::default();
        handle_event(
            "{\"serverContent\":{\"interimInputTranscription\":{\"text\":\"The latest tech build\"}}}",
            &shared,
            false,
        );
        settle(&shared);
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let answer = state
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .expect("the settle answered");
        assert_eq!(answer, "The latest tech build");
    }

    /// A turn end commits an utterance whose text only ever arrived as
    /// speculation, so the next turn's speculation cannot replace it.
    #[test]
    fn a_turn_end_commits_a_speculated_utterance() {
        let shared = Shared::default();
        handle_event(
            "{\"serverContent\":{\"interimInputTranscription\":{\"text\":\"The latest\"}}}",
            &shared,
            false,
        );
        handle_event("{\"serverContent\":{\"turnComplete\":{}}}", &shared, false);
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(state.turns, "The latest");
        assert_eq!(state.interim, "");
    }

    /// The owner's finalize wait: once Google has gone quiet its whole text
    /// — committed turns and the speculative utterance — is the answer;
    /// while its last event is still recent the deadline is a failure, the
    /// whole clip's batch transcription covers it.
    #[test]
    fn the_finalize_wait_answers_quiet_text_and_fails_recent_activity() {
        let shared = Shared::default();
        let cancel = transcribe_cpp::CancelToken::new();
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.turns = "Earlier words".into();
            state.interim = "the latest".into();
        }
        // No event yet: Google never spoke, so the quiet rule reads.
        assert_eq!(
            shared.wait_for_final(Duration::ZERO, &cancel),
            Ok("Earlier words the latest".into())
        );
        // A speculation right now keeps the wait going, to its deadline.
        handle_event(
            "{\"serverContent\":{\"interimInputTranscription\":{\"text\":\"the newest\"}}}",
            &shared,
            false,
        );
        assert_eq!(
            shared.wait_for_final(Duration::ZERO, &cancel),
            Err("Google's live session missed its finish deadline".into())
        );
    }
}
