//! What the Data Sources dialog keeps between its openings and sessions:
//! the width of the list left of the splitter, in `data-sources-window.toml`
//! of the configuration folder (the reference keeps its splitter
//! proportions as window state too). Read and written off the UI thread,
//! only with native windows (tests keep it in memory).

use std::path::PathBuf;

use gpui::{App, AppContext as _, Global};
use serde::{Deserialize, Serialize};

/// The file's name in the configuration folder.
pub const FILE: &str = "data-sources-window.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Layout {
    /// The list's width in pixels.
    list_width: Option<f32>,
}

#[derive(Default)]
struct Kept {
    file: Option<PathBuf>,
    layout: Layout,
}

impl Global for Kept {}

/// Reads the kept layout from the configuration folder in the background,
/// once; the windows draw again when it is read.
pub(crate) fn load(cx: &mut App) {
    if cx.has_global::<Kept>() {
        return;
    }
    let file = settings::config_dir()
        .filter(|_| ui::pointer::native())
        .map(|d| d.join(FILE));
    let Some(file) = file else {
        cx.set_global(Kept::default());
        return;
    };
    cx.set_global(Kept {
        file: Some(file.clone()),
        layout: Layout::default(),
    });
    let read = cx.background_spawn(async move {
        std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| toml::from_str::<Layout>(&text).ok())
    });
    cx.spawn(async move |cx| {
        if let Some(layout) = read.await {
            let _ = cx.update(|cx| {
                if let Some(kept) = cx.try_global::<Kept>()
                    && kept.layout == Layout::default()
                {
                    cx.global_mut::<Kept>().layout = layout;
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

/// The list's width as last left, if any.
pub(crate) fn list_width(cx: &App) -> Option<f32> {
    cx.try_global::<Kept>()?.layout.list_width
}

/// Keeps the list's width and writes it in the background.
pub(crate) fn set_list_width(width: f32, cx: &mut App) {
    if !cx.has_global::<Kept>() {
        cx.set_global(Kept::default());
    }
    let kept = cx.global_mut::<Kept>();
    kept.layout.list_width = Some(width.round());
    let (Some(file), Ok(text)) = (kept.file.clone(), toml::to_string(&kept.layout)) else {
        return;
    };
    cx.background_spawn(async move {
        let _ = settings::write_atomic(&file, &text);
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_reads_back_what_it_wrote() {
        let layout = Layout {
            list_width: Some(312.0),
        };
        let text = toml::to_string(&layout).unwrap();
        assert_eq!(text.trim(), "list-width = 312.0");
        assert_eq!(toml::from_str::<Layout>(&text).unwrap(), layout);
        assert_eq!(toml::from_str::<Layout>("").unwrap(), Layout::default());
    }
}
