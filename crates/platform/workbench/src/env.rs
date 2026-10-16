//! Environment switches for automated runs and measurements.
//!
//! | variable | effect |
//! |---|---|
//! | `PYOXIDE_BACKGROUND=1` | open windows without activating the app or taking focus |
//! | `PYOXIDE_THEME=light` | use the light theme in this run (`dark`, or a theme id), whatever the settings store, until a theme is chosen in the IDE |
//! | `PYOXIDE_SHOT=<file.png>` | save an image of the main window after it settled, then quit (macOS) |
//! | `PYOXIDE_WINDOW_SIZE=<w>x<h>` | open the main window with this content size in points, below the usual minimum too (pictures at several widths) |
//! | `PYOXIDE_RUN=<json>` | run UI commands once the window settled, e.g. `[["find.search", {"text": "def"}]]` (before `PYOXIDE_SHOT`); `workbench.simulate-mouse` moves the mouse or clicks for hover states |
//! | `PYOXIDE_BENCH=startup` | print the first-frame marker and quit |
//! | `PYOXIDE_BENCH=typing` | type 3 x 200 keys at line 50,000 of the open file, print key-to-frame times, quit |
//! | `PYOXIDE_BENCH_MARKS=<n>` | with `typing`: show `n` problems spread over the file first (underlines, error stripe, inspection widget) |
//! | `PYOXIDE_BENCH=open:<file>` | open a file, print the time to its first painted text and to highlighting, scroll it, quit |
//! | `PYOXIDE_BENCH=find:<query>` | type a query into the find bar of the open file char by char (5 runs), print key-to-highlight times, quit |
//! | `PYOXIDE_BENCH=find-in-files:<query>` | search the open project for a query with Find in Files (5 runs), print the times to the first row and to the end of the search, quit |
//! | `PYOXIDE_BENCH=go-to-file:<query>` | type a query into Go to File char by char (5 runs), print the index build time and key-to-rows times, quit |
//! | `PYOXIDE_BENCH=completion[:<ms>]` | with a service that answers after `ms` (50) with 200 items, type `.` 30 times at the end of the open file, print key-to-popup and answer-to-popup times (with `PYOXIDE_SHOT`: a picture of the popup), quit |
//! | `PYOXIDE_BENCH=cursor` | move the mouse (synthetic events) over text, gutter, splitter and toolbar, print the system cursor shown, quit |
//! | `PYOXIDE_DEBUG_CURSOR=1` | print the system cursor after every mouse move |
//! | `PYOXIDE_NATIVE_POINTER=0` | do not set the pointer shape on every mouse move (leave it to GPUI) |
//! | `PYOXIDE_CONFIG_DIR=<dir>` | keep the application settings (`settings.toml`) and other settings that last across sessions (`find.toml`, `settings-window.toml`) in this folder instead of the user's configuration folder |

use std::sync::OnceLock;
use std::time::Instant;

use themes::ThemeKind;

/// Open windows in the background (`PYOXIDE_BACKGROUND=1`): shown, but the
/// app is not activated and the window does not become key, so the person's
/// frontmost application keeps the keyboard.
pub fn background() -> bool {
    std::env::var_os("PYOXIDE_BACKGROUND").is_some_and(|v| v == "1")
}

/// Theme kind requested with `PYOXIDE_THEME` (the theme drawn before
/// the settings are read).
pub fn theme_kind() -> ThemeKind {
    match std::env::var("PYOXIDE_THEME").as_deref() {
        Ok("light") => ThemeKind::Light,
        _ => ThemeKind::Dark,
    }
}

/// The theme `PYOXIDE_THEME` pins for this run: `light`, `dark` or a
/// theme id.
pub fn theme_override() -> Option<String> {
    std::env::var("PYOXIDE_THEME")
        .ok()
        .filter(|t| !t.is_empty())
}

/// `PYOXIDE_DEBUG_CURSOR=1`: print the system cursor after every mouse move.
pub fn debug_cursor() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PYOXIDE_DEBUG_CURSOR").is_some_and(|v| v == "1"))
}

/// `PYOXIDE_SHOT`: where to save a screenshot of the main window.
pub fn shot_path() -> Option<std::path::PathBuf> {
    std::env::var_os("PYOXIDE_SHOT").map(Into::into)
}

/// `PYOXIDE_WINDOW_SIZE=<width>x<height>`: the main window's content size
/// in points; `None` when unset or not two numbers.
pub fn window_size() -> Option<(f32, f32)> {
    parse_window_size(&std::env::var("PYOXIDE_WINDOW_SIZE").ok()?)
}

fn parse_window_size(text: &str) -> Option<(f32, f32)> {
    let (w, h) = text.trim().split_once('x')?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// `PYOXIDE_RUN`: UI commands with their arguments to run once the window
/// settled (a JSON list of `[id, args]` pairs); `Err` when it does not parse.
pub fn run_commands() -> Option<Result<Vec<(String, serde_json::Value)>, String>> {
    let text = std::env::var("PYOXIDE_RUN").ok()?;
    Some(
        serde_json::from_str::<Vec<(String, serde_json::Value)>>(&text)
            .map_err(|e| format!("PYOXIDE_RUN: {e}")),
    )
}

/// The measurement requested with `PYOXIDE_BENCH` (see `bench.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bench {
    /// Print `PYOXIDE first_frame` and quit.
    Startup,
    /// Typing latency in the open file.
    Typing,
    /// Opening a file of the open project, then scrolling it.
    Open(std::path::PathBuf),
    /// The system cursor over the parts of the window.
    Cursor,
    /// Typing a query into the find bar of the open file.
    Find(String),
    /// Searching the open project with Find in Files.
    FindInFiles(String),
    /// Typing a query into Go to File.
    GoToFile(String),
    /// The completion popup in the open file, with a service that answers
    /// after this many milliseconds (`completion` or `completion:<ms>`).
    Completion(u64),
}

/// The latency of the completion bench's service by default.
pub const COMPLETION_LATENCY_MS: u64 = 50;

/// The requested measurement (read once).
pub fn bench() -> Option<&'static Bench> {
    static BENCH: OnceLock<Option<Bench>> = OnceLock::new();
    BENCH
        .get_or_init(|| {
            let value = std::env::var("PYOXIDE_BENCH").ok()?;
            match value.as_str() {
                "startup" => Some(Bench::Startup),
                "typing" => Some(Bench::Typing),
                "cursor" => Some(Bench::Cursor),
                "completion" => Some(Bench::Completion(COMPLETION_LATENCY_MS)),
                _ => value
                    .strip_prefix("open:")
                    .map(|p| Bench::Open(p.into()))
                    .or_else(|| {
                        value
                            .strip_prefix("find:")
                            .map(|q| Bench::Find(q.to_owned()))
                    })
                    .or_else(|| {
                        value
                            .strip_prefix("find-in-files:")
                            .map(|q| Bench::FindInFiles(q.to_owned()))
                    })
                    .or_else(|| {
                        value
                            .strip_prefix("go-to-file:")
                            .map(|q| Bench::GoToFile(q.to_owned()))
                    })
                    .or_else(|| {
                        value
                            .strip_prefix("completion:")
                            .and_then(|ms| ms.parse().ok())
                            .map(Bench::Completion)
                    }),
            }
        })
        .as_ref()
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

#[cfg(test)]
mod tests {
    use super::parse_window_size;

    #[test]
    fn window_sizes() {
        assert_eq!(parse_window_size("900x700"), Some((900.0, 700.0)));
        assert_eq!(parse_window_size(" 480 x 600 "), Some((480.0, 600.0)));
        assert_eq!(parse_window_size("900"), None);
        assert_eq!(parse_window_size("0x700"), None);
        assert_eq!(parse_window_size("wide x 700"), None);
    }
}
