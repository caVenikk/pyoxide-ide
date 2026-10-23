//! Commands of the main menu that act on the window: Close Project,
//! Minimize and Zoom (the reference's `CloseProjectAction`,
//! `MinimizeCurrentWindowAction`, `ZoomCurrentWindowAction`).

use gpui::{Context, InteractiveElement, Window};

use super::Workbench;
use crate::commands::{CloseProject, MinimizeWindow, ZoomWindow};

impl Workbench {
    /// The handlers; Close Project only when the window has a project (the
    /// menu row is disabled without one).
    pub(super) fn window_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        let root = root
            .on_action(|_: &MinimizeWindow, window, _| {
                if crate::shot::native_windows() {
                    window.minimize_window();
                }
            })
            .on_action(|_: &ZoomWindow, window, _| {
                if crate::shot::native_windows() {
                    window.zoom_window();
                }
            });
        if self.project(cx).is_some() {
            root.on_action(cx.listener(Self::close_project))
        } else {
            root
        }
    }

    /// `project.close`: modified files are saved and the window closes;
    /// the last main window is replaced by one without a project that
    /// shows the Welcome tab, in the same place (the reference shows its
    /// Welcome screen).
    fn close_project(&mut self, _: &CloseProject, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        let this = window.window_handle();
        let last = !cx
            .windows()
            .iter()
            .any(|w| w.window_id() != this.window_id() && w.downcast::<Workbench>().is_some());
        if !last {
            window.remove_window();
            return;
        }
        let product = self.product.clone();
        let bounds = window.window_bounds();
        cx.defer(move |cx| {
            let mut options = crate::app::window_options(cx, &product);
            options.window_bounds = Some(bounds);
            let new = crate::app::open_main_window_with(cx, product, options);
            let _ = new.update(cx, |workbench, window, cx| {
                let sync = crate::SyncSettings {
                    watch: true,
                    ..workbench.sync_settings()
                };
                workbench.set_sync_settings(sync);
                workbench.open_welcome(window, cx);
            });
            let _ = this.update(cx, |_, window, _| window.remove_window());
        });
    }
}
