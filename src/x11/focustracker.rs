use std::{cell::Cell, rc::Rc};

use xcb::x;

#[derive(Default)]
pub struct FocusTracker(Rc<FocusTrackerInner>);

#[derive(Default)]
struct FocusTrackerInner {
    cookie: Cell<usize>,
    current: Cell<Option<x::Window>>,
    current_accepted: Cell<bool>,
    last: Cell<Option<x::Window>>,
}

impl FocusTracker {
    pub fn track(&self, root_window: x::Window, display: super::DisplayServer) {
        let ft = self.0.clone();

        let cookie = ft.cookie.get() + 1;
        ft.cookie.set(cookie);

        tokio::task::spawn_local(track(cookie, root_window, ft, display));
    }

    /// Return the `last` window, and swap it with `current`.
    pub fn switch(&self) -> Option<x::Window> {
        self.0.last.swap(&self.0.current);
        self.0.current.get()
    }
}

async fn track(
    cookie: usize,
    root_window: x::Window,
    ft: Rc<FocusTrackerInner>,
    display: super::DisplayServer,
) {
    macro_rules! cookie {
        () => {
            if cookie != ft.cookie.get() {
                return;
            }
        };
    }

    macro_rules! request {
        ($req:expr) => {
            match display.send_request(&$req).await {
                Ok(reply) => {
                    cookie!();
                    reply
                }

                Err(err) => {
                    eprintln!("{}", err);
                    return;
                }
            }
        };
    }

    // Store the initial XKB state, so we don't need to wait for changes
    // in the modifiers if there none of them are active.
    let initial_xkb_mods = {
        let req = xcb::xkb::GetState {
            device_spec: xcb::xkb::Id::UseCoreKbd as xcb::xkb::DeviceSpec,
        };

        request!(req).mods()
    };

    // New value of the _NET_ACTIVE_WINDOW property.
    let active_window: x::Window = {
        let req = xcb::x::GetProperty {
            delete: false,
            window: root_window,
            property: display.atoms().net_active_window,
            r#type: x::ATOM_WINDOW,
            long_offset: 0,
            long_length: 1,
        };

        match request!(req).value().first().copied() {
            Some(w) => w,
            None => {
                eprintln!("No window in _NET_ACTIVE_WINDOW");
                return;
            }
        }
    };

    // If the `active_window` is the current one, just mark it
    // as accepted.
    if ft.current.get() == Some(active_window) {
        ft.current_accepted.set(true);
        return;
    }

    // Register the new window. Don't replace `last` unless `current`
    // is accepted.
    if ft.current_accepted.get() {
        ft.last.set(ft.current.get());
    }

    ft.current.set(Some(active_window));

    // If there are no modifiers, notify the change.
    if initial_xkb_mods.is_empty() {
        ft.current_accepted.set(true);
        return;
    }

    ft.current_accepted.set(false);

    // Mark the new window as `accepted` only when all
    // keyboard modifiers are released.
    let mut rx = display.watch_xkb_state();

    loop {
        cookie!();

        if rx.changed().await.is_err() {
            return;
        }

        if rx.borrow_and_update().is_empty() {
            break;
        }
    }

    cookie!();
    ft.current_accepted.set(true);
}
