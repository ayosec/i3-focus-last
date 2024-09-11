use xcb::x;

pub fn init(property: &str) -> (xcb::Connection, x::Window, x::Atom) {
    let (conn, screen_num) = xcb::Connection::connect(None).unwrap();
    let setup = conn.get_setup();
    let screen = setup.roots().nth(screen_num as usize).unwrap();
    let root = screen.root();

    let req = conn.send_request(&x::InternAtom {
        only_if_exists: false,
        name: property.as_bytes(),
    });

    let atom = conn.wait_for_reply(req).unwrap().atom();

    (conn, root, atom)
}

pub fn set(property: &str, value: &str) -> Result<(), xcb::Error> {
    let (conn, root, atom) = init(property);

    let req = conn.send_request_checked(&x::ChangeProperty {
        mode: x::PropMode::Replace,
        window: root,
        property: atom,
        r#type: x::ATOM_STRING,
        data: value.as_bytes(),
    });

    conn.check_request(req)?;
    Ok(())
}

pub fn get(property: &str) -> Result<String, xcb::Error> {
    let (conn, root, atom) = init(property);

    let req = conn.send_request(&x::GetProperty {
        delete: false,
        window: root,
        property: atom,
        r#type: x::ATOM_STRING,
        long_offset: 0,
        long_length: 1024,
    });

    let reply = conn.wait_for_reply(req)?;
    Ok(std::str::from_utf8(reply.value())
        .expect("Value in xprop")
        .to_string())
}
