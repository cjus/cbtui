//! The engine, as a Chess Node Protocol node.
//!
//! cbtui starts the engine as a child process (`cb2026 --cnp` by default) and talks to it in
//! [CNP](https://github.com/cjus/chess-node-protocol): JSON-RPC 2.0, one message per line on
//! the child's stdin and stdout. Any engine that speaks CNP works; nothing here is specific to
//! cb2026.
//!
//! ## How it works
//!
//! - **The handshake is synchronous.** [`Node::connect`] sends `initialize` and waits for its
//!   answer before anything else, so the rest of the program always knows the node's name,
//!   capabilities and limits ([`Node::info`]).
//! - **Then a thread reads.** Every later message (search progress, responses) is passed to
//!   the `on_event` callback as a [`NodeEvent`], from a reader thread, so the user interface
//!   never waits on the engine. Requests are written straight away and return their id;
//!   the answer arrives later as [`NodeEvent::Response`] with that id.
//! - **The engine's stderr is passed on**, line by line, as [`NodeEvent::Stderr`]. Left alone
//!   it would write over the full-screen display.
//! - **Shutting down is polite, then firm.** [`Node::shutdown`] sends `shutdown` and `exit`
//!   (spec 5.3), waits a moment for the child to leave, and kills it if it doesn't.
//!
//! ## Pitfalls
//!
//! - A node answers only one search at a time (`maxConcurrentSearches`, often 1). A search
//!   sent while another runs may be answered `Busy`; stop the old one first. The node takes a
//!   new search at once after a stop or cancel (spec 4.1), so a takeback can stop and search
//!   again straight away.
//! - Responses can cross requests: a search can answer just as it is stopped. Match each
//!   response to the request that is still wanted, by id, and drop the rest.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cnp::engine::{
    EngineProgress, Evaluate, EvaluateParams, Levels, Progress, Root, RootParams, Search,
    SearchParams, Stop, StopParams, Techniques,
};
use cnp::lifecycle::{
    Exit, Initialize, InitializeParams, InitializeResult, PROTOCOL_VERSION, Shutdown, Software,
};
use cnp::{Message, NoParams, Notification, Request, Response, stdio};

/// The largest message cbtui reads before the node has said its own limit.
const HANDSHAKE_MAX_BYTES: usize = 1024 * 1024;

/// How long [`Node::shutdown`] waits for the engine to leave before killing it.
const EXIT_GRACE: Duration = Duration::from_millis(1500);

/// Something the node sent, or that happened to it.
#[derive(Debug)]
pub enum NodeEvent {
    /// `engine/progress`: a search's best line so far.
    Progress(Progress),
    /// `engine/root`: where a search is at its root.
    Root(RootParams),
    /// The answer to a request, matched to it by id.
    Response(Response),
    /// A line the engine wrote to its stderr.
    Stderr(String),
    /// The connection ended: the engine exited, or sent something that isn't CNP.
    Closed(String),
}

/// A connected node.
pub struct Node {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    next_id: i64,
    info: InitializeResult,
    child: Option<Child>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("node", &self.info.node.name)
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl Node {
    /// Starts `program` with `args` and connects to it.
    pub fn spawn(
        program: &str,
        args: &[String],
        on_event: impl Fn(NodeEvent) + Send + Sync + 'static,
    ) -> Result<Node, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("can't start the engine `{program}`: {e}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let on_event = Arc::new(on_event);
        let to_stderr = Arc::clone(&on_event);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                match line {
                    Ok(line) => to_stderr(NodeEvent::Stderr(line)),
                    Err(_) => break,
                }
            }
        });
        let mut node =
            Node::connect(BufReader::new(stdout), stdin, move |event| on_event(event))
                .map_err(|e| format!("the engine `{program}` didn't start a CNP session: {e}"))?;
        node.child = Some(child);
        Ok(node)
    }

    /// Connects to a node that reads `writer` and writes `reader`: sends `initialize`, waits
    /// for the answer, then reads everything else on a thread of its own.
    pub fn connect(
        mut reader: impl BufRead + Send + 'static,
        writer: impl Write + Send + 'static,
        on_event: impl Fn(NodeEvent) + Send + 'static,
    ) -> Result<Node, String> {
        let mut writer: Box<dyn Write + Send> = Box::new(writer);
        let params = InitializeParams {
            protocol: PROTOCOL_VERSION,
            client: Software {
                name: env!("CARGO_PKG_NAME").into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };
        stdio::write_message(&mut writer, &Request::new::<Initialize>(1, &params).into())
            .map_err(|e| e.to_string())?;
        let info = loop {
            match stdio::read_message(&mut reader, HANDSHAKE_MAX_BYTES) {
                Ok(Some(Message::Response(response))) if response.id == Some(1.into()) => {
                    break response
                        .result::<Initialize>()
                        .map_err(|e| format!("initialize failed: {e}"))?;
                }
                // Nothing else should come first; a node may log a notification, so skip it.
                Ok(Some(_)) => continue,
                Ok(None) => return Err("it closed its output before answering".into()),
                Err(e) => return Err(e.to_string()),
            }
        };
        let max_bytes = usize::try_from(info.limits.message_bytes()).unwrap_or(usize::MAX);
        thread::spawn(move || {
            loop {
                match stdio::read_message(&mut reader, max_bytes) {
                    Ok(Some(Message::Notification(note))) => {
                        if note.is::<EngineProgress>()
                            && let Ok(progress) = note.params::<EngineProgress>()
                        {
                            on_event(NodeEvent::Progress(progress));
                        } else if note.is::<Root>()
                            && let Ok(root) = note.params::<Root>()
                        {
                            on_event(NodeEvent::Root(root));
                        }
                        // Other notifications aren't asked for.
                    }
                    Ok(Some(Message::Response(response))) => {
                        on_event(NodeEvent::Response(response));
                    }
                    // A node doesn't send an engine client requests; ignore any.
                    Ok(Some(Message::Request(_))) => {}
                    Ok(None) => {
                        on_event(NodeEvent::Closed("the engine closed the connection".into()));
                        break;
                    }
                    Err(e) => {
                        if e.response().is_some() {
                            // A malformed line: the reader is at the next one, so carry on.
                            continue;
                        }
                        on_event(NodeEvent::Closed(format!("reading from the engine: {e}")));
                        break;
                    }
                }
            }
        });
        Ok(Node {
            writer: Arc::new(Mutex::new(writer)),
            next_id: 2,
            info,
            child: None,
        })
    }

    /// What the node said about itself in `initialize`: name, version, licence,
    /// capabilities and limits.
    pub fn info(&self) -> &InitializeResult {
        &self.info
    }

    fn send(&self, message: Message) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .expect("a writer never panics while held");
        stdio::write_message(&mut *writer, &message)
    }

    fn request<M: cnp::RequestMethod>(&mut self, params: &M::Params) -> Result<i64, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(Request::new::<M>(id, params).into())
            .map_err(|e| format!("writing to the engine: {e}"))?;
        Ok(id)
    }

    /// Sends `engine/levels`, returning the request's id.
    pub fn levels(&mut self) -> Result<i64, String> {
        self.request::<Levels>(&NoParams)
    }

    /// Sends `engine/techniques`, returning the request's id.
    pub fn techniques(&mut self) -> Result<i64, String> {
        self.request::<Techniques>(&NoParams)
    }

    /// Stops the engine and waits briefly for it to exit, killing it if it doesn't.
    pub fn shutdown(mut self) {
        let _ = self.request::<Shutdown>(&NoParams);
        let _ = self.send(Notification::new::<Exit>(&NoParams).into());
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + EXIT_GRACE;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// What the application asks of an engine. [`Node`] is the real one; tests use a fake.
pub trait Engine {
    /// Sends `engine/search`, returning the request's id.
    fn search(&mut self, params: &SearchParams) -> Result<i64, String>;
    /// Sends `engine/stop` for the search with this id.
    fn stop(&mut self, id: i64) -> Result<(), String>;
    /// Sends `engine/evaluate`, returning the request's id.
    fn evaluate(&mut self, params: &EvaluateParams) -> Result<i64, String>;
}

impl Engine for Node {
    fn search(&mut self, params: &SearchParams) -> Result<i64, String> {
        self.request::<Search>(params)
    }

    fn stop(&mut self, id: i64) -> Result<(), String> {
        self.request::<Stop>(&StopParams { id: id.into() })
            .map(|_| ())
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<i64, String> {
        self.request::<Evaluate>(params)
    }
}

/// The engine when there is none, as for a PGN viewer started without one: a search fails,
/// saying so, and a stop has nothing to stop.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoEngine;

impl Engine for NoEngine {
    fn search(&mut self, _params: &SearchParams) -> Result<i64, String> {
        Err("No engine is connected.".into())
    }

    fn stop(&mut self, _id: i64) -> Result<(), String> {
        Ok(())
    }

    fn evaluate(&mut self, _params: &EvaluateParams) -> Result<i64, String> {
        Err("No engine is connected.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cnp::engine::{Score, SearchLimits, SearchResult};
    use cnp::lifecycle::{Capabilities, Limits, NodeInfo};
    use std::sync::mpsc;

    /// A tiny CNP node on the other end of two pipes: answers `initialize`, and answers each
    /// search with one progress notification and the result `e7e5`.
    fn fake_node(reader: io::PipeReader, mut writer: io::PipeWriter) {
        let mut reader = BufReader::new(reader);
        while let Ok(Some(message)) = stdio::read_message(&mut reader, 1 << 20) {
            let Message::Request(request) = message else {
                continue;
            };
            let response = if request.is::<Initialize>() {
                Response::ok::<Initialize>(
                    request.id,
                    &InitializeResult {
                        protocol: PROTOCOL_VERSION,
                        node: NodeInfo {
                            name: "fake".into(),
                            version: "1.0".into(),
                            author: None,
                            license: "MIT".into(),
                            source: None,
                        },
                        capabilities: Capabilities::default(),
                        limits: Limits::default(),
                        resources: None,
                    },
                )
            } else if request.is::<Search>() {
                let progress = Progress {
                    id: request.id.clone(),
                    depth: 1,
                    sel_depth: 1,
                    score: Score::cp(20),
                    pv: vec!["e7e5".into()],
                    multipv: None,
                    nodes: 10,
                    nps: None,
                    time_ms: 1,
                    hash_full: None,
                    tb_hits: None,
                };
                stdio::write_message(
                    &mut writer,
                    &Notification::new::<EngineProgress>(&progress).into(),
                )
                .unwrap();
                Response::ok::<Search>(
                    request.id,
                    &serde_json::from_value::<SearchResult>(serde_json::json!({
                        "bestMove": "e7e5", "pv": ["e7e5"], "depth": 1, "selDepth": 1,
                        "nodes": 10, "timeMs": 1, "stopped": false
                    }))
                    .unwrap(),
                )
            } else {
                continue;
            };
            stdio::write_message(&mut writer, &response.into()).unwrap();
        }
    }

    #[test]
    fn connects_searches_and_hears_progress() {
        let (to_node_r, to_node_w) = io::pipe().unwrap();
        let (from_node_r, from_node_w) = io::pipe().unwrap();
        thread::spawn(move || fake_node(to_node_r, from_node_w));
        let (tx, rx) = mpsc::channel();
        let mut node = Node::connect(BufReader::new(from_node_r), to_node_w, move |event| {
            let _ = tx.send(event);
        })
        .unwrap();
        assert_eq!(node.info().node.name, "fake");

        let params = SearchParams::new(cnp::engine::Position::start().with_moves(["e2e4"]))
            .limits(SearchLimits::depth(1));
        let id = node.search(&params).unwrap();
        let Ok(NodeEvent::Progress(progress)) = rx.recv() else {
            panic!("expected progress first");
        };
        assert_eq!(progress.id, id.into());
        let Ok(NodeEvent::Response(response)) = rx.recv() else {
            panic!("expected the search's response");
        };
        assert_eq!(response.id, Some(id.into()));
        let result = response.result::<Search>().unwrap();
        assert_eq!(result.best_move.as_deref(), Some("e7e5"));
    }

    #[test]
    fn a_node_that_closes_early_is_an_error() {
        let (_r, w) = io::pipe().unwrap();
        let (r2, w2) = io::pipe().unwrap();
        drop(w2);
        let err = Node::connect(BufReader::new(r2), w, |_| {}).unwrap_err();
        assert!(err.contains("closed"), "{err}");
    }
}
