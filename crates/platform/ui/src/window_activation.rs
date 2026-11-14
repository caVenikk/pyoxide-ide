//! Workaround for the macOS key-window delegate in the pinned GPUI 0.2.2.
#![allow(unexpected_cfgs)] // objc 0.2 message macros

use std::sync::{Once, OnceLock};

use objc::runtime::{BOOL, Class, Imp, Object, Sel, YES, method_setImplementation};
use objc::{msg_send, sel, sel_impl};

type Delegate = extern "C" fn(&Object, Sel, *mut Object);
static WINDOW_DELEGATE: OnceLock<Delegate> = OnceLock::new();
static PANEL_DELEGATE: OnceLock<Delegate> = OnceLock::new();

/// Called during application startup, before creating any native windows.
pub fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        for (name, original, wrapper) in [
            (
                "GPUIWindow",
                &WINDOW_DELEGATE,
                window_did_become_key as Delegate,
            ),
            (
                "GPUIPanel",
                &PANEL_DELEGATE,
                panel_did_become_key as Delegate,
            ),
        ] {
            let Some(class) = Class::get(name) else {
                continue;
            };
            // Only replace a method declared by GPUI itself, never an inherited one.
            let Some(method) = class
                .instance_methods()
                .iter()
                .copied()
                .find(|method| method.name() == sel!(windowDidBecomeKey:))
            else {
                continue;
            };
            // SAFETY: GPUI registers this selector with the Delegate ABI.
            let delegate = unsafe { std::mem::transmute::<Imp, Delegate>(method.implementation()) };
            if original.set(delegate).is_ok() {
                // SAFETY: publish the saved delegate before installing its same-ABI wrapper.
                unsafe {
                    method_setImplementation(
                        std::ptr::from_ref(method).cast_mut(),
                        std::mem::transmute::<Delegate, Imp>(wrapper),
                    );
                }
            }
        }
    });
}

extern "C" fn window_did_become_key(window: &Object, selector: Sel, notification: *mut Object) {
    did_become_key(
        window,
        selector,
        notification,
        *WINDOW_DELEGATE.get().unwrap(),
    );
}

extern "C" fn panel_did_become_key(panel: &Object, selector: Sel, notification: *mut Object) {
    did_become_key(
        panel,
        selector,
        notification,
        *PANEL_DELEGATE.get().unwrap(),
    );
}

fn did_become_key(window: &Object, selector: Sel, notification: *mut Object, original: Delegate) {
    // SAFETY: GPUI uses each live NSWindow or NSPanel as its own delegate.
    let is_key: BOOL = unsafe { msg_send![window, isKeyWindow] };
    if is_key == YES {
        original(window, selector, notification);
    } else {
        // GPUI balances a spurious become-key notification by resigning. Do this
        // before entering GPUI: AppKit can synchronously call its resign delegate,
        // which needs the same window-state mutex.
        unsafe {
            let _: () = msg_send![window, resignKeyWindow];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc::{class, declare::ClassDecl};
    use std::cell::RefCell;

    thread_local! { static EVENTS: RefCell<Vec<Sel>> = const { RefCell::new(Vec::new()) }; }

    // Reenter with a genuine activation while the balancing resignation is on the stack.
    extern "C" fn resign(window: &Object, selector: Sel) {
        EVENTS.with_borrow_mut(|v| v.push(selector));
        did_become_key(
            window,
            sel!(windowDidBecomeKey:),
            std::ptr::null_mut(),
            record,
        );
    }

    extern "C" fn is_key(_: &Object, _: Sel) -> BOOL {
        EVENTS.with_borrow(|v| if v.is_empty() { objc::runtime::NO } else { YES })
    }

    extern "C" fn record(_: &Object, selector: Sel, _: *mut Object) {
        EVENTS.with_borrow_mut(|v| v.push(selector));
    }

    #[test]
    fn spurious_become_resigns_and_allows_reentrant_activation() {
        let mut class = ClassDecl::new("ReentrantKeyWindowTest", class!(NSObject)).unwrap();
        // SAFETY: the test object implements the two NSWindow selectors the adapter sends.
        unsafe {
            class.add_method(
                sel!(isKeyWindow),
                is_key as extern "C" fn(&Object, Sel) -> BOOL,
            );
            class.add_method(sel!(resignKeyWindow), resign as extern "C" fn(&Object, Sel));
            let window: *mut Object = msg_send![class.register(), new];
            did_become_key(
                &*window,
                sel!(windowDidBecomeKey:),
                std::ptr::null_mut(),
                record,
            );
            let _: () = msg_send![window, release];
        }
        EVENTS.with_borrow(|v| {
            assert_eq!(
                v.as_slice(),
                [sel!(resignKeyWindow), sel!(windowDidBecomeKey:)]
            )
        });
    }

    #[test]
    fn installs_once_on_both_gpui_classes_and_preserves_resign() {
        let classes = ["GPUIWindow", "GPUIPanel"].map(|name| Class::get(name).unwrap());
        let resign = classes.map(|class| {
            class
                .instance_method(sel!(windowDidResignKey:))
                .unwrap()
                .implementation()
        });
        install();
        install();
        let wrappers = [
            window_did_become_key as Delegate,
            panel_did_become_key as Delegate,
        ];
        for ((class, resign), wrapper) in classes.into_iter().zip(resign).zip(wrappers) {
            assert!(std::ptr::fn_addr_eq(
                class
                    .instance_method(sel!(windowDidResignKey:))
                    .unwrap()
                    .implementation(),
                resign
            ));
            assert_eq!(
                class
                    .instance_method(sel!(windowDidBecomeKey:))
                    .unwrap()
                    .implementation() as usize,
                wrapper as usize
            );
        }
    }
}
