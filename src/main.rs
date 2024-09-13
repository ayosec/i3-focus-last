use std::process::ExitCode;

use tokio::task;
use xcb::x;

mod x11;

enum Command {
    Server,
    Switch,
}

async fn switch_handler(display: x11::DisplayServer) {
    // https://specifications.freedesktop.org/wm-spec/1.5/ar01s09.html#sourceindication
    const SOURCE_PAGER: u32 = 2;

    loop {
        display.switch_command().notified().await;

        if let Some(window) = display.switch_window() {
            let root = display.roots()[0];

            let event = x::ClientMessageEvent::new(
                window,
                display.atoms().net_active_window,
                x::ClientMessageData::Data32([SOURCE_PAGER, 0, 0, 0, 0]),
            );

            let req = x::SendEvent {
                propagate: false,
                destination: x::SendEventDest::Window(root),
                event_mask: x::EventMask::SUBSTRUCTURE_NOTIFY | x::EventMask::SUBSTRUCTURE_REDIRECT,
                event: &event,
            };

            let _ = display.connection().send_and_check_request(&req);
        };
    }
}

async fn run_server(display: x11::DisplayServer) -> Result<(), xcb::Error> {
    tokio::task::spawn_local(switch_handler(display.clone()));

    display.main_loop().await
}

async fn run_switch(display: x11::DisplayServer) -> Result<(), xcb::Error> {
    let root = display.roots()[0];

    let event = x::ClientMessageEvent::new(
        root,
        display.atoms().switch_command,
        x::ClientMessageData::Data32(Default::default()),
    );

    let req = x::SendEvent {
        propagate: false,
        destination: x::SendEventDest::Window(root),
        event_mask: x::EventMask::STRUCTURE_NOTIFY,
        event: &event,
    };

    Ok(display.connection().send_and_check_request(&req)?)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Parse CLI arguments.
    let mut args = std::env::args();
    let program_name = args.next();

    let command = match (args.next().as_deref(), args.next()) {
        (Some("server"), None) => Command::Server,
        (Some("switch"), None) => Command::Switch,
        _ => {
            eprintln!("Usage: {} server|switch", program_name.unwrap_or_default());
            return ExitCode::FAILURE;
        }
    };

    // Connect to X11.
    let conn = match x11::DisplayServer::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Can't connect to X11: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Execute command from arguments.
    let local = task::LocalSet::new();

    let task = async move {
        match command {
            Command::Server => run_server(conn).await,
            Command::Switch => run_switch(conn).await,
        }
    };

    if let Err(e) = local.run_until(task).await {
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
