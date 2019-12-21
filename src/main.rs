extern crate i3ipc;
extern crate xcb;

use std::collections::VecDeque;
use std::env;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

use i3ipc::event::inner::WindowChange;
use i3ipc::event::Event;
use i3ipc::{I3Connection, I3EventListener, Subscription};

mod xprop;

static BUFFER_SIZE: usize = 100;

const SOCKET_PATH_PROP: &str = "_I3_ALTERNATE_FOCUS_SOCKET";

const SWITCH_COMMAND: &[u8] = b"switch";

fn focus_nth(windows: &VecDeque<i64>, n: usize) -> Result<(), Box<dyn Error>> {
    let mut conn = I3Connection::connect().unwrap();
    let mut k = n;

    // Start from the nth window and try to change focus until it succeeds
    // (so that it skips windows which no longer exist)
    while let Some(wid) = windows.get(k) {
        let r = conn.run_command(format!("[con_id={}] focus", wid).as_str())?;

        if let Some(o) = r.outcomes.get(0) {
            if o.success {
                return Ok(());
            }
        }

        k += 1;
    }

    Err(From::from(format!("Last {}nth window unavailable", n)))
}

fn cmd_server(windows: Arc<Mutex<VecDeque<i64>>>) {
    let socket = {
        let mut base = match env::var("XDG_RUNTIME_DIR") {
            Ok(path) => PathBuf::from(path),
            Err(_) => PathBuf::from("/tmp"),
        };

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let name = format!(
            "i3-alternate-focus.{}.{}.sock",
            std::process::id(),
            timestamp
        );

        base.push(&name);
        base
    };

    xprop::set(SOCKET_PATH_PROP, &socket.to_string_lossy()).expect("Set xprop");

    // Listen to client commands
    let listener = UnixListener::bind(socket).unwrap();

    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            let windows = windows.clone();

            thread::spawn(move || {
                for line in BufReader::new(stream.try_clone().unwrap()).lines() {
                    match line {
                        Ok(line) if line.as_bytes() == SWITCH_COMMAND => {
                            let winc = windows.lock().unwrap();
                            let _ = focus_nth(&winc, 1);
                            let _ = stream.write_all(b"Done\n");
                        }
                        _ => {
                            let _ = stream.write_all(b"Invalid command\n");
                        }
                    }
                }
            });
        }
    }
}

fn get_focused_window() -> Result<i64, ()> {
    let mut conn = I3Connection::connect().unwrap();
    let mut node = conn.get_tree().unwrap();

    while !node.focused {
        let fid = node.focus.into_iter().nth(0).ok_or(())?;
        node = node
            .nodes
            .into_iter()
            .filter(|n| n.id == fid)
            .nth(0)
            .ok_or(())?;
    }

    Ok(node.id)
}

fn focus_server() {
    let mut listener = I3EventListener::connect().unwrap();
    let windows = Arc::new(Mutex::new(VecDeque::new()));
    let windowsc = Arc::clone(&windows);

    // Add the current focused window to bootstrap the list
    get_focused_window()
        .map(|wid| {
            let mut windows = windows.lock().unwrap();

            windows.push_front(wid);
        })
        .ok();

    thread::spawn(|| cmd_server(windowsc));

    // Listens to i3 event
    let subs = [Subscription::Window];
    listener.subscribe(&subs).unwrap();

    for event in listener.listen() {
        match event.unwrap() {
            Event::WindowEvent(e) => {
                if let WindowChange::Focus = e.change {
                    let mut windows = windows.lock().unwrap();

                    // dedupe, push front and truncate
                    windows.retain(|v| *v != e.container.id);
                    windows.push_front(e.container.id);
                    windows.truncate(BUFFER_SIZE);
                }
            }
            _ => unreachable!(),
        }
    }
}

fn focus_client() {
    let socket_path = xprop::get(SOCKET_PATH_PROP).expect("Get xprop");
    let mut stream = UnixStream::connect(socket_path).unwrap();

    stream.write_all(SWITCH_COMMAND).expect("Write to socket");
}

fn main() {
    match env::args().skip(1).next() {
        Some(arg) if arg == "server" => {
            focus_server();
        }
        Some(arg) if arg == "switch" => {
            focus_client();
        }
        _ => {
            eprintln!("Expected argument: server or switch");
        }
    }
}
