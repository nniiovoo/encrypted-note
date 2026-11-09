//! `enote-agent-keys [--owner-uid <uid>] [--socket <path>]`: the stdio MCP server an agent's
//! tool launches. See the library for what it guarantees.
//!
//! `--owner-uid` (or `ENOTE_OWNER_UID`) is the owner's uid: the app must run as it. `--socket`
//! is for trying a debug build of the app with `ENOTE_DEV_AGENT_SOCKET_DIR`; the uid check
//! applies there too.

use std::io::{BufRead, BufReader, Read, Write};

use agent_keys_mcp::{Config, MAX_LINE_BYTES, Server};

fn main() {
    let mut owner_uid = std::env::var("ENOTE_OWNER_UID")
        .ok()
        .and_then(|uid| uid.trim().parse::<u32>().ok());
    let mut config = Config::new(None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--owner-uid", Some(uid)) => match uid.trim().parse() {
                Ok(uid) => owner_uid = Some(uid),
                Err(_) => usage(),
            },
            ("--socket", Some(path)) => config.socket = path.into(),
            _ => usage(),
        }
    }
    config.owner_uid = owner_uid;
    serve(config);
}

fn usage() -> ! {
    eprintln!("usage: enote-agent-keys [--owner-uid <uid>] [--socket <path>]");
    std::process::exit(2);
}

#[cfg(unix)]
fn serve(config: Config) {
    run(Server::new(agent_keys_mcp::SocketRoute(config)));
}

#[cfg(not(unix))]
fn serve(_config: Config) {
    struct Unavailable;
    impl agent_keys_mcp::Route for Unavailable {
        fn send(
            &self,
            _request: &serde_json::Value,
            _wait_for_owner: bool,
        ) -> Result<serde_json::Value, agent_keys_mcp::RouteFailure> {
            Err(agent_keys_mcp::RouteFailure::NotRunning)
        }
    }
    run(Server::new(Unavailable));
}

/// At most this many `tools/call`s run at once on their own threads; past that, the next one is
/// answered on the reading thread (the app takes only a few connections at a time anyway).
const MAX_CALLS_IN_FLIGHT: usize = 4;

/// One whole line to stdout, from whichever thread. False if stdout is gone.
fn write_line(line: &str) -> bool {
    let mut output = std::io::stdout().lock();
    writeln!(output, "{line}").is_ok() && output.flush().is_ok()
}

/// A progress notification from a waiting call. Lost if stdout is gone, as the reply would be.
fn notify(line: String) {
    write_line(&line);
}

/// One message per line until stdin ends. A line over the limit, or one that isn't UTF-8, gets a
/// parse error (a long one is skipped to its end). Each `tools/call` is answered on its own
/// thread, so a call waiting for the owner doesn't hold up `ping` or `list_keys`; at the end of
/// input, the calls still running finish and are answered before the tool exits.
fn run<R: agent_keys_mcp::Route + Sync>(server: Server<R>) {
    let in_flight = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| read_loop(&server, &in_flight, scope));
}

fn read_loop<'scope, R: agent_keys_mcp::Route + Sync>(
    server: &'scope Server<R>,
    in_flight: &'scope std::sync::atomic::AtomicUsize,
    scope: &'scope std::thread::Scope<'scope, '_>,
) {
    use std::sync::atomic::Ordering;
    let mut input = BufReader::new(std::io::stdin().lock());
    loop {
        let mut line = Vec::new();
        match (&mut input)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let reply = if line.len() > MAX_LINE_BYTES && line.last() != Some(&b'\n') {
            if !skip_line(&mut input) {
                return;
            }
            Some(agent_keys_mcp::parse_error())
        } else {
            match std::str::from_utf8(&line) {
                Ok(text) if text.trim().is_empty() => continue,
                Ok(text) => {
                    let text = text.trim_end();
                    // Only this thread adds to the count, so the limit holds.
                    if agent_keys_mcp::is_tool_call(text)
                        && in_flight.load(Ordering::SeqCst) < MAX_CALLS_IN_FLIGHT
                    {
                        in_flight.fetch_add(1, Ordering::SeqCst);
                        let owned = text.to_owned();
                        let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                            if let Some(reply) = server.handle_line_notifying(&owned, &notify) {
                                write_line(&reply);
                            }
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                        });
                        if spawned.is_ok() {
                            continue;
                        }
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                    }
                    server.handle_line_notifying(text, &notify)
                }
                Err(_) => Some(agent_keys_mcp::parse_error()),
            }
        };
        if let Some(reply) = reply
            && !write_line(&reply)
        {
            return;
        }
    }
}

/// Reads past the next newline without keeping what it reads. False at the end of input.
fn skip_line(input: &mut impl BufRead) -> bool {
    loop {
        let (found, used) = match input.fill_buf() {
            Ok([]) | Err(_) => return false,
            Ok(buffer) => match buffer.iter().position(|&b| b == b'\n') {
                Some(end) => (true, end + 1),
                None => (false, buffer.len()),
            },
        };
        input.consume(used);
        if found {
            return true;
        }
    }
}
