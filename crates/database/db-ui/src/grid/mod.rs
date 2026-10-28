//! The data grid of a table (ADR 0021, "UI"; scenario grid-open-table):
//! an editor tab that a double click, Enter, F4 or Cmd+Down on a table or
//! view of the Database tool window opens. Under its toolbar the WHERE and
//! ORDER BY fields (their text goes into the statement as written), then
//! the grid with the column headers (key marks, the type in the tooltip,
//! a click on the sort mark orders on the server), and the page bar
//! floating over its bottom (`1-500 of 501+`, the count on request, the
//! page size menu).
//!
//! Rows come only from the data source's [`ResultPager`]: a page at a time
//! on a data connection, each read cancellable (Stop, Escape); a failing
//! read keeps the rows shown and puts the server's message in a balloon
//! under the WHERE field. Nothing waits on the UI thread: cell texts are
//! made in the background when a page arrives, and the grid draws only the
//! cells in view ([`body`]).
//!
//! The same view shows a console's result (`TableGrid::result`): its rows
//! come from the session's statement, the WHERE and ORDER BY fields are
//! not there, and the pages are the ones the session has fetched.
//!
//! Read-only: editing is a later issue. Values are shown typed and in the
//! formats of the settings ([`value`], [`formats`]); the side panel
//! ([`panel`]: record, value, aggregates), the other views ([`views`]) and
//! the whole value of a cut cell are in [`values`].

pub mod aggregate;
mod body;
mod cell_menu;
pub mod data;
pub mod format;
pub mod formats;
mod panel;
mod render;
mod sql_highlight;
mod status;
pub mod value;
mod values;
pub mod views;

use std::sync::Arc;

use db_core::Backend;
use db_session::{
    DataSourceHandle, JobId, PageError, PageSize, PagerEvent, ResultPager, TableQuery, TableRef,
};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable,
    Global, Pixels, Point, SharedString, Subscription, Task, WeakEntity, Window, point, px,
};
use serde_json::{Value as Json, json};
use ui::command::CommandError;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use workbench::Workbench;
use workbench::tab_view::TabView;

use data::{ColumnMeta, GridPage, Selection};

pub use status::status_widget;

/// The key context of the grid's cells with the focus (its bindings in
/// `grid.toml`).
pub const KEY_CONTEXT: &str = "DatabaseGrid";
/// The key context of the whole tab (toolbar, fields and cells).
pub const VIEW_CONTEXT: &str = "DatabaseGridView";
/// The key context around the WHERE and ORDER BY fields.
pub const FIELD_CONTEXT: &str = "DatabaseGridField";

/// Page sizes of the page size menu.
pub const PAGE_SIZES: [usize; 5] = [10, 100, 250, 500, 1000];
/// The page size of a new grid until another is set as the default.
pub const DEFAULT_PAGE_SIZE: usize = 500;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Open the data of a table in a tab.
    OpenTable { source: String, schema: Option<String>, table: String } => "db.grid.open", "Open Table",
        "Opens the data of table or view `table` of `schema` in data source `source` in an editor tab (or selects its open tab) and reads its first page; the data source must be shown in the Database tool window.",
        r#"{"source": "local-pg", "schema": "demo", "table": "events"}"#;
    /// Open the selected table of the Database tool window.
    OpenSelected => "db.explorer.open-table", "Jump to Data",
        "Opens the data of the table or view selected in the Database tool window in an editor tab (F4); on any other node it does nothing.";
    /// Read the shown page again.
    Reload => "db.grid.reload", "Reload Page",
        "Reads the shown page of the active table grid again with the text of its WHERE and ORDER BY fields.";
    /// Stop the running read.
    Stop => "db.grid.stop", "Cancel Running Statements",
        "Cancels the page or count the active table grid is reading; the rows shown stay.";
    /// The first page.
    FirstPage => "db.grid.first-page", "First Page", "Shows the first page of the active table grid.";
    /// The previous page.
    PreviousPage => "db.grid.previous-page", "Previous Page", "Shows the previous page of the active table grid.";
    /// The next page.
    NextPage => "db.grid.next-page", "Next Page", "Shows the next page of the active table grid.";
    /// The last page.
    LastPage => "db.grid.last-page", "Last Page",
        "Shows the last page of the active table grid; the rows are counted first when their number is not known.";
    /// Count the rows.
    CountRows => "db.grid.count", "Count Rows",
        "Counts the rows of the active table grid's table with its WHERE condition (a separate COUNT(*)) and shows the total in the page bar.";
    /// Set the page size.
    SetPageSize { size: usize } => "db.grid.set-page-size", "Page Size",
        "Sets the page size of the active table grid (`0`: all rows) and reads its first page again.",
        r#"{"size": 100}"#;
    /// Use the page size for new grids.
    SetDefaultPageSize => "db.grid.set-default-page-size", "Set as Default",
        "Makes the active table grid's page size the page size of grids opened later.";
    /// Show the page size menu.
    PageSizeMenu => "db.grid.page-size-menu", "Change Page Size",
        "Shows the page size menu of the active table grid's page bar.";
    /// Set the WHERE text.
    SetFilter { text: String } => "db.grid.set-filter", "Set WHERE",
        "Puts `text` into the WHERE field of the active table grid and reads the first page with it.",
        r#"{"text": "kind = 'error'"}"#;
    /// Set the ORDER BY text.
    SetOrder { text: String } => "db.grid.set-order", "Set ORDER BY",
        "Puts `text` into the ORDER BY field of the active table grid and reads the first page with it.",
        r#"{"text": "at desc"}"#;
    /// Apply the fields.
    ApplyFields => "db.grid.apply-fields", "Apply",
        "Reads the first page of the active table grid with the text of its WHERE and ORDER BY fields (Enter in a field).";
    /// Clear a field.
    ClearField { field: String } => "db.grid.clear-field", "Clear",
        "Empties the `where` or `order` field of the active table grid and reads the first page again.",
        r#"{"field": "where"}"#;
    /// Put the focus into a field.
    FocusField { field: String } => "db.grid.focus-field", "Focus Field",
        "Puts the focus into the `where` or `order` field of the active table grid, or into its cells with `cells`.",
        r#"{"field": "where"}"#;
    /// Sort by a column.
    SortByColumn { column: usize } => "db.grid.sort", "Sort",
        "Sorts the active grid by column `column` (zero-based): ascending, then descending, then unsorted. A table grid orders on the server (the ORDER BY field shows the order); the grid of a console's result orders the rows it holds, the statement is not run again.",
        r#"{"column": 1}"#;
    /// Select cells.
    SelectCells { row: usize, column: usize, to_row: Option<usize>, to_column: Option<usize> } => "db.grid.select", "Select Cells",
        "Selects the cell `row`, `column` (zero-based in the shown page) of the active table grid, or the rectangle to `to_row`, `to_column`.",
        r#"{"row": 0, "column": 2}"#;
    /// Select the column of the current cell.
    SelectColumn { column: usize } => "db.grid.select-column", "Select Column",
        "Selects every row of column `column` (zero-based) of the active table grid (a click on the header's name).",
        r#"{"column": 2}"#;
    /// Move up.
    MoveUp => "db.grid.up", "Up", "Moves the current cell of the active table grid up.";
    /// Move down.
    MoveDown => "db.grid.down", "Down", "Moves the current cell of the active table grid down.";
    /// Move left.
    MoveLeft => "db.grid.left", "Left", "Moves the current cell of the active table grid left.";
    /// Move right.
    MoveRight => "db.grid.right", "Right", "Moves the current cell of the active table grid right.";
    /// Extend up.
    SelectUp => "db.grid.select-up", "Up with Selection", "Extends the selection of the active table grid up.";
    /// Extend down.
    SelectDown => "db.grid.select-down", "Down with Selection", "Extends the selection of the active table grid down.";
    /// Extend left.
    SelectLeft => "db.grid.select-left", "Left with Selection", "Extends the selection of the active table grid left.";
    /// Extend right.
    SelectRight => "db.grid.select-right", "Right with Selection", "Extends the selection of the active table grid right.";
    /// A screen up.
    PageUp => "db.grid.page-up", "Page Up", "Moves the current cell of the active table grid a screen up.";
    /// A screen down.
    PageDown => "db.grid.page-down", "Page Down", "Moves the current cell of the active table grid a screen down.";
    /// The first column.
    RowStart => "db.grid.row-start", "Move to Row Start", "Moves the current cell of the active table grid to the first column.";
    /// The last column.
    RowEnd => "db.grid.row-end", "Move to Row End", "Moves the current cell of the active table grid to the last column.";
    /// The first row.
    FirstRow => "db.grid.first-row", "Move to First Row", "Moves the current cell of the active table grid to the first row of the page.";
    /// The last row.
    LastRow => "db.grid.last-row", "Move to Last Row", "Moves the current cell of the active table grid to the last row of the page.";
    /// Select everything.
    SelectAll => "db.grid.select-all", "Select All", "Selects every cell of the page of the active table grid.";
    /// The state of the grid.
    GridState => "db.grid.state", "Table Grid State",
        "Returns the state of the most recently used table grid of the window: its table, fields, statement, page, columns, the first rows' texts, selection and status.";
    /// Close the balloon.
    Dismiss => "db.grid.dismiss", "Close Message",
        "Closes the error balloon of the active table grid, or cancels its running read.";
    /// Hide a column.
    HideColumn { column: usize } => "db.grid.hide-column", "Hide Column",
        "Hides column `column` (zero-based) of the active table grid; the statement does not change.",
        r#"{"column": 2}"#;
    /// Hide all but one column.
    HideOtherColumns { column: usize } => "db.grid.hide-other-columns", "Hide Other Columns",
        "Hides every column of the active table grid but `column` (zero-based).",
        r#"{"column": 2}"#;
    /// Show every column.
    ShowAllColumns => "db.grid.show-all-columns", "Show All Columns",
        "Shows the columns of the active table grid that were hidden.";
}

/// Grids of the main windows, the most recently used last.
#[derive(Default)]
struct Grids(Vec<(AnyWindowHandle, WeakEntity<TableGrid>)>);

impl Global for Grids {}

/// The page size of new grids ("Set as Default").
struct DefaultPageSize(PageSize);

impl Global for DefaultPageSize {}

fn default_page_size(cx: &App) -> PageSize {
    cx.try_global::<DefaultPageSize>()
        .map_or(PageSize::Rows(DEFAULT_PAGE_SIZE), |d| d.0)
}

/// Marks `grid` as the most recently used of its window.
pub(crate) fn touch(window: AnyWindowHandle, grid: &Entity<TableGrid>, cx: &mut App) {
    let weak = grid.downgrade();
    let grids = &mut cx.default_global::<Grids>().0;
    grids.retain(|(_, g)| g.upgrade().is_some() && *g != weak);
    grids.push((window, weak));
}

/// The grid a command goes to: the most recently used of the active
/// window, else of any window.
fn target(cx: &App) -> Option<(AnyWindowHandle, Entity<TableGrid>)> {
    let live: Vec<_> = cx
        .try_global::<Grids>()?
        .0
        .iter()
        .filter_map(|(w, g)| g.upgrade().map(|g| (*w, g)))
        .collect();
    let active = cx.active_window();
    live.iter()
        .rev()
        .find(|(w, _)| Some(*w) == active)
        .cloned()
        .or_else(|| live.last().cloned())
}

/// The grids of a main window.
pub(crate) fn grids_of(window: AnyWindowHandle, cx: &App) -> Vec<Entity<TableGrid>> {
    cx.try_global::<Grids>()
        .map(|g| {
            g.0.iter()
                .filter(|(w, _)| *w == window)
                .filter_map(|(_, g)| g.upgrade())
                .collect()
        })
        .unwrap_or_default()
}

fn with_target(
    cx: &mut App,
    f: impl FnOnce(&mut TableGrid, &mut Window, &mut Context<TableGrid>) + 'static,
) {
    let Some((window, grid)) = target(cx) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            grid.update(cx, |g, cx| f(g, window, cx))
        });
    });
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|a: &OpenTable, cx| {
        let (source, schema, table) = (a.source.clone(), a.schema.clone(), a.table.clone());
        crate::explorer::open_table_command(source, schema, table, cx);
    });
    cx.on_action(|_: &OpenSelected, cx| crate::explorer::open_selected_command(cx));
    macro_rules! on {
        ($action:ty, |$g:ident, $a:ident, $w:ident, $cx:ident| $body:expr) => {
            cx.on_action(|a: &$action, cx| {
                let $a = a.clone();
                with_target(cx, move |$g, $w, $cx| {
                    let _ = (&$a, &$w);
                    $body
                })
            });
        };
    }
    on!(Reload, |g, a, w, cx| g.reload(cx));
    on!(Stop, |g, a, w, cx| g.stop(cx));
    on!(FirstPage, |g, a, w, cx| g.first_page(cx));
    on!(PreviousPage, |g, a, w, cx| g.previous_page(cx));
    on!(NextPage, |g, a, w, cx| g.next_page(cx));
    on!(LastPage, |g, a, w, cx| g.last_page(cx));
    on!(CountRows, |g, a, w, cx| g.count(cx));
    on!(SetPageSize, |g, a, w, cx| g.set_page_size(a.size, cx));
    on!(SetDefaultPageSize, |g, a, w, cx| g
        .set_default_page_size(cx));
    on!(PageSizeMenu, |g, a, w, cx| g
        .show_page_size_menu(None, w, cx));
    on!(SetFilter, |g, a, w, cx| g.set_field(
        Field::Where,
        &a.text,
        w,
        cx
    ));
    on!(SetOrder, |g, a, w, cx| g.set_field(
        Field::Order,
        &a.text,
        w,
        cx
    ));
    on!(ApplyFields, |g, a, w, cx| g.apply_fields(cx));
    on!(ClearField, |g, a, w, cx| {
        let field = if a.field == "order" {
            Field::Order
        } else {
            Field::Where
        };
        g.set_field(field, "", w, cx)
    });
    on!(SortByColumn, |g, a, w, cx| g.sort_by(a.column, w, cx));
    on!(SelectCells, |g, a, w, cx| {
        let to = (a.to_row.unwrap_or(a.row), a.to_column.unwrap_or(a.column));
        g.select(
            Selection {
                anchor: (a.row, a.column),
                lead: to,
            },
            cx,
        )
    });
    on!(SelectColumn, |g, a, w, cx| g.select_column(a.column, cx));
    on!(HideColumn, |g, a, w, cx| g.hide_columns(
        Some(a.column),
        false,
        cx
    ));
    on!(HideOtherColumns, |g, a, w, cx| g.hide_columns(
        Some(a.column),
        true,
        cx
    ));
    on!(ShowAllColumns, |g, a, w, cx| g
        .hide_columns(None, false, cx));
    on!(MoveUp, |g, a, w, cx| g.move_by(-1, 0, false, cx));
    on!(MoveDown, |g, a, w, cx| g.move_by(1, 0, false, cx));
    on!(MoveLeft, |g, a, w, cx| g.move_by(0, -1, false, cx));
    on!(MoveRight, |g, a, w, cx| g.move_by(0, 1, false, cx));
    on!(SelectUp, |g, a, w, cx| g.move_by(-1, 0, true, cx));
    on!(SelectDown, |g, a, w, cx| g.move_by(1, 0, true, cx));
    on!(SelectLeft, |g, a, w, cx| g.move_by(0, -1, true, cx));
    on!(SelectRight, |g, a, w, cx| g.move_by(0, 1, true, cx));
    on!(PageUp, |g, a, w, cx| {
        let n = g.rows_in_view();
        g.move_by(-(n as isize), 0, false, cx)
    });
    on!(PageDown, |g, a, w, cx| {
        let n = g.rows_in_view();
        g.move_by(n as isize, 0, false, cx)
    });
    on!(RowStart, |g, a, w, cx| g.move_by(
        0,
        isize::MIN / 2,
        false,
        cx
    ));
    on!(RowEnd, |g, a, w, cx| g.move_by(
        0,
        isize::MAX / 2,
        false,
        cx
    ));
    on!(FirstRow, |g, a, w, cx| g.move_by(
        isize::MIN / 2,
        0,
        false,
        cx
    ));
    on!(LastRow, |g, a, w, cx| g.move_by(
        isize::MAX / 2,
        0,
        false,
        cx
    ));
    on!(SelectAll, |g, a, w, cx| g.select_all(cx));
    on!(Dismiss, |g, a, w, cx| g.dismiss(cx));
    on!(FocusField, |g, a, w, cx| g.focus_field(&a.field, w, cx));
    values::init(cx);
    ui::command::register_query(
        cx,
        "db.grid.state",
        std::rc::Rc::new(|_, window, cx| {
            let main = ui::dialog_window::parent_of(window.window_handle(), cx)
                .unwrap_or(window.window_handle());
            let grids = grids_of(main, cx);
            let Some(grid) = grids.last() else {
                return Err(CommandError::new("not_available", "no table grid is open"));
            };
            Ok(grid.read(cx).state_json(cx))
        }),
    );
    status::init(cx);
}

/// What a grid opens: a table of a data source, with what the metadata
/// model knows about its columns.
#[derive(Clone)]
pub struct GridTarget {
    /// The data source's name.
    pub source: String,
    pub backend: Backend,
    pub handle: DataSourceHandle,
    pub table: TableRef,
    pub columns: Vec<ColumnMeta>,
}

/// Opens `target` in an editor tab of `workbench`, or selects its tab.
pub(crate) fn open(
    workbench: &Entity<Workbench>,
    target: GridTarget,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TableGrid> {
    let source = target.source.clone();
    let table = target.table.clone();
    let main = window.window_handle();
    let grid = workbench.update(cx, |wb, cx| {
        wb.open_tab_view(
            move |g: &TableGrid, _| g.source == source && g.table == table,
            move |window, cx| cx.new(|cx| TableGrid::new(target, window, cx)),
            window,
            cx,
        )
    });
    touch(main, &grid, cx);
    let focus = grid.read(cx).focus.clone();
    window.focus(&focus);
    grid
}

/// The WHERE or the ORDER BY field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Where,
    Order,
}

/// Asks the console's session for the next page of the result.
pub(crate) type FetchNext = std::rc::Rc<dyn Fn(&mut App)>;

/// The pages of a console's result: the ones fetched so far, and which
/// one is shown.
struct ResultState {
    columns: Vec<db_core::Column>,
    pages: Vec<Arc<GridPage>>,
    index: usize,
    fetch: Option<FetchNext>,
    /// A fetch of the next page is under way.
    fetching: bool,
    /// The column the shown rows are ordered by. The reference orders the
    /// rows of a query result it has fetched itself (the ORDER BY of the
    /// statement is the statement's own); `pages` hold them as they came.
    sort: Option<(usize, data::SortDir)>,
}

/// What a running page read is for.
#[derive(Debug, Clone)]
struct Load {
    job: JobId,
    query: TableQuery,
    /// Keep the selection (a reload of the same page).
    keep_selection: bool,
}

/// The server's message of a failed read, shown in a balloon.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridError {
    pub text: String,
    /// Where the server located it, when it is in the WHERE text: its
    /// character offset there.
    pub filter_offset: Option<usize>,
}

/// An open popup of the grid.
struct OpenMenu {
    menu: Entity<ui::menu::ContextMenu>,
    position: Point<Pixels>,
    /// The menu's corner that is at `position`.
    corner: gpui::Corner,
    _subscription: Subscription,
}

/// A column being resized by its header's edge.
#[derive(Debug, Clone, Copy)]
struct Resize {
    column: usize,
    start_x: Pixels,
    start_width: Pixels,
}

/// The view of a table's data in an editor tab.
pub struct TableGrid {
    source: String,
    backend: Backend,
    table: TableRef,
    meta: Vec<ColumnMeta>,
    pager: ResultPager,
    /// The cells' focus (what the tab focuses).
    focus: FocusHandle,
    /// The whole tab's.
    outer: FocusHandle,
    filter: Entity<editor::Editor>,
    order: Entity<editor::Editor>,
    /// The query of the shown page.
    query: TableQuery,
    page_size: PageSize,
    page: Option<Arc<GridPage>>,
    load: Option<Load>,
    /// The total when counted for the shown query.
    total: Option<u64>,
    counting: Option<(JobId, TableQuery)>,
    /// Go to the last page when the count arrives.
    last_after_count: bool,
    error: Option<GridError>,
    selection: Option<Selection>,
    /// Scroll offset of the cells (the header follows horizontally).
    scroll: Point<Pixels>,
    /// Widths set by dragging a header's edge, by column name.
    widths: std::collections::HashMap<SharedString, Pixels>,
    /// Columns hidden in the view (by name); the statement is not changed.
    hidden: std::collections::HashSet<SharedString>,
    resize: Option<Resize>,
    /// The left button went down on a cell: moving extends the selection.
    drag_select: bool,
    /// Where the page bar was drawn, for its menu.
    page_bar_bounds: std::rc::Rc<std::cell::Cell<gpui::Bounds<Pixels>>>,
    hover: Option<data::Pos>,
    menu: Option<OpenMenu>,
    /// The value panel, views and whole values ([`values`]).
    values: values::ValueState,
    body: body::BodyState,
    build_task: Task<()>,
    /// Set for the grid of a console's result.
    result: Option<ResultState>,
    _events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for TableGrid {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TabView for TableGrid {
    fn tab_title(&self, _: &App) -> SharedString {
        self.table.name.clone().into()
    }

    fn tab_icon(&self, _: &App) -> IconName {
        crate::icons::TABLE
    }

    fn tab_tooltip(&self, _: &App) -> Option<SharedString> {
        let schema = self
            .table
            .schema
            .as_deref()
            .map(|s| format!("{s}."))
            .unwrap_or_default();
        Some(format!("{}: {schema}{}", self.source, self.table.name).into())
    }

    fn tab_state(&self, cx: &App) -> Json {
        self.state_json(cx)
    }
}

impl TableGrid {
    fn new(target: GridTarget, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(target, false, window, cx)
    }

    /// The grid of a console's result named `title`; rows are given by
    /// [`Self::show_page`].
    pub(crate) fn result(
        target: GridTarget,
        columns: Vec<db_core::Column>,
        fetch: Option<FetchNext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut grid = Self::build(target, true, window, cx);
        grid.result = Some(ResultState {
            columns,
            pages: Vec::new(),
            index: 0,
            fetch,
            fetching: false,
            sort: None,
        });
        grid
    }

    fn build(
        target: GridTarget,
        is_result: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<PagerEvent>();
        let events = cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = rx.next().await {
                if this.update(cx, |g, cx| g.on_event(event, cx)).is_err() {
                    break;
                }
            }
        });
        let pager = target.handle.pager(move |e| {
            let _ = tx.unbounded_send(e);
        });
        let filter = editor::field::new("", None, false, window, cx);
        let order = editor::field::new("", None, false, window, cx);
        let dialect = db_session::syntax_dialect(target.backend.dialect());
        for field in [&filter, &order] {
            field.update(cx, |e, cx| {
                e.set_highlighter(
                    Some(Box::new(sql_highlight::SqlHighlighter::new(dialect))),
                    cx,
                )
            });
        }
        let focus = cx.focus_handle();
        let outer = cx.focus_handle();
        let main = window.window_handle();
        let subscriptions = vec![
            cx.on_focus_in(&outer, window, move |_, _, cx| {
                let me = cx.entity();
                touch(main, &me, cx);
                status::changed(cx);
            }),
            cx.on_focus_out(&outer, window, |_, _, _, cx| status::changed(cx)),
            cx.observe(&filter, |_, _, cx| cx.notify()),
            cx.observe(&order, |_, _, cx| cx.notify()),
        ];
        let mut grid = Self {
            source: target.source,
            backend: target.backend,
            query: TableQuery::new(target.table.clone()),
            table: target.table,
            meta: target.columns,
            pager,
            focus,
            outer,
            filter,
            order,
            page_size: default_page_size(cx),
            page: None,
            load: None,
            total: None,
            counting: None,
            last_after_count: false,
            error: None,
            selection: None,
            scroll: point(px(0.0), px(0.0)),
            widths: Default::default(),
            hidden: Default::default(),
            resize: None,
            drag_select: false,
            page_bar_bounds: Default::default(),
            hover: None,
            menu: None,
            values: Default::default(),
            body: body::BodyState::default(),
            build_task: Task::ready(()),
            result: None,
            _events: events,
            _subscriptions: subscriptions,
        };
        if !is_result {
            grid.read_page(grid.query.clone(), 0, false, cx);
        }
        grid
    }

    /// The grid shows a console's result.
    pub(crate) fn is_result(&self) -> bool {
        self.result.is_some()
    }

    /// A page of the console's result arrived: the first one replaces
    /// what was shown, later ones are added (and shown).
    pub(crate) fn show_page(
        &mut self,
        rows: Vec<db_core::Row>,
        first_row: u64,
        has_more: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(columns) = self.result.as_ref().map(|r| r.columns.clone()) else {
            return;
        };
        let meta = self.meta.clone();
        let fmt = self.cell_format(cx);
        let build = cx.background_spawn(async move {
            GridPage::build(&columns, rows, first_row, has_more, &meta, &fmt)
        });
        self.build_task = cx.spawn(async move |this, cx| {
            let page = Arc::new(build.await);
            let _ = this.update(cx, |g, cx| {
                let Some(state) = g.result.as_mut() else {
                    return;
                };
                if page.first_row == 0 {
                    state.pages.clear();
                }
                state.fetching = false;
                state.pages.push(page.clone());
                state.index = state.pages.len() - 1;
                let sort = state.sort;
                g.page = Some(match sort {
                    Some((column, dir)) => Arc::new(page.sorted(column, dir)),
                    None => page,
                });
                g.selection = None;
                g.hover = None;
                g.scroll.y = px(0.0);
                status::changed(cx);
                cx.notify();
            });
        });
    }

    /// The result has no more rows to fetch (its cursor was closed).
    pub(crate) fn result_closed(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.result.as_mut() {
            state.fetch = None;
            state.fetching = false;
        }
        cx.notify();
    }

    fn show_result_page(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.result.as_mut() else {
            return;
        };
        let Some(page) = state.pages.get(index).cloned() else {
            return;
        };
        state.index = index;
        self.page = Some(match state.sort {
            Some((column, dir)) => Arc::new(page.sorted(column, dir)),
            None => page,
        });
        self.selection = None;
        self.scroll.y = px(0.0);
        cx.notify();
    }

    /// The sort marks of the headers: the result's own sort, or the ORDER
    /// BY text of a table.
    fn sort_marks(&self, page: &GridPage) -> Vec<(usize, data::SortDir)> {
        match &self.result {
            Some(state) => state.sort.into_iter().collect(),
            None => data::sort_of(&self.query.order, &page.columns),
        }
    }

    /// A click on a result's sort mark: ascending, descending, then the
    /// order the rows came in; the page shown is ordered at once.
    fn sort_result(&mut self, column: usize, cx: &mut Context<Self>) {
        let Some(state) = self.result.as_mut() else {
            return;
        };
        let Some(unsorted) = state.pages.get(state.index).cloned() else {
            return;
        };
        if column >= unsorted.column_count() {
            return;
        }
        state.sort = data::next_sort(state.sort, column);
        self.page = Some(match state.sort {
            Some((column, dir)) => Arc::new(unsorted.sorted(column, dir)),
            None => unsorted,
        });
        self.selection = None;
        cx.notify();
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn table(&self) -> &TableRef {
        &self.table
    }

    fn dialect(&self) -> db_core::Dialect {
        self.backend.dialect()
    }

    // ---- reading ----------------------------------------------------------------

    /// Starts reading the page of `query` at `first_row`, cancelling a read
    /// that runs.
    fn read_page(&mut self, query: TableQuery, first_row: u64, keep: bool, cx: &mut Context<Self>) {
        if let Some(load) = self.load.take() {
            self.pager.cancel(load.job);
        }
        let size = self.page_size;
        let job = self.pager.page(&query, first_row, size);
        self.load = Some(Load {
            job,
            query,
            keep_selection: keep,
        });
        cx.notify();
    }

    fn on_event(&mut self, event: PagerEvent, cx: &mut Context<Self>) {
        match event {
            PagerEvent::Page { job, result } => {
                let Some(load) = self.load.clone().filter(|l| l.job == job) else {
                    return;
                };
                match result {
                    Ok(page) => {
                        // The cell texts are made off the UI thread; the
                        // read counts as running until they are there.
                        let meta = self.meta.clone();
                        let fmt = self.cell_format(cx);
                        let build = cx.background_spawn(async move {
                            GridPage::build(
                                &page.columns,
                                page.rows,
                                page.first_row,
                                page.has_more,
                                &meta,
                                &fmt,
                            )
                        });
                        self.build_task = cx.spawn(async move |this, cx| {
                            let page = build.await;
                            let _ = this.update(cx, |g, cx| g.page_built(load, page, cx));
                        });
                    }
                    Err(e) => {
                        self.load = None;
                        self.failed(&e, &load.query);
                        cx.notify();
                    }
                }
            }
            PagerEvent::Cell { job, result } => self.on_cell(job, result, cx),
            PagerEvent::Count { job, result } => {
                let Some((_, query)) = self.counting.clone().filter(|(j, _)| *j == job) else {
                    return;
                };
                self.counting = None;
                match result {
                    Ok(total) => {
                        if query == self.query {
                            self.total = Some(total);
                            if std::mem::take(&mut self.last_after_count) {
                                self.go_last(total, cx);
                            }
                        }
                    }
                    Err(e) => {
                        self.last_after_count = false;
                        self.failed(&e, &query);
                    }
                }
                cx.notify();
            }
        }
    }

    fn page_built(&mut self, load: Load, page: GridPage, cx: &mut Context<Self>) {
        if self.load.as_ref().map(|l| l.job) != Some(load.job) {
            return;
        }
        self.load = None;
        if load.query != self.query {
            self.total = None;
        }
        self.query = load.query;
        self.error = None;
        let rows = page.row_count();
        let columns = page.column_count();
        self.selection = if load.keep_selection {
            self.selection.and_then(|s| s.clamp(rows, columns))
        } else {
            None
        };
        if !load.keep_selection {
            self.scroll.y = px(0.0);
        }
        self.page = Some(Arc::new(page));
        self.values_page_changed();
        self.hover = None;
        status::changed(cx);
        cx.notify();
    }

    fn failed(&mut self, e: &PageError, query: &TableQuery) {
        if e.is_cancelled() {
            return;
        }
        let mut text = match &e.error.code {
            Some(code) => format!("[{code}] {}", e.error.message),
            None => e.error.message.clone(),
        };
        let mut filter_offset = None;
        if let Some(position) = e.error.position {
            text.push_str(&format!("\nPosition: {}", position + 1));
            filter_offset = filter_offset_of(&e.sql, position, &query.filter);
        }
        self.error = Some(GridError {
            text,
            filter_offset,
        });
    }

    fn field_text(&self, field: Field, cx: &App) -> String {
        match field {
            Field::Where => self.filter.read(cx).text(),
            Field::Order => self.order.read(cx).text(),
        }
    }

    /// The query of the fields' text.
    fn fields_query(&self, cx: &App) -> TableQuery {
        TableQuery {
            table: self.table.clone(),
            filter: self.field_text(Field::Where, cx).trim().to_owned(),
            order: self.field_text(Field::Order, cx).trim().to_owned(),
        }
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.is_result() {
            return;
        }
        let query = self.fields_query(cx);
        let same = query == self.query;
        let first = if same {
            self.page.as_ref().map_or(0, |p| p.first_row)
        } else {
            0
        };
        if !same {
            self.total = None;
        }
        self.read_page(query, first, same, cx);
    }

    pub(crate) fn apply_fields(&mut self, cx: &mut Context<Self>) {
        if self.is_result() {
            return;
        }
        let query = self.fields_query(cx);
        self.read_page(query, 0, false, cx);
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(load) = self.load.take() {
            self.pager.cancel(load.job);
            self.build_task = Task::ready(());
        }
        if let Some((job, _)) = self.counting.take() {
            self.pager.cancel(job);
            self.last_after_count = false;
        }
        self.cancel_full();
        cx.notify();
    }

    pub(crate) fn is_running(&self) -> bool {
        self.load.is_some() || self.counting.is_some()
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.error.take().is_some() {
            cx.notify();
        } else if self.is_running() {
            self.stop(cx);
        }
    }

    fn page_rows(&self) -> Option<usize> {
        self.page_size.rows()
    }

    pub(crate) fn first_page(&mut self, cx: &mut Context<Self>) {
        if self.is_result() {
            self.show_result_page(0, cx);
            return;
        }
        self.read_page(self.query.clone(), 0, false, cx);
    }

    pub(crate) fn previous_page(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &self.result {
            if state.index > 0 {
                self.show_result_page(state.index - 1, cx);
            }
            return;
        }
        let (Some(size), Some(page)) = (self.page_rows(), self.page.as_ref()) else {
            return;
        };
        if page.first_row == 0 {
            return;
        }
        let first = page.first_row.saturating_sub(size as u64);
        self.read_page(self.query.clone(), first, false, cx);
    }

    pub(crate) fn next_page(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.result.as_mut() {
            if state.index + 1 < state.pages.len() {
                let next = state.index + 1;
                self.show_result_page(next, cx);
            } else if !state.fetching
                && self.page.as_ref().is_some_and(|p| p.has_more)
                && let Some(fetch) = state.fetch.clone()
            {
                state.fetching = true;
                cx.defer(move |cx| fetch(cx));
                cx.notify();
            }
            return;
        }
        let (Some(size), Some(page)) = (self.page_rows(), self.page.as_ref()) else {
            return;
        };
        if !page.has_more {
            return;
        }
        let first = page.first_row + size as u64;
        self.read_page(self.query.clone(), first, false, cx);
    }

    pub(crate) fn last_page(&mut self, cx: &mut Context<Self>) {
        if self.is_result() {
            return;
        }
        if self.page_rows().is_none() {
            return;
        }
        match self.total {
            Some(total) => self.go_last(total, cx),
            None => {
                self.last_after_count = true;
                self.count(cx);
            }
        }
    }

    fn go_last(&mut self, total: u64, cx: &mut Context<Self>) {
        let Some(size) = self.page_rows() else {
            return;
        };
        let size = size as u64;
        let first = total.saturating_sub(1) / size * size;
        self.read_page(self.query.clone(), first, false, cx);
    }

    pub(crate) fn count(&mut self, cx: &mut Context<Self>) {
        if self.is_result() {
            return;
        }
        if let Some((job, _)) = self.counting.take() {
            self.pager.cancel(job);
        }
        let query = self.query.clone();
        let job = self.pager.count(&query);
        self.counting = Some((job, query));
        cx.notify();
    }

    pub(crate) fn set_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.is_result() {
            return;
        }
        self.menu = None;
        self.page_size = if size == 0 {
            PageSize::All
        } else {
            PageSize::Rows(size)
        };
        self.read_page(self.query.clone(), 0, false, cx);
    }

    fn set_default_page_size(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        cx.set_global(DefaultPageSize(self.page_size));
        cx.notify();
    }

    fn set_field(&mut self, field: Field, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        let editor = match field {
            Field::Where => &self.filter,
            Field::Order => &self.order,
        };
        editor.update(cx, |e, cx| e.set_text_selected(text, cx));
        self.apply_fields(cx);
    }

    fn focus_field(&mut self, field: &str, window: &mut Window, cx: &mut Context<Self>) {
        let handle = match field {
            "where" => self.filter.focus_handle(cx),
            "order" => self.order.focus_handle(cx),
            _ => self.focus.clone(),
        };
        window.focus(&handle);
        cx.notify();
    }

    fn sort_by(&mut self, column: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_result() {
            self.sort_result(column, cx);
            return;
        }
        let Some(page) = self.page.clone().filter(|_| !self.is_transposed()) else {
            return;
        };
        let Some(c) = page.columns.get(column) else {
            return;
        };
        let quoted = db_session::syntax_dialect(self.dialect()).quote(&c.name);
        let order = data::next_order(
            &self.field_text(Field::Order, cx),
            &page.columns,
            column,
            quoted,
        );
        self.set_field(Field::Order, &order, window, cx);
    }

    // ---- selection --------------------------------------------------------------

    pub(crate) fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        let Some(page) = self.shown_page() else {
            return;
        };
        self.selection = selection.clamp(page.row_count(), page.column_count());
        self.reveal(cx);
        status::changed(cx);
        cx.notify();
    }

    fn select_column(&mut self, column: usize, cx: &mut Context<Self>) {
        let Some(page) = self.shown_page() else {
            return;
        };
        let last = page.row_count().saturating_sub(1);
        let top = self.selection.map_or(0, |s| s.lead.0.min(last));
        self.select(
            Selection {
                anchor: (0, column),
                lead: (last, column),
            },
            cx,
        );
        // The current cell stays in the row it was in.
        if let Some(s) = &mut self.selection {
            s.lead.0 = last;
            let _ = top;
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.shown_page() else {
            return;
        };
        let rows = page.row_count();
        let columns = page.column_count();
        if rows == 0 || columns == 0 {
            return;
        }
        self.selection = Some(Selection {
            anchor: (0, 0),
            lead: (rows - 1, columns - 1),
        });
        status::changed(cx);
        cx.notify();
    }

    /// Moves the current cell, or the selection's lead with `extend`.
    fn move_by(&mut self, rows: isize, columns: isize, extend: bool, cx: &mut Context<Self>) {
        let Some(page) = self.shown_page() else {
            return;
        };
        let (n_rows, n_columns) = (page.row_count(), page.column_count());
        if n_rows == 0 || n_columns == 0 {
            return;
        }
        let current = self.selection.unwrap_or_default();
        let step = |v: usize, d: isize, n: usize| -> usize {
            (v as isize).saturating_add(d).clamp(0, n as isize - 1) as usize
        };
        let lead = if self.selection.is_none() {
            (0, 0)
        } else {
            (
                step(current.lead.0, rows, n_rows),
                self.shown_column_step(current.lead.1, columns, n_columns),
            )
        };
        let selection = if extend {
            Selection {
                anchor: current.anchor,
                lead,
            }
        } else {
            Selection::cell(lead)
        };
        self.select(selection, cx);
    }

    /// Whether column `column` is hidden.
    fn is_hidden(&self, column: usize) -> bool {
        // Hidden columns are the table view's.
        if self.is_transposed() {
            return false;
        }
        self.page
            .as_ref()
            .and_then(|p| p.columns.get(column))
            .is_some_and(|c| self.hidden.contains(&c.name))
    }

    /// The shown column `by` steps from `from` (a hidden one is skipped; at
    /// an end the move stops).
    fn shown_column_step(&self, from: usize, by: isize, count: usize) -> usize {
        let dir = by.signum();
        let mut at = from;
        let mut left = by.unsigned_abs();
        while left > 0 {
            let next = at as isize + dir;
            if next < 0 || next >= count as isize {
                break;
            }
            at = next as usize;
            if !self.is_hidden(at) {
                left -= 1;
            }
        }
        // Not on a hidden column: the nearest shown one back.
        while self.is_hidden(at) && at > 0 {
            at -= 1;
        }
        if self.is_hidden(at) {
            while at + 1 < count && self.is_hidden(at) {
                at += 1;
            }
        }
        at
    }

    /// Hides `column` (or every other column with `others`); with `None`
    /// shows all columns again.
    fn hide_columns(&mut self, column: Option<usize>, others: bool, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_ref().filter(|_| !self.is_transposed()) else {
            return;
        };
        match column {
            None => self.hidden.clear(),
            Some(c) => {
                let Some(name) = page.columns.get(c).map(|m| m.name.clone()) else {
                    return;
                };
                if others {
                    self.hidden = page
                        .columns
                        .iter()
                        .map(|m| m.name.clone())
                        .filter(|n| *n != name)
                        .collect();
                } else if self.hidden.len() + 1 < page.columns.len() {
                    // The last shown column stays.
                    self.hidden.insert(name);
                }
            }
        }
        if let Some(s) = self.selection
            && self.is_hidden(s.lead.1)
        {
            self.selection = None;
        }
        status::changed(cx);
        cx.notify();
    }

    /// The header's menu of column `column` at `at`.
    pub(super) fn show_header_menu(
        &mut self,
        column: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut items = vec![
            MenuItem::action("Select Column", SelectColumn { column }),
            MenuItem::separator(),
            MenuItem::action("Hide Column", HideColumn { column }),
            MenuItem::action("Hide Other Columns", HideOtherColumns { column }),
        ];
        items.push(if self.hidden.is_empty() {
            MenuItem::unavailable("Show All Columns")
        } else {
            MenuItem::action("Show All Columns", ShowAllColumns)
        });
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |g, _, _: &DismissEvent, cx| {
            g.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position: at,
            corner: gpui::Corner::BottomLeft,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn rows_in_view(&self) -> usize {
        let h = f32::from(self.body.viewport().size.height);
        let row = f32::from(self.body.row_height());
        if row <= 0.0 {
            return 1;
        }
        ((h / row) as usize).saturating_sub(1).max(1)
    }

    /// Scrolls the current cell into view.
    fn reveal(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.selection else {
            return;
        };
        let _ = cx;
        self.scroll = self.body.reveal(self.scroll, s.lead);
    }

    // ---- state ------------------------------------------------------------------

    /// The text of the page bar's range: `1-500`, or `120 rows` when the
    /// page holds every row.
    pub(crate) fn range_text(&self) -> Option<String> {
        let page = self.page.as_ref()?;
        let n = page.row_count() as u64;
        if page.first_row == 0 && !page.has_more {
            return Some(format!("{n} {}", if n == 1 { "row" } else { "rows" }));
        }
        if n == 0 {
            return Some(format!("{}-{}", page.first_row, page.first_row));
        }
        Some(format!("{}-{}", page.first_row + 1, page.first_row + n))
    }

    /// `of 501+` while the total is not known, `of 12 000` after a count.
    pub(crate) fn total_text(&self) -> Option<String> {
        let page = self.page.as_ref()?;
        if page.first_row == 0 && !page.has_more {
            return None;
        }
        Some(match self.total {
            Some(total) => format!("of {}", group_thousands(total)),
            None => {
                let known = page.first_row + page.row_count() as u64;
                if page.has_more {
                    format!("of {}+", known + 1)
                } else {
                    format!("of {known}")
                }
            }
        })
    }

    pub(crate) fn state_json(&self, cx: &App) -> Json {
        let page = self.page.as_ref();
        json!({
            "source": self.source,
            "schema": self.table.schema,
            "table": self.table.name,
            "where": self.field_text(Field::Where, cx),
            "order": self.field_text(Field::Order, cx),
            "query": {"where": self.query.filter, "order": self.query.order},
            "sql": self.query.page_sql(
                self.dialect(),
                page.map_or(0, |p| p.first_row),
                self.page_size,
            ),
            "page_size": self.page_size.rows().unwrap_or(0),
            "loading": self.load.is_some(),
            "counting": self.counting.is_some(),
            "total": self.total,
            "range": self.range_text(),
            "total_text": self.total_text(),
            "error": self.error.as_ref().map(|e| json!({
                "text": e.text,
                "where_offset": e.filter_offset,
            })),
            "columns": page.map(|p| p.columns.iter().map(|c| json!({
                "name": c.name.as_ref(),
                "type": c.type_name.as_ref(),
                "key": match c.key {
                    data::KeyMark::None => Json::Null,
                    data::KeyMark::Primary => json!("primary"),
                    data::KeyMark::Foreign => json!("foreign"),
                },
                "right": c.right,
                "hidden": self.hidden.contains(&c.name),
            })).collect::<Vec<_>>()),
            "sort": page.map(|p| self.sort_marks(p)
                .into_iter()
                .map(|(c, d)| json!({"column": c, "desc": d == data::SortDir::Desc}))
                .collect::<Vec<_>>()),
            "first_row": page.map(|p| p.first_row),
            "rows": page.map(|p| p.row_count()),
            "has_more": page.map(|p| p.has_more),
            "cells": page.map(|p| p.cells.iter().take(20).map(|r| r.iter()
                .map(|c| c.as_ref().map_or(Json::Null, |t| json!(t.as_ref())))
                .collect::<Vec<_>>()).collect::<Vec<_>>()),
            "selection": self.selection.map(|s| json!({
                "anchor": [s.anchor.0, s.anchor.1],
                "lead": [s.lead.0, s.lead.1],
            })),
            "status": self.summary().map(|s| json!({
                "sum": s.sum, "counts": s.counts, "position": s.position,
            })),
            "page_menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
            "values": self.values_state_json(cx),
        })
    }

    pub(crate) fn summary(&self) -> Option<data::SelectionSummary> {
        Some(data::summary(
            self.shown_page()?,
            self.selection.as_ref()?,
            &|c| !self.is_hidden(c),
        ))
    }
}

/// `12000` -> `12 000`, as the reference groups the total.
pub(crate) fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Where a server's error offset into the statement falls in the WHERE
/// text, as a character offset there.
fn filter_offset_of(sql: &str, position: usize, filter: &str) -> Option<usize> {
    let filter = filter.trim();
    if filter.is_empty() {
        return None;
    }
    let start_byte = sql.find(&format!("\nWHERE {filter}"))? + "\nWHERE ".len();
    let start = sql[..start_byte].chars().count();
    let len = filter.chars().count();
    (position >= start && position < start + len).then(|| position - start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_are_grouped() {
        assert_eq!(group_thousands(12000), "12 000");
        assert_eq!(group_thousands(501), "501");
        assert_eq!(group_thousands(1234567), "1 234 567");
    }

    #[test]
    fn error_offsets_map_into_the_where_text() {
        let sql = "SELECT t.*\nFROM demo.events t\nWHERE nonexistent = 1\nLIMIT 501";
        let at = sql.find("nonexistent").unwrap();
        assert_eq!(filter_offset_of(sql, at, "nonexistent = 1"), Some(0));
        assert_eq!(filter_offset_of(sql, at + 14, "nonexistent = 1"), Some(14));
        assert_eq!(filter_offset_of(sql, 3, "nonexistent = 1"), None);
        assert_eq!(filter_offset_of(sql, at, ""), None);
    }
}
