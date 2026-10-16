//! Pointer shapes in a headless window: links, lists, text links and what
//! is drawn over an element that asks for another shape; and a check that
//! no code asks GPUI for a shape behind `ui::pointer`'s back.

use std::path::{Path, PathBuf};

use gpui::{
    Context, CursorStyle, InteractiveElement as _, IntoElement, Modifiers, ParentElement as _,
    Pixels, Point, Render, Styled as _, StyledText, TestAppContext, VisualTestContext, Window, div,
    point, px,
};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt as _;

struct Fixture;

impl Render for Fixture {
    #[allow(clippy::single_range_in_vec_init)] // one link range
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = cx.theme().mono_family();
        div()
            .relative()
            .size_full()
            .font_family(mono)
            .text_size(px(13.0))
            .child(ui::pointer::frame())
            // A link.
            .child(div().absolute().top(px(10.0)).left(px(10.0)).child(
                ui::link::link("docs-link", "Docs", cx).debug_selector(|| "docs-link".into()),
            ))
            // A list popup's list.
            .child(
                div()
                    .absolute()
                    .top(px(40.0))
                    .left(px(10.0))
                    .w(px(100.0))
                    .h(px(40.0))
                    .debug_selector(|| "list".into())
                    .pointing_hand(),
            )
            // Text with a link in the middle.
            .child(
                div().absolute().top(px(90.0)).left(px(10.0)).flex().child(
                    ui::link::linked_text(
                        "text",
                        StyledText::new("ab link cd"),
                        vec![3..7],
                        |_, _, _| {},
                    )
                    .debug_selector(|| "text".into()),
                ),
            )
            // An editor: the text cursor over it. Over it, a button that
            // takes the mouse from it, and one that does not.
            .child(
                div()
                    .absolute()
                    .top(px(130.0))
                    .left(px(10.0))
                    .w(px(400.0))
                    .h(px(150.0))
                    .debug_selector(|| "editor".into())
                    .pointer(CursorStyle::IBeam)
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .right(px(8.0))
                            .size(px(24.0))
                            .block_mouse_except_scroll()
                            .debug_selector(|| "overlay".into()),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(8.0))
                            .right(px(8.0))
                            .size(px(24.0))
                            .debug_selector(|| "loose-overlay".into()),
                    ),
            )
    }
}

fn setup(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| ui::init(cx, themes::ThemeKind::Dark));
    let (_, cx) = cx.add_window_view(|_, _| Fixture);
    draw(cx);
    cx
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

/// The shape with the mouse at `at`.
fn shape(cx: &mut VisualTestContext, at: Point<Pixels>) -> CursorStyle {
    cx.simulate_mouse_move(at, None, Modifiers::none());
    draw(cx);
    cx.update(|window, _| ui::pointer::resolve(window))
}

fn center(cx: &mut VisualTestContext, selector: &'static str) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

#[gpui::test]
fn a_link_shows_the_hand_and_is_underlined_under_the_mouse(cx: &mut TestAppContext) {
    let cx = setup(cx);
    let id = gpui::ElementId::from("docs-link");
    assert!(!ui::pointer::link_hovered(&id));
    let at = center(cx, "docs-link");
    assert_eq!(shape(cx, at), CursorStyle::PointingHand);
    assert!(ui::pointer::link_hovered(&id));
    assert_eq!(shape(cx, point(px(300.0), px(60.0))), CursorStyle::Arrow);
    assert!(!ui::pointer::link_hovered(&id));
}

#[gpui::test]
fn a_list_shows_the_hand(cx: &mut TestAppContext) {
    let cx = setup(cx);
    let at = center(cx, "list");
    assert_eq!(shape(cx, at), CursorStyle::PointingHand);
}

#[gpui::test]
fn only_the_link_ranges_of_a_text_show_the_hand(cx: &mut TestAppContext) {
    let cx = setup(cx);
    let bounds = cx.debug_bounds("text").expect("the text is drawn");
    // A monospaced font: ten characters of equal width.
    let char_width = bounds.size.width / 10.0;
    let y = bounds.center().y;
    let at = |index: f32| point(bounds.left() + char_width * (index + 0.5), y);
    assert_eq!(shape(cx, at(1.0)), CursorStyle::Arrow);
    assert_eq!(shape(cx, at(5.0)), CursorStyle::PointingHand);
    assert_eq!(shape(cx, at(8.0)), CursorStyle::Arrow);
}

#[gpui::test]
fn what_is_drawn_over_an_editor_takes_the_text_cursor_from_it(cx: &mut TestAppContext) {
    let cx = setup(cx);
    let editor = cx.debug_bounds("editor").expect("the editor is drawn");
    let text = point(editor.left() + px(20.0), editor.top() + px(60.0));
    assert_eq!(shape(cx, text), CursorStyle::IBeam);
    // A button that blocks the mouse (the Soft-Wrap button over a preview).
    let at = center(cx, "overlay");
    assert_eq!(shape(cx, at), CursorStyle::Arrow);
    // One that does not lets the editor's request through: an overlay
    // must block the mouse.
    let at = center(cx, "loose-overlay");
    assert_eq!(shape(cx, at), CursorStyle::IBeam);
}

/// GPUI's own `cursor_pointer()` and the like never reach `ui::pointer`,
/// which sets the pointer on every mouse move and so overwrites them with
/// the arrow; so code asks for shapes through `ui::pointer` only.
#[test]
fn no_code_asks_gpui_for_a_pointer_shape_itself() {
    const STYLES: &[&str] = &[
        "default",
        "pointer",
        "text",
        "move",
        "not_allowed",
        "context_menu",
        "crosshair",
        "vertical_text",
        "alias",
        "copy",
        "no_drop",
        "grab",
        "grabbing",
        "ew_resize",
        "ns_resize",
        "nesw_resize",
        "nwse_resize",
        "col_resize",
        "row_resize",
        "n_resize",
        "e_resize",
        "s_resize",
        "w_resize",
    ];
    let mut banned: Vec<String> = STYLES.iter().map(|s| format!(".cursor_{s}()")).collect();
    banned.extend(
        [
            ".set_cursor_style(",
            ".set_window_cursor_style(",
            "InteractiveText::new(",
        ]
        .map(String::from),
    );
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    sources(&crates, &mut files);
    assert!(files.len() > 100, "found only {} sources", files.len());
    let mut found = Vec::new();
    for file in files {
        let name = file.to_string_lossy().replace('\\', "/");
        // The modules that own these calls, and this check.
        if name.ends_with("ui/src/pointer.rs")
            || name.ends_with("ui/src/link.rs")
            || name.ends_with("ui/tests/pointer.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(b) = banned.iter().find(|b| line.contains(b.as_str())) {
                found.push(format!("{name}:{}: {b}", n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "use ui::pointer (PointerExt) or ui::link instead:\n{}",
        found.join("\n")
    );
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && !name.to_string_lossy().starts_with('.') {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
