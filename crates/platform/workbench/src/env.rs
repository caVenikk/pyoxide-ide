//! Environment switches for automated runs and measurements.
//!
//! | variable | effect |
//! |---|---|
//! | `PYOXIDE_BACKGROUND=1` | open windows without activating the app or taking focus |
//! | `PYOXIDE_THEME=light` | start with the light theme (default dark) |
//! | `PYOXIDE_SHOT=<file.png>` | save an image of the main window after it settled, then quit (macOS) |
//! | `PYOXIDE_BENCH=startup` | print the first-frame marker and quit |

use std::sync::OnceLock;
use std::time::Instant;

use themes::ThemeKind;

/// Open windows in the background (`PYOXIDE_BACKGROUND=1`): shown, but the
/// app is not activated and the window does not become key, so the person's
/// frontmost application keeps the keyboard.
pub fn background() -> bool {
    std::env::var_os("PYOXIDE_BACKGROUND").is_some_and(|v| v == "1")
}

/// Theme kind requested with `PYOXIDE_THEME`.
pub fn theme_kind() -> ThemeKind {
    match std::env::var("PYOXIDE_THEME").as_deref() {
        Ok("light") => ThemeKind::Light,
        _ => ThemeKind::Dark,
    }
}

/// `PYOXIDE_SHOT`: where to save a screenshot of the main window.
pub fn shot_path() -> Option<std::path::PathBuf> {
    std::env::var_os("PYOXIDE_SHOT").map(Into::into)
}

/// The measurement requested with `PYOXIDE_BENCH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bench {
    /// Print `PYOXIDE first_frame` and quit.
    Startup,
}

pub fn bench() -> Option<Bench> {
    match std::env::var("PYOXIDE_BENCH").ok()?.as_str() {
        "startup" => Some(Bench::Startup),
        _ => None,
    }
}

static MAIN: OnceLock<Instant> = OnceLock::new();

/// Records the start of `main`; call it first.
pub fn mark_main() {
    MAIN.get_or_init(Instant::now);
}

/// Milliseconds since [`mark_main`].
pub fn since_main_ms() -> f64 {
    MAIN.get()
        .map(|t| t.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Prints a measurement marker line (`PYOXIDE <text>`) and flushes stdout.
pub fn marker(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "PYOXIDE {text}");
    let _ = out.flush();
}
