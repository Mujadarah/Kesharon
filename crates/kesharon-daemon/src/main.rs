use std::error::Error;
use std::io::{BufRead, Write};

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{LaunchToken, PROTOCOL_VERSION};

fn main() {
    if let Err(error) = run() {
        eprintln!("kesharon-daemon: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut endpoint = None;
    let mut once = false;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--endpoint" => {
                endpoint = arguments.next();
            }
            "--once" => once = true,
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }

    let endpoint = LocalEndpoint::new(endpoint.ok_or("missing --endpoint")?)?;
    let mut token = String::new();
    std::io::stdin().lock().read_line(&mut token)?;
    let token = token.trim_end_matches(['\r', '\n']);
    let launch_token = LaunchToken::parse_hex(token)?;
    let server = LocalServer::bind(&endpoint)?;
    let daemon = Daemon::new(launch_token);

    println!("READY {PROTOCOL_VERSION}");
    std::io::stdout().flush()?;

    if once {
        daemon.serve_local_connection(&server)?;
        return Ok(());
    }

    ServerRuntime::new(daemon).run(&server)?;
    Ok(())
}
