//! The line the program is suspended on (the reference's
//! `ExecutionPointHighlighter`): a full-line background in the editors of
//! its file (`EXECUTIONPOINT_ATTRIBUTES` on the top frame,
//! `NOT_TOP_FRAME_ATTRIBUTES` on another, at
//! `EXECUTION_LINE_HIGHLIGHTERLAYER`), no gutter icon of its own.

use std::path::PathBuf;

use debugger::{Phase, SessionId, SessionSnapshot};
use editor::decorations::layer;
use editor::{Decoration, Editor};
use gpui::{App, Entity};
use themes::UiRole;

/// The decorations' group in an editor.
pub(crate) const GROUP: &str = "debugger-execution-point";

/// Where a session is suspended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionPoint {
    pub session: SessionId,
    pub path: PathBuf,
    /// From 1.
    pub line: u32,
    /// The selected frame is the top one.
    pub top: bool,
    /// The frame's id: a new stop has new frames.
    pub frame: i64,
}

impl ExecutionPoint {
    /// The selected frame of a suspended session, when its source is a
    /// file.
    pub fn of(snapshot: &SessionSnapshot) -> Option<Self> {
        if snapshot.phase != Phase::Suspended {
            return None;
        }
        let selected = snapshot.selected_frame?;
        let index = snapshot.frames.iter().position(|f| f.id == selected)?;
        let frame = &snapshot.frames[index];
        let path = frame.source.as_ref()?.path.clone()?;
        Some(Self {
            session: snapshot.id,
            path,
            line: frame.line.max(1),
            top: index == 0,
            frame: frame.id,
        })
    }

    /// The same place of the same stop.
    pub fn same_stop(&self, other: &Self) -> bool {
        self.session == other.session && self.frame == other.frame && self.line == other.line
    }
}

/// Shows `point` in an editor, or takes it away.
pub(crate) fn show(editor: &Entity<Editor>, point: Option<&ExecutionPoint>, cx: &mut App) {
    let line = point.and_then(|p| {
        let shows = editor.read(cx).path().is_some_and(|path| *path == p.path);
        shows.then_some(p.line as usize - 1)
    });
    editor.update(cx, |e, cx| match line {
        Some(line) => {
            let last = e.buffer().len_lines().saturating_sub(1);
            let line = line.min(last);
            // Frames other than the top one are selected with the frames
            // view (#166), which brings `NOT_TOP_FRAME_ATTRIBUTES`.
            e.set_decorations(
                GROUP,
                vec![(
                    line..line + 1,
                    Decoration::default()
                        .background(UiRole::ExecutionPoint, layer::EXECUTION_POINT),
                )],
                cx,
            );
        }
        None => e.clear_decorations(GROUP, cx),
    });
}
