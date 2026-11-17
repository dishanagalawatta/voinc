//! The live text plate (Labs): a click-through pill beside the recording
//! HUD with what the dictation has said so far, on every OS. The words
//! wrap over up to four lines, and past that the oldest words give way
//! with "…", so the newest ones stay in view. It shows while a recording
//! has text and hides when the recording ends
//! ([`crate::dictation::live_text`]); `rune_hud` says where the HUD is,
//! so the plate sits just past it, on the HUD's screen. Labs › Failure
//! notices reuses the same plate for one line saying why a dictation
//! failed, for a few seconds or until the next capture.
//!
//! The overlay is prewarmed: macOS keeps it drawn while hidden and refreshes
//! it before a reveal, so it never reappears with the last dictation's
//! words for a frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rune::text::Span;
use rune::{
    Anim, Cx, El, OverlayHandle, OverlayOptions, OverlayPlacement, OverlaySpec, ScreenEdge, row,
    text,
};

use super::controls::Paint;
use crate::transcription::engine::Partial;

/// The overlay's size: room for about a dozen words a line and four
/// wrapped lines of them (four 20-point lines of the UI face, the pill's
/// padding and the root's, and slack).
pub(crate) const SIZE: (f32, f32) = (560.0, 108.0);
/// The gap between the plate's start and the HUD's own margin.
const PAD: f32 = 6.0;
/// The most lines the plate shows.
const MAX_LINES: usize = 4;
/// The most characters the plate keeps (about four lines at the width the
/// overlay leaves them): past it the oldest words give way, cut between
/// words, with "…" standing for them, so the newest words stay in view.
const WORD_BUDGET: usize = 240;
/// The least characters a trimmed part keeps: fewer cannot show a whole
/// word beside its "…", so the part gives way entirely.
const MIN_TAIL: usize = 8;
/// How long a failure notice stays beside the HUD.
const FAILURE_VISIBLE: Duration = Duration::from_secs(6);

#[derive(Default)]
pub(crate) struct Plate {
    /// The words shown, one string: the committed text, then a space, then
    /// the speculative one. [`Plate::tail`] is the byte where the
    /// speculative words start — 0 when there are none, a head "…" from
    /// the trim included then.
    words: String,
    tail: usize,
    /// Whether the pill hugs the overlay's bottom — the side the HUD is on
    /// when the HUD sits below the plate; otherwise it hugs the top.
    hug_bottom: bool,
    /// Labs › Failure notices: why the last dictation failed, shown instead
    /// of the live words while it lasts.
    failure: Option<String>,
    /// Whether live words are being published (a recording is showing text).
    live: bool,
    /// Whether the overlay is on screen. Every change happens in an overlay
    /// update, in queue order, so a queued hide cannot race a newer notice.
    shown: bool,
}

impl Plate {
    /// Keeps the newest words: the speculative text takes the room it
    /// needs, the committed text gets the rest, and past [`WORD_BUDGET`]
    /// the oldest words give way, cut between words, with "…" standing
    /// for them.
    fn set_words(&mut self, stable: String, tentative: String) {
        let stable = stable.trim();
        let tentative = tentative.trim();
        let join = usize::from(!stable.is_empty() && !tentative.is_empty());
        // A separating space counts toward the room.
        let room = WORD_BUDGET.saturating_sub(join);
        let kept_tentative = keep_tail(tentative, room);
        let stable_room = room.saturating_sub(kept_tentative.chars().count());
        // A remnant too small to show a whole word gives way with
        // everything before it.
        let kept_stable = if stable.chars().count() > stable_room && stable_room < MIN_TAIL {
            String::new()
        } else {
            keep_tail(stable, stable_room)
        };
        if kept_stable.is_empty() {
            self.tail = 0;
            self.words = kept_tentative;
        } else if kept_tentative.is_empty() {
            self.tail = kept_stable.len();
            self.words = kept_stable;
        } else {
            self.tail = kept_stable.len() + 1;
            self.words = format!("{kept_stable} {kept_tentative}");
        }
        self.live = true;
    }

    /// The recording stopped publishing words. Its last words stay drawn
    /// while the plate fades.
    fn words_ended(&mut self) {
        self.live = false;
    }

    fn show_failure(&mut self, message: String) {
        self.failure = Some(message);
    }

    fn take_failure(&mut self) -> bool {
        self.failure.take().is_some()
    }

    fn mark_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// Nothing live or failed remains to justify the plate.
    fn should_hide(&self) -> bool {
        !self.live && self.failure.is_none()
    }
}

/// A failure notice's message and deadline, kept apart from the drawn plate
/// so a timer thread can dismiss it without the live text's state.
#[derive(Default)]
struct FailureNotice {
    message: Option<String>,
    expires_at: Option<Instant>,
}

impl FailureNotice {
    /// Replaces any notice with `message` until [`FAILURE_VISIBLE`] from
    /// `now`; returns the new deadline.
    fn show(&mut self, message: String, now: Instant) -> Instant {
        self.message = Some(message);
        let deadline = now + FAILURE_VISIBLE;
        self.expires_at = Some(deadline);
        deadline
    }

    /// A new capture dismisses the notice at once; whether one was showing.
    fn capture_started(&mut self) -> bool {
        self.expires_at = None;
        self.message.take().is_some()
    }

    /// Dismisses a notice whose own deadline is due; whether it was showing.
    fn expire(&mut self, deadline: Instant, now: Instant) -> bool {
        if self.expires_at != Some(deadline) || now < deadline {
            return false;
        }
        self.expires_at = None;
        self.message.take().is_some()
    }
}

/// Where the plate starts, just past the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Spot {
    /// This far from the edge that the HUD is on, on the HUD's display
    /// (`rune_hud::display`).
    Edge(ScreenEdge, f32),
    /// At `y` in Rune's global space (points from the primary display's
    /// top-left, y down), centred on `x`; the plate hangs down from `y`
    /// when `below`, else rises from it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Global { x: f32, y: f32, below: bool },
}

impl Spot {
    /// The overlay's placement for the plate to start at this spot.
    fn placement(self) -> OverlayPlacement {
        match self {
            Self::Edge(edge, margin) => OverlayPlacement::Edge {
                edge,
                margin: margin - PAD,
                align: rune::Align::Center,
                inset: 0.0,
            },
            Self::Global { x, y, below } => OverlayPlacement::Rect {
                x: x - SIZE.0 / 2.0,
                y: if below { y - PAD } else { y - SIZE.1 + PAD },
                width: SIZE.0,
                height: SIZE.1,
            },
        }
    }

    /// Whether the pill hugs the overlay's bottom: the side the HUD is on
    /// when the HUD sits below the plate. Otherwise it hugs the top, so
    /// the pill grows down from the HUD instead of up into it.
    fn hugs_bottom(self) -> bool {
        match self {
            Self::Edge(edge, _) => matches!(edge, ScreenEdge::Bottom),
            Self::Global { below, .. } => !below,
        }
    }

    /// Places `overlay` for the plate to start at this spot.
    fn apply(self, overlay: &OverlayHandle<Plate>) {
        // Off macOS the HUD is placed with `place`, which picks the display
        // again and measures from the same edge of its work area (above the
        // Windows taskbar), so the plate is placed the same way and never
        // lands under the HUD (#797).
        #[cfg(not(target_os = "macos"))]
        if let Self::Edge(edge, margin) = self {
            overlay.place(edge, margin - PAD);
            return;
        }
        overlay.set_placement(Some(self.placement()));
    }
}

/// The plate's overlay, once installed.
static LIVE: std::sync::OnceLock<rune::OverlayHandle<Plate>> = std::sync::OnceLock::new();

/// Whether a non-blank partial has been queued since the last words-ended
/// update. It lets that update be skipped while nothing was ever shown,
/// without deciding visibility outside the overlay's queue.
static WORDS: AtomicBool = AtomicBool::new(false);

/// Where the plate goes, kept for a failure notice that appears after
/// installation.
static PLACE: OnceLock<Place> = OnceLock::new();

/// The current failure notice, which a timer thread dismisses.
static NOTICE: Mutex<FailureNotice> = Mutex::new(FailureNotice {
    message: None,
    expires_at: None,
});

/// The timer thread's queue: each newer notice's deadline supersedes the
/// one before it.
static TIMER: OnceLock<Sender<Instant>> = OnceLock::new();

fn notice() -> std::sync::MutexGuard<'static, FailureNotice> {
    NOTICE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Moves a shown plate to the display under the pointer when it changed
/// (`rune_hud`'s follow, #807).
#[cfg(not(target_os = "macos"))]
pub(crate) fn refollow_pointer() {
    if let Some(overlay) = LIVE.get() {
        overlay.refollow_pointer();
    }
}

/// Where the plate goes. Called on the app's thread each time the plate
/// appears, so it may read the screen.
pub(crate) type Place = fn() -> Spot;

/// Adds the plate to the app. It shows while a dictation publishes its text
/// (Labs › Live text) or while a failure notice explains a failed dictation
/// (Labs › Failure notices), applied at once. On macOS it's always prewarmed,
/// since either lab may be turned on after launch and an unprewarmed plate
/// reappears for a frame with the last dictation's words (#1075); elsewhere
/// when either is on at launch.
pub(crate) fn install<S: 'static>(app: &mut rune::App<S>, place: Place) {
    let spec = OverlaySpec {
        name: "Voinc live text",
        size: SIZE,
        // Placed for real each time it appears.
        edge: ScreenEdge::Top,
        margin: 0.0,
        align: rune::Align::Center,
        inset: 0.0,
        interactive: false,
        view,
    }
    .options(OverlayOptions {
        prewarm: cfg!(target_os = "macos")
            || crate::labs::live_text()
            || crate::labs::failure_notices(),
        #[cfg(not(target_os = "macos"))]
        display: crate::rune_hud::display(),
        ..Default::default()
    })
    // Hidden, it holds no GPU surface (nor, on Windows, a window), so it
    // never passes for a game to capture tools (#1308).
    .release_gpu_when_hidden();
    let overlay = app.overlay(spec, Plate::default());
    let _ = LIVE.set(overlay.clone());
    let _ = PLACE.set(place);
    crate::dictation::live_text::watch(move |partial| {
        let partial = partial.filter(|partial| !is_blank(partial));
        let Some(Partial { stable, tentative }) = partial.cloned() else {
            // Nothing was published since the last end and no notice is up:
            // this is a no-op, and skipping it keeps a dictation that never
            // showed text off the update queue.
            let words = WORDS.swap(false, Ordering::AcqRel);
            let notice_up = crate::labs::failure_notices() && notice().message.is_some();
            if !words && !notice_up {
                return;
            }
            let handle = overlay.clone();
            overlay.update(move |plate, _| {
                plate.words_ended();
                if plate.should_hide() && plate.shown {
                    // Hidden from the same queue that shows it, so a recording
                    // that ends as its plate appears never leaves it showing.
                    // Its last words stay while it fades (macOS and Windows);
                    // the next appearance replaces them first.
                    handle.fade_out(FADE);
                    plate.mark_shown(false);
                }
            });
            return;
        };
        WORDS.store(true, Ordering::Release);
        let handle = overlay.clone();
        overlay.update(move |plate, tx| {
            plate.set_words(stable, tentative);
            tx.instant();
            if plate.failure.is_none() && !plate.shown {
                // Placed and shown after the new words, so it appears
                // where it belongs with them already drawn.
                let spot = place();
                plate.hug_bottom = spot.hugs_bottom();
                spot.apply(&handle);
                handle.show();
                plate.mark_shown(true);
            }
        });
    });
}

/// Shows `message` as a one-line notice beside the HUD for
/// [`FAILURE_VISIBLE`] (Labs › Failure notices), replacing any live words
/// until it leaves or a new capture starts.
pub(crate) fn show_failure(message: &str) {
    if !crate::labs::failure_notices() {
        return;
    }
    let (Some(overlay), Some(place)) = (LIVE.get(), PLACE.get().copied()) else {
        return;
    };
    let message = crate::desktop::activity::failure_explanation(message);
    let deadline = notice().show(message.clone(), Instant::now());
    schedule(deadline);
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        plate.show_failure(message);
        tx.instant();
        if !plate.shown {
            let spot = place();
            plate.hug_bottom = spot.hugs_bottom();
            spot.apply(&handle);
            handle.show();
            plate.mark_shown(true);
        }
    });
}

/// Hides a failure notice at once when a new capture starts; live words
/// already on the plate stay.
pub(crate) fn capture_started() {
    dismiss_notice();
}

/// Hides a failure notice when Labs › Failure notices is turned off.
pub(crate) fn dismiss_failure() {
    dismiss_notice();
}

fn dismiss_notice() {
    let cleared = { notice().capture_started() };
    if !cleared {
        return;
    }
    let Some(overlay) = LIVE.get() else {
        return;
    };
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        // A failure that arrived after this dismissal keeps its notice.
        if notice().message.is_some() {
            return;
        }
        if plate.take_failure() {
            tx.instant();
        }
        if plate.should_hide() && plate.shown {
            handle.fade_out(FADE);
            plate.mark_shown(false);
        }
    });
}

/// Queues `deadline` to the timer thread, starting it on the first notice so
/// a Voinc with the lab off runs no notice thread.
fn schedule(deadline: Instant) {
    let sender = TIMER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("voinc-failure-notice".into())
            .spawn(move || run_timer(receiver));
        sender
    });
    let _ = sender.send(deadline);
}

fn run_timer(receiver: Receiver<Instant>) {
    while let Ok(mut deadline) = receiver.recv() {
        loop {
            let now = Instant::now();
            if now >= deadline {
                hide_expired(deadline);
                break;
            }
            match receiver.recv_timeout(deadline - now) {
                Ok(newer) => deadline = newer,
                Err(RecvTimeoutError::Timeout) => {
                    hide_expired(deadline);
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn hide_expired(deadline: Instant) {
    let expired = { notice().expire(deadline, Instant::now()) };
    if !expired {
        return;
    }
    let Some(overlay) = LIVE.get() else {
        return;
    };
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        // A failure that arrived after this deadline keeps its notice.
        if notice().message.is_some() {
            return;
        }
        if plate.take_failure() {
            tx.instant();
        }
        if plate.should_hide() && plate.shown {
            handle.fade_out(FADE);
            plate.mark_shown(false);
        }
    });
}

/// How long the plate fades out once a dictation stops publishing.
const FADE: Duration = Duration::from_millis(150);

/// The model's first update often has no words yet.
fn is_blank(partial: &Partial) -> bool {
    partial.stable.trim().is_empty() && partial.tentative.trim().is_empty()
}

/// Keeps the last words of `text` within `budget` characters, cutting
/// between words, with "…" standing for what gave way; text within the
/// budget is unchanged. Prose with no separators to cut at (CJK) cuts
/// between characters instead, where there is no word boundary to keep.
fn keep_tail(text: &str, budget: usize) -> String {
    let count = text.chars().count();
    if count <= budget {
        return text.to_owned();
    }
    if budget == 0 {
        return String::new();
    }
    let cut = count - budget;
    let mut at = text
        .char_indices()
        .nth(cut)
        .map_or(text.len(), |(index, _)| index);
    // A cut inside a word gives way with the whole word, so no half word
    // is shown.
    if text[..at]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace())
        && text[at..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace())
        && let Some(space) = text[at..].find(char::is_whitespace)
    {
        at += space;
    }
    format!("… {}", text[at..].trim_start())
}

fn plate(p: &Paint, body: El) -> El {
    let c = &p.k.c;
    row()
        .key("plate")
        .max_w(SIZE.0 - 8.0)
        .px(14.0)
        .py(6.0)
        .radius(p.k.radius.md)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .child(body)
}

fn view(plate_state: &Plate, cx: &mut Cx<Plate>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let root = row()
        .key("live-text")
        .w_full()
        .h_full()
        .justify_center()
        .px(4.0)
        .py(PAD)
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT);
    // The pill hugs the side the HUD is on: a top HUD hangs it from the
    // overlay's top, a bottom one raises it from the bottom, and it grows
    // away from the HUD as the words wrap.
    let root = if plate_state.hug_bottom {
        root.items_end()
    } else {
        root.items_start()
    };
    if let Some(failure) = &plate_state.failure {
        let line = text(failure.clone())
            .key("failure")
            .truncate()
            .font(p.k.ty.ui)
            .color(p.danger());
        return root.child(plate(&p, line));
    }
    if plate_state.words.trim().is_empty() {
        return root;
    }
    // The committed words read first and the speculative ones follow, one
    // paragraph with spans: bidi lays right-to-left text out, so no row
    // flip is needed, and the speculative words keep their faint colour to
    // the reading end.
    let only_tentative = plate_state.tail == 0;
    let mut words = text(plate_state.words.clone())
        .key("words")
        .shrink()
        .min_w(0.0)
        .max_lines(MAX_LINES)
        .font(p.k.ty.ui)
        .color(if only_tentative { p.faint() } else { p.text() });
    if !only_tentative && plate_state.tail < plate_state.words.len() {
        words = words.spans(vec![
            Span::new(plate_state.tail, plate_state.words.len()).color(p.faint()),
        ]);
    }
    root.child(plate(&p, words))
}

#[cfg(test)]
mod tests {
    use rune::Runtime;
    use rune::text::TextSystem;

    use super::*;

    const SHORT: &str = "So the plan for today";
    const LONG: &str = "So the plan for today is to finish the release notes, then check the \
        streaming numbers on the M4, and after lunch we can review the onboarding flow together";
    const HEBREW: &str = "אז התוכנית להיום";
    /// Longer than the plate keeps, to see the head trim's "…" in the
    /// snapshot.
    const OVERLONG: &str = "so we walked through the whole plan once more, and then the \
        streaming numbers came in, and after that the onboarding flow was \
        reviewed together with the team, and finally the release notes were \
        finished, checked twice, and shipped with the build that afternoon";

    #[test]
    fn a_failure_notice_lasts_six_seconds() {
        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let deadline = notice.show(
            "No microphone is available. Connect a microphone.".into(),
            start,
        );
        assert_eq!(
            notice.message.as_deref(),
            Some("No microphone is available. Connect a microphone.")
        );
        assert!(!notice.expire(deadline, start + FAILURE_VISIBLE - Duration::from_millis(1)));
        assert!(notice.expire(deadline, start + FAILURE_VISIBLE));
        assert_eq!(notice.message, None);
    }

    #[test]
    fn a_new_capture_dismisses_a_failure_notice_at_once() {
        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let deadline = notice.show("Could not open microphone".into(), start);
        assert!(notice.capture_started());
        assert_eq!(notice.message, None);
        assert!(!notice.capture_started());
        assert!(
            !notice.expire(deadline, start + FAILURE_VISIBLE),
            "the old deadline cannot dismiss a later notice"
        );
    }

    #[test]
    fn only_a_notices_own_deadline_dismisses_it() {
        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let older = notice.show("first".into(), start);
        let newer = notice.show("second".into(), start + Duration::from_secs(1));
        assert!(!notice.expire(older, start + FAILURE_VISIBLE));
        assert_eq!(notice.message.as_deref(), Some("second"));
        assert!(notice.expire(newer, start + Duration::from_secs(1) + FAILURE_VISIBLE));
        assert_eq!(notice.message, None);
    }

    fn runtime(stable: &str, tentative: &str, dark: bool) -> Runtime<Plate> {
        let mut plate = Plate::default();
        plate.set_words(stable.into(), tentative.into());
        runtime_plate(plate, dark)
    }

    fn runtime_plate(plate: Plate, dark: bool) -> Runtime<Plate> {
        let mut rt = crate::rune_ui::offscreen::runtime(
            plate,
            view,
            SIZE,
            rune::Env {
                system_dark: dark,
                scale: 2.0,
            },
            TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn rect(rt: &Runtime<Plate>, key: &str) -> Option<[f32; 4]> {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
            .map(|e| e.rect)
    }

    #[test]
    fn a_short_line_hugs_its_words() {
        let rt = runtime(SHORT, "", false);
        let [x, y, w, h] = rect(&rt, "plate").expect("the plate draws");
        assert!(w < SIZE.0 / 2.0, "the plate hugs a short line: {w}");
        assert!((x + w / 2.0 - SIZE.0 / 2.0).abs() < 1.0, "centred: {x} {w}");
        // The HUD sits above: the pill hangs from the overlay's top, its
        // one line as tall as before.
        assert!((y - PAD).abs() < 1.0, "top: {y}");
        assert!((h - 32.0).abs() < 0.5, "one line: {h}");
        assert!(rect(&rt, "words").is_some());
    }

    #[test]
    fn the_pill_hugs_the_side_the_hud_is_on() {
        // A bottom-edge HUD: the pill rises from the overlay's bottom.
        let mut plate = Plate::default();
        plate.set_words(SHORT.into(), String::new());
        plate.hug_bottom = true;
        let rt = runtime_plate(plate, false);
        let [_, y, _, h] = rect(&rt, "plate").unwrap();
        assert!((y + h - (SIZE.1 - PAD)).abs() < 1.0, "bottom: {y} {h}");
    }

    #[test]
    fn a_long_line_wraps_over_four_lines_at_most() {
        for tentative in ["", " together with the team"] {
            let rt = runtime(LONG, tentative, true);
            let [x, _, w, _] = rect(&rt, "plate").unwrap();
            assert!(x >= 0.0 && x + w <= SIZE.0, "inside: {x} {w}");
            let [_, _, _, lines] = rect(&rt, "words").unwrap();
            assert!(lines > 20.0, "more than one line: {lines}");
            assert!(
                lines <= 4.0 * 20.0 + 0.5,
                "within the four lines the plate keeps: {lines}"
            );
            let [_, _, _, pill] = rect(&rt, "plate").unwrap();
            assert!(pill <= 4.0 * 20.0 + 12.0 + 0.5, "the pill fits: {pill}");
        }
    }

    #[test]
    fn the_newest_words_stay_and_the_oldest_give_way() {
        let said = format!(
            "{LONG} {}now",
            "and then we went over the streaming numbers again ".repeat(4)
        );
        let mut plate = Plate::default();
        plate.set_words(said.clone(), String::new());
        let words = plate.words.clone();
        assert!(words.starts_with("… "), "the head gives way: {words}");
        assert!(
            words.chars().count() <= WORD_BUDGET + 2,
            "within the budget: {}",
            words.chars().count()
        );
        // The kept words are the dictation's newest ones, whole: they end
        // where it did, and a separator stood before the first kept word.
        let kept = &words["… ".len()..];
        assert!(said.ends_with(kept), "the tail is kept: {kept}");
        assert!(kept.ends_with("now"), "the newest words stay: {kept}");
        let at = said.len() - kept.len();
        assert!(
            said[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace),
            "the cut fell between words"
        );
    }

    #[test]
    fn a_long_speculation_leaves_the_committed_words_out() {
        let mut plate = Plate::default();
        plate.set_words(
            "earlier words that already landed".into(),
            format!(
                "{LONG} {}",
                "and the streaming numbers keep coming in ".repeat(3) + "now"
            ),
        );
        // The speculative text takes the room it needs; the committed words
        // that would not fit give way whole.
        assert_eq!(plate.tail, 0, "no committed words fit");
        assert!(!plate.words.contains("earlier"));
        assert!(plate.words.starts_with("… "), "its own head gives way");
        assert!(plate.words.ends_with("now"), "the newest words stay");
    }

    #[test]
    fn short_words_stay_whole() {
        let mut plate = Plate::default();
        plate.set_words(SHORT.into(), " is to finish".into());
        assert_eq!(plate.words, "So the plan for today is to finish");
        assert_eq!(plate.tail, SHORT.len() + 1);
    }

    #[test]
    fn prose_without_separators_cuts_between_characters() {
        let mut plate = Plate::default();
        plate.set_words("早".repeat(300), String::new());
        assert!(plate.words.starts_with("… "));
        assert_eq!(plate.words.chars().count(), WORD_BUDGET + 2);
    }

    #[test]
    fn right_to_left_words_stay_inside_the_plate() {
        // The words are one paragraph now: bidi orders the runs, so the
        // reading order needs no row flip, and the plate only has to keep
        // the text inside the overlay.
        for (stable, tentative) in [(HEBREW, " והיום"), ("", HEBREW), (HEBREW, "")] {
            let rt = runtime(stable, tentative, false);
            let [x, _, w, _] = rect(&rt, "plate").unwrap();
            assert!(x >= 0.0 && x + w <= SIZE.0, "inside: {x} {w}");
            assert!(rect(&rt, "words").is_some());
        }
    }

    #[test]
    fn a_spot_places_the_plate_just_past_the_hud() {
        assert_eq!(
            Spot::Edge(ScreenEdge::Bottom, 50.0).placement(),
            OverlayPlacement::Edge {
                edge: ScreenEdge::Bottom,
                margin: 50.0 - PAD,
                align: rune::Align::Center,
                inset: 0.0,
            }
        );
        let rect = |below| match (Spot::Global {
            x: 700.0,
            y: 100.0,
            below,
        })
        .placement()
        {
            OverlayPlacement::Rect { x, y, width, .. } => (x, y, width),
            other => panic!("{other:?}"),
        };
        // Centred on the HUD; the plate's top edge at the spot when hanging
        // below it, its bottom edge at the spot when rising above it.
        assert_eq!(rect(true), (700.0 - SIZE.0 / 2.0, 100.0 - PAD, SIZE.0));
        assert_eq!(
            rect(false),
            (700.0 - SIZE.0 / 2.0, 100.0 - SIZE.1 + PAD, SIZE.0)
        );
        // The pill hugs the side the HUD is on.
        assert!(!Spot::Edge(ScreenEdge::Top, 50.0).hugs_bottom());
        assert!(Spot::Edge(ScreenEdge::Bottom, 50.0).hugs_bottom());
        assert!(
            !Spot::Global {
                x: 0.0,
                y: 0.0,
                below: true
            }
            .hugs_bottom()
        );
        assert!(
            Spot::Global {
                x: 0.0,
                y: 0.0,
                below: false
            }
            .hugs_bottom()
        );
    }

    #[test]
    fn a_failure_arriving_after_a_queued_words_end_stays_visible() {
        let mut plate = Plate::default();
        plate.set_words("old words".into(), String::new());
        plate.mark_shown(true);

        // A queued words-ended update runs first and would hide an empty
        // plate, but the failure queued behind it brings the plate back.
        plate.words_ended();
        assert!(plate.should_hide());
        plate.show_failure("No microphone is available.".into());
        assert!(!plate.should_hide());
        assert_eq!(plate.words, "old words", "the old words stay for the fade");

        // Once the notice is up, a later words end cannot hide it.
        plate.words_ended();
        assert!(!plate.should_hide());
        assert!(plate.take_failure());
        assert!(plate.should_hide());
        plate.mark_shown(false);
    }

    #[test]
    fn a_failure_notice_takes_the_plate_from_the_live_words() {
        let mut plate = Plate::default();
        plate.set_words(SHORT.into(), " and then".into());
        plate.show_failure("No microphone is available. Connect a microphone.".into());
        let rt = runtime_plate(plate, false);
        assert!(rect(&rt, "plate").is_some());
        assert!(rect(&rt, "failure").is_some());
        assert!(rect(&rt, "words").is_none());
    }

    #[test]
    fn no_words_draw_nothing() {
        let rt = runtime("  ", "", false);
        assert!(rect(&rt, "plate").is_none());
        let blank = |stable: &str, tentative: &str| {
            is_blank(&Partial {
                stable: stable.into(),
                tentative: tentative.into(),
            })
        };
        assert!(blank("", ""));
        assert!(blank("  ", " "));
        assert!(!blank("", " so"));
        assert!(!blank("so", ""));
    }

    /// Renders the plate to `VOINC_LIVE_TEXT_SNAPS/live-text-*.png` in both
    /// appearances, to look at.
    #[test]
    #[ignore = "set VOINC_LIVE_TEXT_SNAPS to a folder; needs a GPU"]
    fn snapshots() {
        let folder = std::path::PathBuf::from(std::env::var("VOINC_LIVE_TEXT_SNAPS").unwrap());
        let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
        for (name, stable, tentative) in [
            ("short", SHORT, ""),
            ("long", LONG, ""),
            ("overlong", OVERLONG, ""),
            ("tentative", SHORT, " is to finish"),
            ("rtl", HEBREW, " והיום"),
        ] {
            for dark in [false, true] {
                let mut rt = runtime(stable, tentative, dark);
                let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
                let mut renderer = crate::rune_ui::offscreen::renderer(&gpu.device);
                let target = crate::rune_ui::offscreen::target(&gpu.device, w, h);
                renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
                renderer.prepare(
                    &gpu.device,
                    &gpu.queue,
                    &mut rt.text.atlas,
                    &rt.instances,
                    (w, h),
                );
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                // A desktop grey behind the clear overlay.
                let backdrop = if dark { 0.08 } else { 0.62 };
                renderer.draw(
                    &mut encoder,
                    &target.view,
                    [backdrop, backdrop, backdrop, 1.0],
                );
                target.copy(&mut encoder);
                gpu.queue.submit([encoder.finish()]);
                let theme = if dark { "dark" } else { "light" };
                let path = folder.join(format!("live-text-{name}-{theme}.png"));
                rune::render::write_png(&path, w, h, &target.read(&gpu.device)).unwrap();
            }
        }
    }
}
