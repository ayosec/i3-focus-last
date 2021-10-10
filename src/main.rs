use std::collections::VecDeque;
use std::env;
use std::error::Error;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use i3ipc::event::inner::WindowChange;
use i3ipc::event::Event;
use i3ipc::{I3Connection, I3EventListener, Subscription};

mod xprop;

/// Min. time with focus required to keep a window in the queue.
const MIN_FOCUS: Duration = Duration::from_secs(1);

static BUFFER_SIZE: usize = 64;

const SOCKET_PATH_PROP: &str = "_I3_ALTERNATE_FOCUS_SOCKET";

const SWITCH_COMMAND: &[u8] = b"switch";
const DEBUG_COMMAND: &[u8] = b"debug";

#[derive(Default, Debug)]
struct State {
    windows: VecDeque<i64>,

    focus_after_switch: bool,
}

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

fn cmd_server(state: Arc<Mutex<State>>) {
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

    for mut stream in listener.incoming().flatten() {
        let state = state.clone();

        thread::spawn(move || {
            let mut reader = BufReader::new(stream.try_clone().unwrap()).lines();
            let line = reader.next();
            match line {
                Some(Ok(line)) if line.as_bytes() == SWITCH_COMMAND => {
                    let mut state = state.lock().unwrap();
                    state.focus_after_switch = true;
                    let _ = focus_nth(&state.windows, 1);
                }

                Some(Ok(line)) if line.as_bytes() == DEBUG_COMMAND => {
                    let state = state.lock().unwrap();
                    let _ = writeln!(&mut stream, "{:#?}", state);
                }

                _ => {
                    let _ = stream.write_all(b"Invalid command\n");
                }
            }
        });
    }
}

fn get_focused_window() -> Result<i64, ()> {
    let mut conn = I3Connection::connect().unwrap();
    let mut node = conn.get_tree().unwrap();

    while !node.focused {
        let fid = node.focus.into_iter().next().ok_or(())?;
        node = node.nodes.into_iter().find(|n| n.id == fid).ok_or(())?;
    }

    Ok(node.id)
}

fn focus_server() {
    let mut listener = I3EventListener::connect().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let state2 = Arc::clone(&state);

    // Add the current focused window to bootstrap the list
    get_focused_window()
        .map(|wid| {
            let mut state = state.lock().unwrap();
            state.windows.push_front(wid);
        })
        .ok();

    thread::spawn(|| cmd_server(state2));

    // Listens to i3 event
    let subs = [Subscription::Window];
    listener.subscribe(&subs).unwrap();

    let mut last_change = Instant::now();

    for event in listener.listen() {
        if let Event::WindowEvent(e) = event.unwrap() {
            match e.change {
                WindowChange::Focus => {
                    let mut state = state.lock().unwrap();

                    // Ignore last window if it had focus for less than
                    // MIN_FOCUS
                    if !std::mem::take(&mut state.focus_after_switch)
                        && last_change.elapsed() < MIN_FOCUS
                    {
                        state.windows.pop_front();
                    }

                    last_change = Instant::now();

                    let cid = e.container.id;
                    if state.windows.front().copied() != Some(cid) {
                        state.windows.push_front(cid);
                        state.windows.truncate(BUFFER_SIZE);
                    }
                }

                WindowChange::Close => {
                    state
                        .lock()
                        .unwrap()
                        .windows
                        .retain(|v| *v != e.container.id);
                }

                _ => (),
            }
        }
    }
}

fn focus_client(command: &str) {
    let socket_path = xprop::get(SOCKET_PATH_PROP).expect("Get xprop");
    let mut stream = UnixStream::connect(socket_path).unwrap();

    writeln!(&mut stream, "{}", command).expect("Write to socket");
    io::copy(&mut stream, &mut io::stdout().lock()).expect("Copy server output");
}

fn main() {
    match env::args().nth(1) {
        Some(arg) if arg == "server" => {
            focus_server();
        }
        Some(arg) => {
            focus_client(&arg);
        }
        _ => {
            eprintln!("Expected argument: server, switch, debug");
        }
    }
}
