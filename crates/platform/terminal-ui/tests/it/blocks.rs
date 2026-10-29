//! The command blocks of a shell with integration: the marks of OSC 133
//! split the output into blocks, the layout leaves room above every block
//! after the first, a block whose command failed is marked, and the mouse
//! finds the cell under a position in spite of the room. A shell command
//! prints the marks and hands over to `cat`.
#![cfg(unix)]

use gpui::{TestAppContext, point, px};

use crate::view::{open, wait};

/// A prompt, a command that succeeded, a prompt, a command that failed, a
/// prompt.
const SESSION: &str = concat!(
    r"\033]133;A\007$ \033]133;B\007one\r\n\033]133;C\007out\r\n\033]133;D;0\007",
    r"\033]133;A\007$ \033]133;B\007two\r\n\033]133;C\007bad\r\n\033]133;D;1\007",
    r"\033]133;A\007$ \033]133;B\007",
);

fn blocks(s: &serde_json::Value) -> Vec<serde_json::Value> {
    s["blocks"]["visible"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[gpui::test]
fn blocks_have_room_above_them_and_failed_ones_are_marked(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "/bin/sh",
        &["-c", &format!("printf '{SESSION}'; exec cat")],
    );
    let s = wait(cx, &view, "three blocks", |s| blocks(s).len() == 3);
    let b = blocks(&s);
    // The first block has no room above it; the others have.
    assert_eq!(b[0]["start_row"], 0);
    assert_eq!(b[0]["separated"], false);
    assert_eq!(b[1]["start_row"], 2);
    assert_eq!(b[1]["separated"], true);
    assert_eq!(b[1]["failed"], true);
    assert_eq!(b[0]["failed"], false);
    assert_eq!(b[2]["start_row"], 4);
    assert_eq!(b[2]["current"], true);

    // The rows after a block's start are pushed down by the room.
    let g = view.read_with(cx, |v, _| v.geometry()).expect("laid out");
    let height = f32::from(g.cell.height);
    let room = |row: usize| f32::from(g.top_of(row) - g.origin.y) - height * row as f32;
    assert_eq!(room(0), 0.0);
    assert_eq!(room(1), 0.0);
    assert_eq!(room(2), terminal_ui::blocks::BLOCK_GAP);
    assert_eq!(room(3), terminal_ui::blocks::BLOCK_GAP);
    assert_eq!(room(4), 2.0 * terminal_ui::blocks::BLOCK_GAP);

    // The mouse finds the row under it, whatever the room above.
    for row in 0..5 {
        let y = g.top_of(row) + g.cell.height * 0.5;
        let (found, _) = g.cell_at(point(g.origin.x + px(1.0), y));
        assert_eq!(found.line, row);
    }
    // A position in the room above a block belongs to the row above or
    // below it, never to a row further away.
    let in_room = g.top_of(2) - px(3.0);
    let (found, _) = g.cell_at(point(g.origin.x + px(1.0), in_room));
    assert!(found.line == 1 || found.line == 2, "{}", found.line);
}

#[gpui::test]
fn a_screen_without_marks_has_no_blocks(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "/bin/sh",
        &["-c", "printf 'plain\\r\\ntext\\r\\n'; exec cat"],
    );
    let s = wait(cx, &view, "the text", |s| s["lines"][1] == "text");
    assert!(blocks(&s).is_empty());
    let g = view.read_with(cx, |v, _| v.geometry()).expect("laid out");
    assert_eq!(g.top_of(1) - g.origin.y, g.cell.height);
}
