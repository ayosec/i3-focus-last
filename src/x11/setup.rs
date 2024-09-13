use super::Atoms;

use xcb::x;

pub(super) fn use_xkb(conn: &xcb::Connection) -> Result<(), xcb::Error> {
    let req = xcb::xkb::UseExtension {
        wanted_major: 1,
        wanted_minor: 0,
    };

    let cookie = conn.send_request(&req);

    conn.wait_for_reply(cookie)?;

    Ok(())
}

pub(super) fn intern_atoms(conn: &xcb::Connection) -> Result<Atoms, xcb::Error> {
    macro_rules! atom {
        ($name:expr) => {
            conn.wait_for_reply(conn.send_request(&x::InternAtom {
                only_if_exists: false,
                name: $name.as_bytes(),
            }))?
            .atom()
        };
    }

    Ok(Atoms {
        net_active_window: atom!("_NET_ACTIVE_WINDOW"),
        switch_command: atom!("x11-alternate-focus/switch"),
    })
}

pub(super) fn listen_root_properties(
    conn: &xcb::Connection,
) -> Result<Box<[x::Window]>, xcb::Error> {
    let mut roots = Vec::new();

    let setup = conn.get_setup();
    for screen in setup.roots() {
        roots.push(screen.root());

        let req = conn.send_request_checked(&x::ChangeWindowAttributes {
            window: screen.root(),
            value_list: &[x::Cw::EventMask(
                x::EventMask::PROPERTY_CHANGE | x::EventMask::STRUCTURE_NOTIFY,
            )],
        });

        conn.check_request(req)?;
    }

    Ok(roots.into_boxed_slice())
}
