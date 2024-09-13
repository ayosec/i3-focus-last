mod focustracker;
mod rqueue;
mod setup;

use std::{rc::Rc, sync::Mutex};

use tokio::{
    io::{unix::AsyncFd, Interest},
    sync::{oneshot, watch, Notify},
};

use xcb::x;

#[derive(Clone)]
pub struct DisplayServer(Rc<DisplayInner>);

struct DisplayInner {
    connection: AsyncFd<xcb::Connection>,
    atoms: Atoms,
    roots: Box<[x::Window]>,
    requests: rqueue::Queue<DisplayServer>,
    focus_tracker: focustracker::FocusTracker,
    xkb_state_watcher: Mutex<Option<watch::Sender<x::ModMask>>>,
    switch_command: Notify,
}

pub struct Atoms {
    pub net_active_window: x::Atom,
    pub switch_command: x::Atom,
}

impl DisplayServer {
    pub fn new() -> Result<Self, xcb::Error> {
        let (conn, _) =
            xcb::Connection::connect_with_extensions(None, &[xcb::Extension::Xkb], &[])?;

        setup::use_xkb(&conn)?;

        let atoms = setup::intern_atoms(&conn)?;
        let roots = setup::listen_root_properties(&conn)?;
        let connection = AsyncFd::with_interest(conn, Interest::READABLE).unwrap();

        let display = DisplayInner {
            connection,
            atoms,
            roots,
            requests: rqueue::Queue::new(),
            focus_tracker: Default::default(),
            xkb_state_watcher: Default::default(),
            switch_command: Default::default(),
        };

        Ok(DisplayServer(Rc::new(display)))
    }

    #[inline]
    pub fn connection(&self) -> &xcb::Connection {
        self.0.connection.get_ref()
    }

    #[inline]
    fn is_root(&self, window: x::Window) -> bool {
        self.0.roots.iter().any(|&r| r == window)
    }

    #[inline]
    pub fn atoms(&self) -> &Atoms {
        &self.0.atoms
    }

    #[inline]
    pub fn roots(&self) -> &[x::Window] {
        &self.0.roots[..]
    }

    #[inline]
    pub fn switch_command(&self) -> &Notify {
        &self.0.switch_command
    }

    fn handle_root_property(&self, prop: x::PropertyNotifyEvent) {
        if prop.state() != x::Property::NewValue {
            // Ignore non-NewValue notifications.
            return;
        }

        if prop.atom() == self.0.atoms.net_active_window {
            self.0.focus_tracker.track(prop.window(), self.clone());
        }
    }

    fn handle_xkb_state(&self, state: xcb::xkb::StateNotifyEvent) {
        if let Some(watcher) = &*self.0.xkb_state_watcher.lock().unwrap() {
            if watcher.send(state.mods()).is_ok() {
                return;
            }
        }

        // If we receive a state notification, but there are no receivers,
        // stop watching XKB notifications.
        xkb_select_events(self.connection(), false);
    }

    fn handle_client_message(&self, msg: x::ClientMessageEvent) {
        if msg.r#type() == self.0.atoms.switch_command {
            self.0.switch_command.notify_waiters();
        }
    }

    pub async fn main_loop(&self) -> Result<(), xcb::Error> {
        while let Ok(mut guard) = self.0.connection.readable().await {
            // Events.
            while let Some(event) = self.connection().poll_for_event()? {
                match event {
                    xcb::Event::X(x::Event::PropertyNotify(prop))
                        if self.is_root(prop.window()) =>
                    {
                        self.handle_root_property(prop);
                    }

                    xcb::Event::X(x::Event::ClientMessage(msg)) => {
                        if self.is_root(msg.window()) {
                            self.handle_client_message(msg);
                        }
                    }

                    xcb::Event::Xkb(xcb::xkb::Event::StateNotify(state)) => {
                        self.handle_xkb_state(state);
                    }

                    unknown => {
                        eprintln!("Unexpected event: {unknown:?}");
                    }
                }
            }

            // Replies from requests.
            self.0.requests.process_queue(self);

            guard.clear_ready();
            self.connection().flush()?;
        }

        Ok(())
    }

    pub async fn send_request<R>(
        &self,
        request: &R,
    ) -> Result<<R::Cookie as xcb::CookieWithReplyChecked>::Reply, xcb::Error>
    where
        R: xcb::Request + 'static,
        R::Cookie: xcb::CookieWithReplyChecked,
    {
        let cookie = self.connection().send_request(request);
        self.connection().flush()?;

        let (tx, rx) = oneshot::channel();
        let mut tx = Some(tx);

        self.0.requests.add(Box::new(move |c| {
            match c.connection().poll_for_reply(&cookie) {
                Some(r) => {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(r);
                    }

                    false
                }

                None => true,
            }
        }));

        match rx.await {
            Ok(r) => r,
            Err(_) => Err(xcb::Error::Connection(xcb::ConnError::Connection)),
        }
    }

    pub fn watch_xkb_state(&self) -> watch::Receiver<x::ModMask> {
        let mut xkb_state_watcher = self.0.xkb_state_watcher.lock().unwrap();

        match &*xkb_state_watcher {
            Some(w) => {
                // Add a new subscriber to the existing watcher.
                w.subscribe()
            }

            None => {
                // No previous watcher.
                //
                // Create a new watcher and configure XKB events.

                xkb_select_events(self.connection(), true);

                let (tx, rx) = watch::channel(x::ModMask::empty());
                *xkb_state_watcher = Some(tx.clone());

                xkb_close_listener(self.clone(), tx);

                rx
            }
        }
    }

    pub fn switch_window(&self) -> Option<x::Window> {
        self.0.focus_tracker.switch()
    }
}

/// Enable or disable the notifications when the modifiers state is updated.
fn xkb_select_events(conn: &xcb::Connection, active: bool) {
    let events = xcb::xkb::EventType::STATE_NOTIFY;
    let map = xcb::xkb::MapPart::MODIFIER_MAP;

    let select_all;
    let clear;

    if active {
        select_all = events;
        clear = xcb::xkb::EventType::empty();
    } else {
        select_all = xcb::xkb::EventType::empty();
        clear = events;
    }

    let request = xcb::xkb::SelectEvents {
        device_spec: xcb::xkb::Id::UseCoreKbd as xcb::xkb::DeviceSpec,
        affect_which: events,
        clear,
        select_all,
        affect_map: map,
        map,
        details: &[],
    };

    if let Err(e) = conn.check_request(conn.send_request_checked(&request)) {
        eprintln!("xkb_select_events(*, {active}): {e}");
    }
}

/// Wait until `tx` is closed to disable XKB notifications.
fn xkb_close_listener(display: DisplayServer, tx: watch::Sender<x::ModMask>) {
    tokio::task::spawn_local(async move {
        tx.closed().await;

        *display.0.xkb_state_watcher.lock().unwrap() = None;
        xkb_select_events(display.connection(), false);
    });
}
