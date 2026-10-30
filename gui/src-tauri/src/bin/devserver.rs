//! The editor's backend over HTTP, for driving the GUI in a browser (tests,
//! screenshots): `POST /invoke/<command>` with the command's arguments as
//! JSON, as the desktop app passes them to Tauri. Not part of the app.
//!
//!   cargo run -p docex-gui --bin devserver -- [PORT]     (default 1431)
//!
//! Run it with `XDG_DATA_HOME` pointing somewhere temporary so "since you
//! last saw it" snapshots don't mix with real ones.

#[allow(dead_code)]
#[path = "../backend.rs"]
mod backend;
#[path = "../recovery.rs"]
mod recovery;
#[path = "../snapshot.rs"]
mod snapshot;

use backend::AppState;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null)).map_err(|e| format!("argument {name}: {e}"))
}

fn to<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn dispatch(st: &AppState, cmd: &str, a: &Value) -> Result<Value, String> {
    match cmd {
        "open_doc" => to(backend::open_doc(st, arg(a, "path")?)?),
        "new_doc" => to(backend::new_doc(st)?),
        "initial_path" => Ok(Value::Null),
        "sync" => to(backend::sync(st, arg(a, "model")?, arg(a, "tracked")?)?),
        "resolve" => to(backend::resolve(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "wids")?, arg(a, "accept")?)?),
        "resolve_batch" => to(backend::resolve_batch(
            st,
            arg(a, "model")?,
            arg(a, "tracked")?,
            arg(a, "accept")?,
            arg(a, "reject")?,
            arg(a, "done_comments")?,
        )?),
        "resolve_all" => to(backend::resolve_all(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "accept")?)?),
        "undo_review" => to(backend::undo_review(st, arg(a, "model")?, arg(a, "tracked")?)?),
        "comment_reply" => to(backend::comment_reply(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "id")?, arg(a, "text")?)?),
        "comment_done" => to(backend::comment_done(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "id")?, arg(a, "done")?)?),
        "comment_delete" => to(backend::comment_delete(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "id")?)?),
        "set_track" => to(backend::set_track(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "on")?)?),
        "save" => to(backend::save(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "path")?)?),
        "theme" => to(backend::theme()),
        "image_data" => to(backend::image_data(arg(a, "path")?)?),
        "snapshot_swap" => to(backend::snapshot_swap(arg(a, "path")?, arg(a, "paras")?)),
        "snapshot_put" => to(backend::snapshot_put(arg(a, "path")?, arg(a, "paras")?)?),
        "md_to_model" => to(backend::md_to_model(arg(a, "md")?)),
        "recovery_put" => to(backend::recovery_put(st, arg(a, "model")?, arg(a, "tracked")?)?),
        "recovery_check" => to(backend::recovery_check(arg(a, "path")?)),
        "recovery_open" => to(backend::recovery_open(st, arg(a, "path")?)?),
        "recovery_clear" => to(backend::recovery_clear(arg(a, "path")?)),
        // The browser checks spelling itself.
        "spellcheck" => Ok(arg::<Value>(a, "langs")?),
        "get_author" => to(backend::get_author(st)),
        "set_author" => to(backend::set_author(st, arg(a, "name")?)),
        "ai_source" => to(backend::ai_source(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "marked")?)?),
        "ai_apply" => to(backend::ai_apply(st, arg(a, "model")?, arg(a, "tracked")?, arg(a, "md")?)?),
        // Tests script the AI's answer: the file named by DOCEX_FAKE_AI.
        "ai_complete" => match std::env::var("DOCEX_FAKE_AI") {
            Ok(p) => std::fs::read_to_string(p).map(Value::String).map_err(|e| e.to_string()),
            Err(_) => Err("no AI in the dev server".into()),
        },
        // A newer version: whatever a test writes to the file named by DOCEX_FAKE_UPDATE.
        "check_update" => match std::env::var("DOCEX_FAKE_UPDATE").ok().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(v) => serde_json::from_str(&v).map_err(|e| e.to_string()),
            None => Ok(Value::Null),
        },
        "open_release" => Ok(Value::Null),
        // No AI in tests: nothing is configured.
        "ai_settings" => Ok(json!({ "provider": "anthropic", "anthropic_model": "", "effort": "", "openai_base_url": "", "openai_model": "", "anthropic_key_set": false, "anthropic_key_from_env": false, "openai_key_set": false })),
        _ => Err(format!("{cmd}: not available in the dev server")),
    }
}

fn handle(st: &AppState, mut stream: TcpStream) -> std::io::Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut len = 0;
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    let (status, out) = match path.strip_prefix("/invoke/") {
        Some(cmd) => {
            let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            match dispatch(st, cmd, &args) {
                Ok(v) => ("200 OK", v),
                Err(e) => ("500 Internal Server Error", Value::String(e)),
            }
        }
        None => ("404 Not Found", Value::String("POST /invoke/<command>".into())),
    };
    let body = out.to_string();
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "1431".into());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("port in use");
    eprintln!("docex dev server on http://127.0.0.1:{port}");
    let st = AppState::default();
    for s in listener.incoming().flatten() {
        if let Err(e) = handle(&st, s) {
            eprintln!("request failed: {e}");
        }
    }
}
