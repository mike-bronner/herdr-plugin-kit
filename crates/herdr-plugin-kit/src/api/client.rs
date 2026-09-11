//! The socket client, and the protocol handshake.
//!
//! Hand-written, and deliberately in its own file. A regeneration rewrites
//! `generated.rs` whole, and nothing it does may threaten this. See
//! SCOPE.md §4.2.
//!
//! # Why `interprocess` rather than `UnixStream`
//!
//! 🚨 **`HERDR_SOCKET_PATH` is a Unix socket on Unix and a named pipe on
//! Windows.** All three donor plugins reach for
//! `std::os::unix::net::UnixStream`, so all three are Unix-only, and none of
//! them could be promoted as written. `interprocess` puts one API over both,
//! and it is the same crate Herdr itself depends on.
//!
//! ⚠️ The Windows half is compile-verified and nothing more. Nobody on this
//! project has Windows hardware. See the README.
//!
//! # Do not assume the socket is named
//!
//! ✅ **Measured 2026-09-11 on Herdr 0.9.0, protocol 22, macOS.** A
//! `[[build]]` hook during `herdr plugin install` receives **zero** `HERDR_*`
//! variables. The install was run twice, the second time with
//! `HERDR_SOCKET_PATH` set explicitly on the invoking CLI, and the hook's
//! environment was identical both times. `GIT_CONFIG_*` passed through
//! untouched, so Herdr strips its own variables rather than losing them to
//! inheritance. A plugin event hook receives pane and workspace variables but
//! not this one. A `[[startup]]` hook and a pane-hosted command both receive
//! full environments.
//!
//! So the fallback is an ordinary path rather than a rare branch. [`Socket`]
//! records which of the two answered, and [`CallError::Connect`] says so,
//! because "cannot reach this path" and "cannot reach this path, and nothing
//! named it" are different problems.
//!
//! # Error codes live next door, and there is only one
//!
//! SCOPE.md §4.2.1 requires any error-code match to be hand-maintained beside
//! the transport, carrying the measurement that put it there. ✅ The kit has
//! exactly one such code, `dialog::BUSY_CODE`, and it already carries
//! its measurement. [`Client`]'s `dialog::Transport` implementation
//! routes through `dialog::OpenError::from_error` rather than
//! matching the string a second time. A second list here would be a structure
//! pretending to be a policy.
//!
//! Everywhere else the code and message are handed back verbatim in
//! [`CallError::Server`], for the caller to read or ignore.

use std::fmt;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericFilePath, Stream, ToFsName as _};
use serde::Deserialize;
use serde_json::Value;

use crate::api::generated::{
    ErrorBody, PingParams, RequestMethod, ResponseResult, ServerCapabilities, GENERATED_PROTOCOL,
};
use crate::api::Request;
use crate::env::{Environment, SOCKET_PATH_VAR};

#[cfg(feature = "dialog")]
use crate::api::generated::{NotificationShowParams, NotificationShowReason, PluginPaneOpenParams};
#[cfg(feature = "dialog")]
use crate::dialog::{OpenError, Transport};

/// The documented default socket, relative to the user's home directory.
///
/// 🚨 **`None` off Unix, and that is a measurement rather than an oversight.**
/// `HERDR_SOCKET_PATH` names a named pipe on Windows, and `interprocess`
/// accepts only `\\.\pipe\…` there: a filesystem path is refused outright.
/// Nobody has measured what Herdr calls that pipe, so there is no default to
/// document and inventing one would fail with a message pointing at a path
/// Herdr never used. [`Socket::resolve`] answers [`NoSocket`] instead, naming
/// the one variable that fixes it.
pub const DEFAULT_SOCKET: Option<&str> = match cfg!(unix) {
    true => Some(".config/herdr/herdr.sock"),
    false => None,
};

/// How long a call waits to send, and then to be answered.
///
/// Five seconds each way, which is what recent-spaces uses today.
/// [`Client::with_timeout`] overrides it.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Makes every request id in this process unique.
///
/// Process-global rather than per-[`Client`], so two clients in one plugin
/// cannot mint the same id. `Relaxed` is enough: uniqueness needs the
/// increment to be atomic, and nothing here orders against other memory.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Where a [`Socket`]'s path came from.
///
/// Kept because the answer changes what a reader should do about a failure.
/// See the module documentation for the measurement that makes this worth a
/// type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `HERDR_SOCKET_PATH` named it.
    Variable,
    /// The variable was unset or empty, so [`DEFAULT_SOCKET`] answered.
    Default,
    /// A caller named it outright, through [`Socket::at`].
    Given,
}

/// A socket to call Herdr on, and the record of how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socket {
    path: PathBuf,
    origin: Origin,
}

impl Socket {
    /// Reads `HERDR_SOCKET_PATH`, falling back to [`DEFAULT_SOCKET`].
    ///
    /// ⚠️ **A variable set to the empty string counts as unset here.** That is
    /// a deliberate divergence from [`Environment::get`], which reports the
    /// two apart because its callers differ. An empty path cannot be
    /// connected to, so the only useful reading is the fallback.
    pub fn resolve(env: &Environment) -> Result<Socket, NoSocket> {
        Socket::resolve_with(env, DEFAULT_SOCKET)
    }

    /// Resolves against a default supplied by the caller.
    ///
    /// [`Socket::resolve`] passes [`DEFAULT_SOCKET`]. This takes it as an
    /// argument so that the `None` case is reachable from a test running on a
    /// platform that has a default. A branch no harness can reach is not
    /// covered, and the Windows arm is otherwise checked by the compiler
    /// alone.
    pub fn resolve_with(env: &Environment, default: Option<&str>) -> Result<Socket, NoSocket> {
        match env.get(SOCKET_PATH_VAR).filter(|value| !value.is_empty()) {
            Some(named) => Ok(Socket {
                path: PathBuf::from(named),
                origin: Origin::Variable,
            }),
            None => match default {
                Some(relative) => Ok(Socket {
                    path: env.home().join(relative),
                    origin: Origin::Default,
                }),
                None => Err(NoSocket),
            },
        }
    }

    /// Takes a caller's own path, resolving nothing.
    ///
    /// For a plugin with a `--socket` flag, and for a test with a server of
    /// its own.
    pub fn at(path: impl Into<PathBuf>) -> Socket {
        Socket {
            path: path.into(),
            origin: Origin::Given,
        }
    }

    /// The path itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How the path was chosen.
    pub fn origin(&self) -> Origin {
        self.origin
    }
}

impl fmt::Display for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.origin {
            Origin::Variable => write!(f, "{} (from {})", self.path.display(), SOCKET_PATH_VAR),
            Origin::Default => write!(
                f,
                "{} (the default, because {} was not set)",
                self.path.display(),
                SOCKET_PATH_VAR
            ),
            Origin::Given => write!(f, "{}", self.path.display()),
        }
    }
}

/// No socket could be named at all.
///
/// Raised only where [`DEFAULT_SOCKET`] is `None`, which today means off Unix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSocket;

impl fmt::Display for NoSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is not set, and this platform has no documented default",
            SOCKET_PATH_VAR
        )
    }
}

/// Why a call did not produce a result.
///
/// 🔑 **Four cases, because a caller that cannot tell them apart cannot react
/// differently to them.** A wedged server, an absent one, a server speaking a
/// shape the kit does not know, and a server refusing the request on purpose
/// all want different answers. SCOPE.md §4.2 names these four.
#[derive(Debug)]
pub enum CallError {
    /// The socket could not be opened, configured, or used.
    ///
    /// Carries the [`Socket`], so the message can say where the path came from.
    Connect {
        /// The socket the call tried.
        socket: Socket,
        /// What the operating system said.
        reason: String,
    },
    /// Nothing arrived inside the timeout.
    ///
    /// ⚠️ **Distinct from [`CallError::Connect`] on purpose.** A wedged server
    /// accepts the connection and then says nothing, so a caller retrying a
    /// connect failure would retry this one forever.
    Timeout {
        /// The method that went unanswered.
        method: String,
        /// The bound it passed.
        after: Duration,
    },
    /// Bytes came back, and they were not a well-formed answer to this call.
    ///
    /// An answer that is not JSON, an answer carrying neither a result nor an
    /// error, an answer to some other request, and a result whose `type` this
    /// build does not know all land here. The last of those is what
    /// [`Handshake::mismatch`] exists to explain.
    Protocol(String),
    /// Herdr answered, and the answer was a refusal.
    ///
    /// The code and message are verbatim. Nothing in the kit interprets them
    /// except `dialog::OpenError::from_error`, which owns the one
    /// measured code the kit matches.
    Server(ErrorBody),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Connect { socket, reason } => {
                write!(f, "cannot reach {}: {}", socket, reason)
            }
            CallError::Timeout { method, after } => write!(
                f,
                "{} was not answered within {} seconds",
                method,
                after.as_secs_f32()
            ),
            CallError::Protocol(detail) => write!(f, "{}", detail),
            CallError::Server(body) => write!(f, "{} ({})", body.message, body.code),
        }
    }
}

/// What `ping` answered.
///
/// ✅ SCOPE.md §4.3: `version` and `protocol` are required, `capabilities` is
/// optional.
#[derive(Debug, Clone)]
pub struct Handshake {
    /// The server's own version string, such as `0.9.0`.
    pub version: String,
    /// The wire protocol the server speaks.
    pub protocol: u32,
    /// What the server says it can do, when it says anything.
    pub capabilities: Option<ServerCapabilities>,
}

impl Handshake {
    /// Compares the live server against [`GENERATED_PROTOCOL`].
    ///
    /// 🔑 **A value, never a side effect.** The kit detects; the plugin
    /// decides. That is how [`crate::version`] reports, how §7.2 hands back a
    /// notification reason, and how `dialog` answers `Shown`. A
    /// client that printed to stderr by itself would be the first place the
    /// kit decided something on a plugin's behalf, and it would hide a socket
    /// round trip inside an unrelated call, which is the exact confusion §4.3
    /// exists to end.
    ///
    /// ⚠️ **A mismatch is a warning and never a failure.** A plugin that still
    /// works has to keep working, so nothing here refuses to run.
    pub fn mismatch(&self) -> Option<ProtocolMismatch> {
        match self.protocol == GENERATED_PROTOCOL {
            true => None,
            false => Some(ProtocolMismatch {
                server: self.protocol,
                generated: GENERATED_PROTOCOL,
            }),
        }
    }
}

/// The live server speaks a different protocol than the types were built from.
///
/// **No plugin can detect this today.** A wire-format change arrives as a
/// parse failure somewhere unrelated, and the person reading it has no way to
/// learn that the server simply moved.
///
/// [`fmt::Display`] writes the warning line, naming both numbers, so a plugin
/// gets the wording for free:
///
/// ```
/// use herdr_plugin_kit::api::client::{Handshake, ProtocolMismatch};
///
/// let mismatch = ProtocolMismatch { server: 23, generated: 22 };
/// let line = mismatch.to_string();
///
/// assert!(line.contains("23"));
/// assert!(line.contains("22"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMismatch {
    /// What the live server answered.
    pub server: u32,
    /// What this build's types were generated for.
    pub generated: u32,
}

impl fmt::Display for ProtocolMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "this plugin was built for Herdr protocol {}, and the server speaks protocol {}. \
             Calls may fail in ways that look unrelated to the protocol. \
             This is a warning, not a failure.",
            self.generated, self.server
        )
    }
}

/// A Herdr response, either half.
///
/// Hand-written because the generated layer has `SuccessResponse` and
/// `ErrorResponse` but no union of the two. An untagged enum over the pair
/// would report a malformed answer as "data did not match any variant",
/// which names neither problem.
///
/// ⚠️ **Unknown top-level keys are ignored on purpose.** A future Herdr adding
/// a field to the envelope must not break every plugin built before it. The
/// `result` payload is still closed: a `type` this build does not know fails
/// to deserialize, and becomes [`CallError::Protocol`] rather than silence.
#[derive(Debug, Deserialize)]
struct Answer {
    id: String,
    result: Option<ResponseResult>,
    error: Option<ErrorBody>,
}

/// Calls Herdr over its socket.
///
/// One connection per call, newline-delimited JSON, which is the wire protocol
/// as measured and as all three donor plugins use it.
///
/// The whole of a plugin's opening move, including the protocol warning:
///
/// ```no_run
/// use herdr_plugin_kit::api::client::{Client, Socket};
/// use herdr_plugin_kit::env::Environment;
///
/// let env = Environment::from_process();
/// let socket = Socket::resolve(&env).expect("no socket could be named");
/// let client = Client::new(socket, "mikebronner.my-plugin");
///
/// match client.ping() {
///     Ok(handshake) => {
///         if let Some(mismatch) = handshake.mismatch() {
///             eprintln!("warning: {}", mismatch);
///         }
///     }
///     Err(error) => eprintln!("herdr is not answering: {}", error),
/// }
/// ```
///
/// `no_run` because it compiles here and connects nowhere. Reaching a real
/// server is what the suite's scripted peer is for.
#[derive(Debug, Clone)]
pub struct Client {
    socket: Socket,
    plugin_id: String,
    timeout: Duration,
}

impl Client {
    /// Builds a client with the [`CALL_TIMEOUT`] default.
    ///
    /// `plugin_id` prefixes every request id, so a Herdr-side log names the
    /// plugin that asked.
    pub fn new(socket: Socket, plugin_id: &str) -> Client {
        Client {
            socket,
            plugin_id: plugin_id.to_string(),
            timeout: CALL_TIMEOUT,
        }
    }

    /// Replaces the read and write timeout.
    pub fn with_timeout(self, timeout: Duration) -> Client {
        Client { timeout, ..self }
    }

    /// The socket this client calls on.
    pub fn socket(&self) -> &Socket {
        &self.socket
    }

    /// Sends one request, and returns the result it was answered with.
    ///
    /// # The id is checked, and that is what the counter is for
    ///
    /// 🔑 **Without the check the counter is decoration.** One connection per
    /// call already correlates a request with its answer, so the id buys
    /// nothing until something verifies it. What it catches is an answer the
    /// server sent for some other reason: a queued event, a reply to an
    /// earlier request on a reused handle, or a server that lost track. Wire
    /// data is untrusted input, and an answer addressed to somebody else is
    /// worse than no answer, because it looks like one.
    ///
    /// A mismatch is [`CallError::Protocol`] and names both ids.
    pub fn call(&self, method: RequestMethod) -> Result<ResponseResult, CallError> {
        let id = format!(
            "{}-{}",
            self.plugin_id,
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let request = Request {
            id: id.clone(),
            method,
        };

        // Through a `Value` rather than straight to a string, because the
        // method name is needed for every message below and `RequestMethod`
        // carries it only as a serde tag.
        let wire = serde_json::to_value(&request).map_err(|error| {
            CallError::Protocol(format!("the request could not be serialised: {}", error))
        })?;
        let method = wire
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("the request")
            .to_string();

        let name = self
            .socket
            .path
            .as_path()
            .to_fs_name::<GenericFilePath>()
            .map_err(|error| self.unreachable(error.to_string()))?;
        let stream = Stream::connect(name).map_err(|error| self.unreachable(error.to_string()))?;

        // ⚠️ Not `let _ =`, which is what recent-spaces writes today. A
        // timeout that silently failed to apply leaves the call able to hang
        // forever against a wedged server, which is the one situation the
        // timeout exists for.
        stream
            .set_recv_timeout(Some(self.timeout))
            .and_then(|()| stream.set_send_timeout(Some(self.timeout)))
            .map_err(|error| {
                self.unreachable(format!("the call timeout could not be set: {}", error))
            })?;

        let mut writer = &stream;
        writer
            .write_all(format!("{}\n", wire).as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| self.classify(&method, "cannot send", error))?;

        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(|error| self.classify(&method, "cannot read the answer to", error))?;

        self.answer(&id, &method, &line)
    }

    /// Asks the server what it is, and what it speaks.
    ///
    /// Read [`Handshake::mismatch`] on the way past. Nothing else in the kit
    /// checks the protocol, because nothing else has an answer to check.
    pub fn ping(&self) -> Result<Handshake, CallError> {
        match self.call(RequestMethod::Ping(PingParams(serde_json::Map::new())))? {
            ResponseResult::Pong {
                capabilities,
                protocol,
                version,
            } => Ok(Handshake {
                version,
                protocol,
                capabilities,
            }),
            other => Err(CallError::Protocol(format!(
                "ping answered {:?}, which is not a pong",
                other
            ))),
        }
    }

    /// Reads one answer line into a result, or into the reason it is not one.
    fn answer(&self, id: &str, method: &str, line: &str) -> Result<ResponseResult, CallError> {
        if line.trim().is_empty() {
            return Err(CallError::Protocol(format!(
                "the server closed the connection without answering {}",
                method
            )));
        }

        let answer: Answer = serde_json::from_str(line).map_err(|error| {
            CallError::Protocol(format!(
                "the answer to {} is not a Herdr response: {}",
                method, error
            ))
        })?;

        // Before the result, because an answer addressed elsewhere says
        // nothing about this call even when it is well-formed.
        if answer.id != id {
            return Err(CallError::Protocol(format!(
                "the answer to {} carries id {:?}, and the request carried {:?}",
                method, answer.id, id
            )));
        }

        match (answer.error, answer.result) {
            // An answer carrying both is not something Herdr sends. The error
            // wins: it fails closed, and it keeps the server's own words.
            (Some(error), _) => Err(CallError::Server(error)),
            (None, Some(result)) => Ok(result),
            (None, None) => Err(CallError::Protocol(format!(
                "the answer to {} carries neither a result nor an error",
                method
            ))),
        }
    }

    /// Builds the "could not reach the socket" case, naming where it came from.
    fn unreachable(&self, reason: String) -> CallError {
        CallError::Connect {
            socket: self.socket.clone(),
            reason,
        }
    }

    /// Sorts an I/O failure into the case that describes it.
    ///
    /// ✅ **Measured 2026-09-11 on macOS: a receive timeout arrives as
    /// `WouldBlock`.** Not inferred from `set_recv_timeout` existing. A server
    /// that accepts a connection and then says nothing was stood up, the call
    /// was timed, and the mutation harness confirms the arm is load-bearing:
    /// dropping `WouldBlock` from this match reddens the suite. See
    /// `tools/mutations/client.json`.
    ///
    /// ⚠️ `TimedOut` is the Windows side, where a receive timeout surfaces as
    /// `WSAETIMEDOUT`. It is matched on the documented mapping and verified by
    /// the compiler alone, like every other Windows path here. Both arms are
    /// kept because dropping the unverifiable one would leave a wedged server
    /// hanging on the platform nobody can check.
    fn classify(&self, method: &str, doing: &str, error: std::io::Error) -> CallError {
        match error.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => CallError::Timeout {
                method: method.to_string(),
                after: self.timeout,
            },
            ErrorKind::InvalidData => {
                CallError::Protocol(format!("the answer to {} is not UTF-8: {}", method, error))
            }
            _ => self.unreachable(format!("{} {}: {}", doing, method, error)),
        }
    }
}

/// The kit's own client, answering the two calls a dialog needs.
///
/// 🔑 **This is what SCOPE.md §7.5.6 was waiting for.** The trait was not a
/// workaround for the transport being unbuilt, so nothing in [`crate::dialog`]
/// changes now that it exists. A consumer supplies nothing.
#[cfg(feature = "dialog")]
impl Transport for Client {
    /// ⚠️ **Any success answers `Ok`, and that is deliberate.** The trait
    /// promises that Herdr accepted the request, not that a pane appeared. ✅
    /// A popup answers `{"type":"ok"}` and an overlay answers
    /// `plugin_pane_opened`, both measured 2026-09-11, and both are
    /// acceptance. Reading the shape here would refuse a placement the trait
    /// never restricted.
    fn open_pane(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
        match self.call(RequestMethod::PluginPaneOpen(params)) {
            Ok(_) => Ok(()),
            // The one hand-maintained code list in the kit, with its
            // measurement, lives on the other side of this call.
            Err(CallError::Server(body)) => Err(OpenError::from_error(&body.code, &body.message)),
            Err(other) => Err(OpenError::Failed(other.to_string())),
        }
    }

    fn show_notification(
        &mut self,
        params: NotificationShowParams,
    ) -> Result<NotificationShowReason, String> {
        match self.call(RequestMethod::NotificationShow(params)) {
            Ok(ResponseResult::NotificationShow { reason, .. }) => Ok(reason),
            Ok(other) => Err(format!(
                "notification.show answered {:?}, which carries no delivery reason",
                other
            )),
            Err(error) => Err(error.to_string()),
        }
    }
}
