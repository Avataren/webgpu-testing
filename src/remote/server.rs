//! Loopback TCP listener that forwards requests to the main thread.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::hub::{RemoteRequest, Reply};

/// How long a connection waits for the main thread to answer one request. Long enough
/// for multi-second waits and slow first frames, short enough that a hung engine is
/// reported instead of blocking the client forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(300);

/// Start listening on `127.0.0.1:port`. Requests arrive on the returned receiver.
pub(super) fn start(port: u16) -> std::io::Result<Receiver<RemoteRequest>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let (sender, receiver) = mpsc::channel();

    thread::Builder::new()
        .name("remote-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let sender = sender.clone();
                        let spawned = thread::Builder::new()
                            .name("remote-connection".into())
                            .spawn(move || serve_connection(stream, sender));
                        if let Err(err) = spawned {
                            log::warn!(target: "remote", "Failed to spawn connection thread: {err}");
                        }
                    }
                    Err(err) => log::warn!(target: "remote", "Failed to accept connection: {err}"),
                }
            }
        })?;

    Ok(receiver)
}

fn serve_connection(stream: TcpStream, requests: Sender<RemoteRequest>) {
    let peer = stream
        .peer_addr()
        .map_or_else(|_| "unknown".to_string(), |addr| addr.to_string());
    log::debug!(target: "remote", "Remote client connected from {peer}");

    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(err) => {
            log::warn!(target: "remote", "Failed to clone remote stream: {err}");
            return;
        }
    };
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = handle_line(&line, &requests);
        let mut encoded = response.to_string();
        encoded.push('\n');
        if writer.write_all(encoded.as_bytes()).is_err() || writer.flush().is_err() {
            break;
        }
    }

    log::debug!(target: "remote", "Remote client {peer} disconnected");
}

fn handle_line(line: &str, requests: &Sender<RemoteRequest>) -> Value {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => return error_response(Value::Null, format!("invalid JSON: {err}")),
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return error_response(id, "missing \"method\"".into());
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let (reply_sender, reply_receiver) = mpsc::channel();
    let request = RemoteRequest {
        method: method.to_owned(),
        params,
        reply: Reply::new(reply_sender),
    };
    if requests.send(request).is_err() {
        return error_response(id, "engine is shutting down".into());
    }

    match reply_receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(result)) => json!({ "id": id, "result": result }),
        Ok(Err(message)) => error_response(id, message),
        Err(mpsc::RecvTimeoutError::Timeout) => error_response(
            id,
            format!(
                "timed out after {}s waiting for the engine; is the window minimized or the main thread blocked?",
                REPLY_TIMEOUT.as_secs()
            ),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            error_response(id, "request dropped without a reply".into())
        }
    }
}

fn error_response(id: Value, message: String) -> Value {
    json!({ "id": id, "error": { "message": message } })
}
