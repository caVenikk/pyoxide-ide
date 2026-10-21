//! The recent projects, kept for the application in `recent-projects.toml`
//! of the configuration folder (the reference's `RecentProjectsManagerBase`
//! and its `recentProjects.xml`): each project's folder, name, when it was
//! opened last and its colour. A project goes first when it opens; the
//! list keeps [`MAX`] projects and keeps a folder that is gone (a removable
//! drive may come back), as the reference does. The project widget's popup
//! and the Welcome tab show it.
//!
//! A new project gets the colour after the one the previous new project
//! got, from a random one at the first (`ProjectWindowCustomizerService.
//! nextColorIndex`), and keeps it. The file is read and written off the
//! UI thread; what changes before it is read is applied to it then.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{App, Global, Hsla};
use serde::{Deserialize, Serialize};
use themes::Controls;
use ui::theme::ThemeExt;

/// The file's name in the configuration folder.
pub const FILE: &str = "recent-projects.toml";

/// Most projects the list keeps (`ide.max.recent.projects`).
pub const MAX: usize = 50;

/// How many colours a project can get (`RecentProjectColorPalette.
/// COLOR_COUNT`).
pub const COLORS: u8 = 9;

/// A project opened before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RecentProject {
    /// The project's folder.
    pub path: PathBuf,
    /// Its name (the folder's name).
    pub name: String,
    /// When it was opened last, in milliseconds since the Unix epoch.
    #[serde(default)]
    pub opened: u64,
    /// Its colour, an index into the palette ([`colors`]).
    #[serde(default)]
    pub color: u8,
}

/// The recent projects, the one opened last first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RecentProjects {
    /// The colour the last new project got.
    last_color: Option<u8>,
    #[serde(rename = "project")]
    projects: Vec<RecentProject>,
}

impl RecentProjects {
    /// The projects, the one opened last first.
    pub fn projects(&self) -> &[RecentProject] {
        &self.projects
    }

    pub fn get(&self, path: &Path) -> Option<&RecentProject> {
        self.projects.iter().find(|p| p.path == path)
    }

    /// The project at `path` was opened at `now`: it goes first, with
    /// `name`, keeping its colour; a new project gets the colour after the
    /// last one given (the first one after `seed`). Projects beyond [`MAX`]
    /// go, the oldest first, except those `open` says are open.
    pub fn opened(
        &mut self,
        path: &Path,
        name: &str,
        now: u64,
        seed: u64,
        open: impl Fn(&Path) -> bool,
    ) -> &RecentProject {
        let color = match self.projects.iter().position(|p| p.path == path) {
            Some(ix) => self.projects.remove(ix).color,
            None => {
                let last = self.last_color.unwrap_or((seed % u64::from(COLORS)) as u8);
                let next = (last + 1) % COLORS;
                self.last_color = Some(next);
                next
            }
        };
        self.projects.insert(
            0,
            RecentProject {
                path: path.to_owned(),
                name: name.to_owned(),
                opened: now,
                color,
            },
        );
        self.trim(open);
        &self.projects[0]
    }

    /// Takes a project off the list; false if it was not on it.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.projects.len();
        self.projects.retain(|p| p.path != path);
        self.projects.len() != before
    }

    /// Drops the oldest projects beyond [`MAX`] that are not open.
    fn trim(&mut self, open: impl Fn(&Path) -> bool) {
        let mut ix = self.projects.len();
        while self.projects.len() > MAX && ix > 0 {
            ix -= 1;
            if !open(&self.projects[ix].path) {
                self.projects.remove(ix);
            }
        }
    }

    /// The list stored in `text`; a broken file gives an empty list.
    /// Projects without a folder or listed twice are left out, a colour
    /// out of range wraps.
    pub fn parse(text: &str) -> Self {
        let mut list: Self = toml::from_str(text).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        list.projects
            .retain(|p| !p.path.as_os_str().is_empty() && seen.insert(p.path.clone()));
        for p in &mut list.projects {
            p.color %= COLORS;
        }
        list.last_color = list.last_color.map(|c| c % COLORS);
        list
    }

    /// The list as stored.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

/// The two colours of the project avatar of colour `index`
/// (`ProjectIconPalette`): the New UI themes' `RecentProject.ColorN.Avatar`
/// start and end, the platform's defaults in other themes.
pub fn colors(index: u8, cx: &App) -> (Hsla, Hsla) {
    let theme = cx.theme();
    let ix = usize::from(index % COLORS);
    let (start, end) = match (theme.controls(), theme.is_dark()) {
        (Controls::New, _) => NEW_UI[ix],
        (Controls::Classic, true) => CLASSIC_DARK[ix],
        (Controls::Classic, false) => CLASSIC_LIGHT[ix],
    };
    (rgb(start), rgb(end))
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// `RecentProject.Color1..9.Avatar.Start` and `.End` of the New UI themes
/// (`expUI_dark.theme.json`, `expUI_light.theme.json`).
const NEW_UI: [(u32, u32); 9] = [
    (0xE08855, 0xE9806F),
    (0xB08B14, 0xBB7F19),
    (0xA1A359, 0x87AA59),
    (0x3B92B8, 0x6183EC),
    (0x3574F0, 0x7A64F0),
    (0xC84D8F, 0xA956CF),
    (0x955AE0, 0xA84DE0),
    (0x24A394, 0x279CCD),
    (0x5FAD65, 0x3D968B),
];

/// The defaults of `ProjectIconPalette` (dark).
const CLASSIC_DARK: [(u32, u32); 9] = [
    (0xCE443C, 0xE77E41),
    (0xE27237, 0xE8A83E),
    (0x2DBCAD, 0x35D6A4),
    (0x3895E1, 0x51C5EA),
    (0x7B75E8, 0x7D99EB),
    (0x7854AD, 0x897AE6),
    (0x8F4593, 0xB572E3),
    (0xC840B9, 0xE074AE),
    (0xD75370, 0xE96FA3),
];

/// The defaults of `ProjectIconPalette` (light).
const CLASSIC_LIGHT: [(u32, u32); 9] = [
    (0xDB3D3C, 0xFF8E42),
    (0xF57236, 0xFCBA3F),
    (0x2BC8BB, 0x36EBAE),
    (0x359AF2, 0x57DBFF),
    (0x8379FB, 0x85A8FF),
    (0x7E54B5, 0x9486FF),
    (0xD63CC8, 0xF582B9),
    (0x954294, 0xC87DFF),
    (0xE75371, 0xFF78B5),
];

/// The main toolbar's colour of a project of colour `index`
/// (`RecentProjectColorPalette.mainToolbarGradientStart`): the New UI dark
/// themes' and the Islands Light theme's `RecentProject.ColorN.
/// MainToolbarGradientStart`, the platform's defaults in other themes.
pub fn toolbar_color(index: u8, cx: &App) -> Hsla {
    let theme = cx.theme();
    let ix = usize::from(index % COLORS);
    let hex = match (theme.controls(), theme.is_dark()) {
        (Controls::New, true) => TOOLBAR_NEW_UI_DARK[ix],
        (Controls::New, false) => TOOLBAR_NEW_UI_LIGHT[ix],
        (Controls::Classic, true) => CLASSIC_DARK[ix].0,
        (Controls::Classic, false) => CLASSIC_LIGHT[ix].0,
    };
    rgb(hex)
}

/// `RecentProject.ColorN.MainToolbarGradientStart` of `expUI_dark.theme.json`.
const TOOLBAR_NEW_UI_DARK: [u32; 9] = [
    0x654B40, 0x534C33, 0x455038, 0x31515F, 0x344C7D, 0x5D354A, 0x4F3E65, 0x1D4744, 0x3E5540,
];

/// `RecentProject.ColorN.MainToolbarGradientStart` of
/// `ManyIslandsLight.theme.json`.
const TOOLBAR_NEW_UI_LIGHT: [u32; 9] = [
    0xF7D7CB, 0xF5E6B8, 0xE2F2C9, 0xCBE4F7, 0xD4D2FC, 0xECD3F5, 0xE4D3F5, 0xCEF0E9, 0xCEEDD4,
];

/// The avatar of a project: its initials in its colours.
pub fn avatar(name: &str, color: u8, cx: &App) -> ui::avatar::Avatar {
    let (start, end) = colors(color, cx);
    ui::avatar::Avatar::new(name, start, end)
}

/// A change made before the file was read.
#[derive(Debug, Clone)]
enum Change {
    Opened {
        path: PathBuf,
        name: String,
        now: u64,
    },
    Removed(PathBuf),
}

/// The list of the running application and the file it is kept in.
pub struct Store {
    file: Option<PathBuf>,
    list: RecentProjects,
    /// The file was read (or there is none).
    loaded: bool,
    /// What changed before the file was read.
    pending: Vec<Change>,
    /// The list changed since it was last written.
    dirty: bool,
    /// A write is under way.
    saving: bool,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            file: None,
            list: RecentProjects::default(),
            loaded: true,
            pending: Vec::new(),
            dirty: false,
            saving: false,
        }
    }
}

impl Global for Store {}

/// Reads the list from `file` off the UI thread and writes it there from
/// now on.
pub fn load(cx: &mut App, file: PathBuf) {
    {
        let g = cx.default_global::<Store>();
        g.file = Some(file.clone());
        g.loaded = false;
    }
    let read = cx
        .background_executor()
        .spawn(async move { std::fs::read_to_string(&file).ok() });
    cx.spawn(async move |cx| {
        let text = read.await;
        let _ = cx.update(|cx| {
            let list = text
                .as_deref()
                .map(RecentProjects::parse)
                .unwrap_or_default();
            let g = cx.default_global::<Store>();
            g.list = list;
            g.loaded = true;
            for change in std::mem::take(&mut g.pending) {
                apply(&mut g.list, change);
                g.dirty = true;
            }
            schedule_save(cx);
        });
    })
    .detach();
}

/// The colour of the project at `path`, when the list has it.
pub fn color(path: &Path, cx: &App) -> Option<u8> {
    cx.try_global::<Store>()?.list.get(path).map(|p| p.color)
}

/// The recent projects now.
pub fn list(cx: &App) -> RecentProjects {
    cx.try_global::<Store>()
        .map(|g| g.list.clone())
        .unwrap_or_default()
}

/// A project was opened: it goes first in the list (a folder whose path
/// is not Unicode is not kept).
pub fn opened(cx: &mut App, path: &Path, name: &str) {
    if path.to_str().is_none() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    change(
        cx,
        Change::Opened {
            path: path.to_owned(),
            name: name.to_owned(),
            now,
        },
    );
}

/// Takes a project off the list.
pub fn remove(cx: &mut App, path: &Path) {
    change(cx, Change::Removed(path.to_owned()));
}

fn change(cx: &mut App, change: Change) {
    let g = cx.default_global::<Store>();
    if g.loaded {
        apply(&mut g.list, change);
        g.dirty = true;
    } else {
        g.pending.push(change);
    }
    schedule_save(cx);
}

fn apply(list: &mut RecentProjects, change: Change) {
    match change {
        Change::Opened { path, name, now } => {
            // A random first colour (`nextColorIndex`).
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| u64::from(d.subsec_nanos()));
            list.opened(&path, &name, now, seed, |_| false);
        }
        Change::Removed(path) => {
            list.remove(&path);
        }
    }
}

/// Writes a changed list in the background, one write at a time.
fn schedule_save(cx: &mut App) {
    let g = cx.default_global::<Store>();
    if g.saving || !g.dirty || !g.loaded {
        return;
    }
    let Some(file) = g.file.clone() else {
        g.dirty = false;
        return;
    };
    let text = g.list.to_toml();
    g.dirty = false;
    g.saving = true;
    let write = cx
        .background_executor()
        .spawn(async move { ::settings::write_atomic(&file, &text).map_err(|e| (file, e)) });
    cx.spawn(async move |cx| {
        if let Err((file, e)) = write.await {
            eprintln!("cannot save {}: {e}", file.display());
        }
        let _ = cx.update(|cx| {
            cx.global_mut::<Store>().saving = false;
            schedule_save(cx);
        });
    })
    .detach();
}

/// Where the list is kept: [`FILE`] in the configuration folder.
pub fn file() -> Option<PathBuf> {
    ::settings::config_dir().map(|d| d.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(list: &mut RecentProjects, path: &str, now: u64) -> u8 {
        let name = Path::new(path).file_name().unwrap().to_string_lossy();
        list.opened(Path::new(path), &name, now, 4, |_| false).color
    }

    fn paths(list: &RecentProjects) -> Vec<&str> {
        list.projects()
            .iter()
            .map(|p| p.path.to_str().unwrap())
            .collect()
    }

    #[test]
    fn the_project_opened_last_comes_first_once() {
        let mut list = RecentProjects::default();
        opened(&mut list, "/work/alpha", 1);
        opened(&mut list, "/work/beta", 2);
        opened(&mut list, "/work/gamma", 3);
        assert_eq!(paths(&list), ["/work/gamma", "/work/beta", "/work/alpha"]);
        opened(&mut list, "/work/alpha", 4);
        assert_eq!(paths(&list), ["/work/alpha", "/work/gamma", "/work/beta"]);
        let alpha = list.get(Path::new("/work/alpha")).unwrap();
        assert_eq!((alpha.name.as_str(), alpha.opened), ("alpha", 4));
    }

    #[test]
    fn new_projects_take_the_next_colour_and_keep_it() {
        let mut list = RecentProjects::default();
        // The first after the seed (4 % 9).
        assert_eq!(opened(&mut list, "/work/alpha", 1), 5);
        assert_eq!(opened(&mut list, "/work/beta", 2), 6);
        assert_eq!(opened(&mut list, "/work/alpha", 3), 5);
        for (i, name) in ["c", "d", "e", "f", "g"].iter().enumerate() {
            opened(&mut list, &format!("/work/{name}"), 4 + i as u64);
        }
        // 7, 8, 0, 1, 2: round the palette.
        assert_eq!(list.projects()[0].color, 2);
        assert_eq!(list.last_color, Some(2));
    }

    #[test]
    fn the_list_keeps_fifty_and_drops_the_oldest_closed_ones() {
        let mut list = RecentProjects::default();
        for i in 0..MAX + 3 {
            opened(&mut list, &format!("/work/p{i}"), i as u64);
        }
        assert_eq!(list.projects().len(), MAX);
        assert_eq!(list.projects()[0].path, Path::new("/work/p52"));
        assert_eq!(list.projects()[MAX - 1].path, Path::new("/work/p3"));
        // An open project stays, however old.
        list.opened(Path::new("/work/new"), "new", 99, 0, |p| {
            p == Path::new("/work/p3")
        });
        assert_eq!(list.projects().len(), MAX);
        assert_eq!(list.projects()[MAX - 1].path, Path::new("/work/p3"));
        assert!(list.get(Path::new("/work/p4")).is_none());
    }

    #[test]
    fn removed_projects_go_and_missing_folders_stay() {
        let mut list = RecentProjects::default();
        // Neither folder exists: both stay listed.
        opened(&mut list, "/nonexistent/alpha", 1);
        opened(&mut list, "/nonexistent/beta", 2);
        assert!(list.remove(Path::new("/nonexistent/alpha")));
        assert!(!list.remove(Path::new("/nonexistent/alpha")));
        assert_eq!(paths(&list), ["/nonexistent/beta"]);
        let read = RecentProjects::parse(&list.to_toml());
        assert_eq!(read, list);
    }

    #[test]
    fn the_file_reads_back_and_a_broken_one_is_empty() {
        let mut list = RecentProjects::default();
        opened(&mut list, "/work/alpha", 10);
        opened(&mut list, "/work/beta", 20);
        let text = list.to_toml();
        assert!(text.contains("[[project]]"), "{text}");
        assert!(text.contains("last-color = 6"), "{text}");
        assert_eq!(RecentProjects::parse(&text), list);
        assert_eq!(
            RecentProjects::parse("project = 3"),
            RecentProjects::default()
        );
        assert_eq!(RecentProjects::parse("{"), RecentProjects::default());
        // Duplicates and empty folders are left out, colours wrap.
        let text = r#"
            [[project]]
            path = "/work/a"
            name = "a"
            color = 12
            [[project]]
            path = "/work/a"
            name = "again"
            [[project]]
            path = ""
            name = "none"
        "#;
        let read = RecentProjects::parse(text);
        assert_eq!(read.projects().len(), 1);
        assert_eq!(read.projects()[0].name, "a");
        assert_eq!(read.projects()[0].color, 3);
    }
}
