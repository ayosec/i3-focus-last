use xcb::ffi::base::xcb_generic_error_t;

pub fn init(property: &str) -> (xcb::base::Connection, u32, u32) {
    let (conn, screen_num) = xcb::Connection::connect(None).unwrap();
    let setup = conn.get_setup();
    let screen = setup.roots().nth(screen_num as usize).unwrap();
    let root = screen.root();

    let atom = xcb::xproto::intern_atom(&conn, false, property)
        .get_reply()
        .unwrap()
        .atom();

    (conn, root, atom)
}

pub fn set(property: &str, value: &str) -> Result<(), xcb::base::Error<xcb_generic_error_t>> {
    let (conn, root, atom) = init(property);

    xcb::change_property(
        &conn,
        xcb::PROP_MODE_REPLACE as u8,
        root,
        atom,
        xcb::ATOM_STRING,
        8,
        value.as_bytes(),
    )
    .request_check()?;

    Ok(())
}

pub fn get(property: &str) -> Result<String, xcb::base::Error<xcb_generic_error_t>> {
    let (conn, root, atom) = init(property);

    let reply =
        xcb::get_property(&conn, false, root, atom, xcb::ATOM_STRING, 0, 1024).get_reply()?;

    Ok(std::str::from_utf8(reply.value::<u8>())
        .expect("Value in xprop")
        .to_string())
}
