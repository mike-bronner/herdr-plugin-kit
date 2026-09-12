//! What the socket transport promises.
//!
//! Everything here runs without a Herdr server. A scripted local-socket server
//! stands in, built on the same `interprocess` API the client uses, so the
//! bytes are real even though the peer is not.
//!
//! [`a_wedged_server_times_out_rather_than_hanging`] is the one that cannot be
//! reasoned about statically. `set_recv_timeout` being present says nothing
//! about the timeout firing, and a timeout set on the wrong handle only shows
//! itself when a server accepts a connection and then says nothing, which is
//! exactly when a plugin needs to fail fast. So a server that does precisely
//! that is stood up, and the call is timed.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName as _};
use serde_json::{json, Value};

use herdr_plugin_kit::api::client::{
    CallError, Client, Handshake, NoSocket, Origin, ProtocolMismatch, Socket, CALL_TIMEOUT,
    DEFAULT_SOCKET,
};
use herdr_plugin_kit::api::generated::{
    PaneListAnswer, PaneListParams, PingParams, RequestMethod, ResponseResult, GENERATED_PROTOCOL,
};
use herdr_plugin_kit::env::{Environment, SOCKET_PATH_VAR};

const PLUGIN: &str = "mikebronner.test-plugin";

/// Generous for a local socket, and short enough that a hang is obvious.
const PATIENT: Duration = Duration::from_secs(2);

/// What [`a_wedged_server_times_out_rather_than_hanging`] allows the call.
const IMPATIENT: Duration = Duration::from_millis(150);

/// How long a silent server holds the connection open.
///
/// An order of magnitude past [`IMPATIENT`], so a call that returns inside
/// this window returned because the timeout fired and for no other reason.
const HELD: Duration = Duration::from_millis(1_500);

/// Keeps two servers in one test run from colliding on a path.
static NEXT_SOCKET: AtomicU32 = AtomicU32::new(0);

/// A scripted Herdr, speaking the real wire protocol over a real socket.
struct Server {
    path: PathBuf,
    worker: Option<JoinHandle<Vec<Value>>>,
}

impl Server {
    /// Serves `calls` requests through `reply`, then stops.
    ///
    /// `reply` answering `None` closes the connection without a word, which is
    /// what a crashing server looks like from here.
    fn answering(calls: usize, reply: impl Fn(&Value) -> Option<Value> + Send + 'static) -> Server {
        Server::start(calls, Duration::ZERO, move |request| reply(request))
    }

    /// Accepts one request, answers nothing, and holds the connection open.
    fn wedged() -> Server {
        Server::start(1, HELD, |_| None)
    }

    fn start(
        calls: usize,
        hold: Duration,
        reply: impl Fn(&Value) -> Option<Value> + Send + 'static,
    ) -> Server {
        let path = scratch_socket();
        let name = path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .expect("a scratch socket path must map to a local socket name");
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .expect("the scratch socket must be bindable");

        let worker = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..calls {
                let Ok(stream) = listener.accept() else { break };
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    break;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    break;
                };
                if let Some(answer) = reply(&request) {
                    let mut writer = &stream;
                    let _ = writer.write_all(format!("{}\n", answer).as_bytes());
                    let _ = writer.flush();
                }
                seen.push(request);
                std::thread::sleep(hold);
            }
            seen
        });

        Server {
            path,
            worker: Some(worker),
        }
    }

    /// A client pointed at this server.
    fn client(&self) -> Client {
        Client::new(Socket::at(&self.path), PLUGIN).with_timeout(PATIENT)
    }

    /// Waits for the script to finish, and hands back what it received.
    fn requests(&mut self) -> Vec<Value> {
        self.worker
            .take()
            .expect("a server's requests can only be collected once")
            .join()
            .expect("the scripted server must not panic")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A path for a scratch server, in the shape this platform accepts.
///
/// ⚠️ **The Windows arm is compile-verified only**, like every other Windows
/// path in this crate. It is written rather than skipped because
/// `GenericFilePath` refuses any Windows path that does not already start
/// `\\.\pipe\`, so a temp-directory name would make this whole harness
/// structurally Unix-only. `cfg!` rather than `#[cfg]` so both arms are
/// type-checked everywhere.
fn scratch_socket() -> PathBuf {
    let ordinal = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
    let leaf = format!("herdr-kit-{}-{}", std::process::id(), ordinal);
    match cfg!(windows) {
        true => PathBuf::from(format!(r"\\.\pipe\{}", leaf)),
        false => std::env::temp_dir().join(format!("{}.sock", leaf)),
    }
}

/// Answers `request` with a success carrying `result`.
fn success(request: &Value, result: Value) -> Option<Value> {
    Some(json!({"id": request["id"], "result": result}))
}

/// A pane, in the shape Herdr sends one.
fn pane(id: &str) -> Value {
    json!({
        "agent_status": "unknown",
        "focused": true,
        "pane_id": id,
        "revision": 1,
        "tab_id": "t1",
        "terminal_id": "term1",
        "workspace_id": "w1",
    })
}

/// The pong a real Herdr 0.9.0 answers, with the protocol swapped in.
fn pong(request: &Value, protocol: u32) -> Option<Value> {
    success(
        request,
        json!({"type": "pong", "version": "0.9.0", "protocol": protocol}),
    )
}

/// A client whose socket does not exist, resolved the way a plugin resolves.
#[cfg(unix)]
fn absent_client(env: &Environment) -> Client {
    Client::new(
        Socket::resolve(env).expect("Unix has a documented default"),
        PLUGIN,
    )
    .with_timeout(PATIENT)
}

// ---------------------------------------------------------------- resolution

#[test]
fn the_variable_names_the_socket_when_it_is_set() {
    let env = Environment::from_pairs(&[(SOCKET_PATH_VAR, "/run/herdr/named.sock")]);
    let socket = Socket::resolve(&env).expect("a named socket resolves");

    assert_eq!(socket.path(), PathBuf::from("/run/herdr/named.sock"));
    assert_eq!(socket.origin(), Origin::Variable);
}

#[test]
#[cfg(unix)]
fn an_absent_variable_falls_back_to_the_default() {
    // ✅ Measured: a `[[build]]` hook gets zero HERDR_* variables, and an
    // event hook gets no socket path either. This is an ordinary path.
    let env = Environment::from_pairs(&[("HOME", "/home/mike")]);
    let socket = Socket::resolve(&env).expect("Unix has a documented default");

    assert_eq!(
        socket.path(),
        PathBuf::from("/home/mike/.config/herdr/herdr.sock")
    );
    assert_eq!(socket.origin(), Origin::Default);
}

#[test]
#[cfg(unix)]
fn an_empty_variable_falls_back_to_the_default() {
    // Herdr does inject empty values, and an empty path cannot be connected
    // to. `Environment::get` keeps the two apart; resolution must not.
    let env = Environment::from_pairs(&[(SOCKET_PATH_VAR, ""), ("HOME", "/home/mike")]);
    let socket = Socket::resolve(&env).expect("an empty variable is not a path");

    assert_eq!(
        socket.path(),
        PathBuf::from("/home/mike/.config/herdr/herdr.sock")
    );
    assert_eq!(socket.origin(), Origin::Default);
}

#[test]
#[cfg(unix)]
fn an_absent_home_still_resolves_rather_than_failing() {
    let env = Environment::from_pairs(&[]);
    let socket = Socket::resolve(&env).expect("home degrades to a root, never to a failure");

    assert_eq!(socket.path(), PathBuf::from("/.config/herdr/herdr.sock"));
}

#[test]
fn a_platform_with_no_documented_default_resolves_to_nothing() {
    // 🚨 The Windows arm. Reached here by passing the `None` default
    // explicitly, because a test on macOS can never reach it through
    // `resolve`, and a branch no harness can reach is not covered.
    let env = Environment::from_pairs(&[("HOME", "/home/mike")]);

    assert_eq!(Socket::resolve_with(&env, None), Err(NoSocket));
}

#[test]
fn resolving_to_nothing_names_the_variable_that_fixes_it() {
    assert!(NoSocket.to_string().contains(SOCKET_PATH_VAR));
}

#[test]
fn a_named_variable_still_wins_where_there_is_no_default() {
    let env = Environment::from_pairs(&[(SOCKET_PATH_VAR, r"\\.\pipe\herdr-abc")]);
    let socket = Socket::resolve_with(&env, None).expect("the variable answers without a default");

    assert_eq!(socket.origin(), Origin::Variable);
}

#[test]
fn the_documented_default_is_the_one_the_donors_use() {
    // Both arms, because the `None` one is the whole Windows decision and
    // nothing else pins its value.
    match cfg!(unix) {
        true => assert_eq!(DEFAULT_SOCKET, Some(".config/herdr/herdr.sock")),
        false => assert_eq!(DEFAULT_SOCKET, None),
    }
}

#[test]
#[cfg(unix)]
fn a_socket_says_where_its_path_came_from() {
    let named = Environment::from_pairs(&[(SOCKET_PATH_VAR, "/run/named.sock")]);
    let fallen_back = Environment::from_pairs(&[("HOME", "/home/mike")]);

    let from_variable = Socket::resolve(&named).unwrap().to_string();
    let from_default = Socket::resolve(&fallen_back).unwrap().to_string();
    let given = Socket::at("/run/given.sock").to_string();

    assert_eq!(
        from_variable,
        format!("/run/named.sock (from {})", SOCKET_PATH_VAR)
    );
    assert_eq!(
        from_default,
        format!(
            "/home/mike/.config/herdr/herdr.sock (the default, because {} was not set)",
            SOCKET_PATH_VAR
        )
    );
    assert_eq!(given, "/run/given.sock");
}

#[test]
fn the_default_timeout_is_the_five_seconds_recent_spaces_uses() {
    assert_eq!(CALL_TIMEOUT, Duration::from_secs(5));
}

// --------------------------------------------------------------------- calls

#[test]
fn a_call_sends_one_json_line_carrying_the_method_and_its_params() {
    let mut server = Server::answering(1, |request| success(request, json!({"type": "ok"})));
    let client = server.client();

    let result = client.call::<ResponseResult>(RequestMethod::PaneList(PaneListParams {
        workspace_id: None,
    }));

    let sent = server.requests();
    assert!(matches!(result, Ok(ResponseResult::Ok)));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["method"], json!("pane.list"));
    assert_eq!(sent[0]["params"], json!({}));
}

#[test]
fn a_request_id_is_prefixed_with_the_plugin_id_and_never_repeats() {
    let mut server = Server::answering(2, |request| pong(request, GENERATED_PROTOCOL));
    let client = server.client();

    client.ping().expect("the first call answers");
    client.ping().expect("the second call answers");

    let sent = server.requests();
    let first = sent[0]["id"].as_str().expect("an id is a string");
    let second = sent[1]["id"].as_str().expect("an id is a string");

    assert!(first.starts_with(PLUGIN), "{} lacks the plugin id", first);
    assert!(second.starts_with(PLUGIN), "{} lacks the plugin id", second);
    assert_ne!(first, second);
}

#[test]
fn a_success_comes_back_as_a_typed_result() {
    let server = Server::answering(1, |request| {
        success(request, json!({"type": "workspace_list", "workspaces": []}))
    });

    let result = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect("a well-formed success is a result");

    assert!(matches!(
        result,
        ResponseResult::WorkspaceList { ref workspaces } if workspaces.is_empty()
    ));
}

#[test]
fn a_caller_can_name_the_one_result_it_expects() {
    let server = Server::answering(1, |request| {
        success(request, json!({"type": "pane_list", "panes": [pane("p1")]}))
    });

    let listed: PaneListAnswer = server
        .client()
        .call(RequestMethod::PaneList(PaneListParams {
            workspace_id: None,
        }))
        .expect("a pane_list answer is a PaneListAnswer");

    assert_eq!(listed.panes.len(), 1);
    assert_eq!(listed.panes[0].pane_id, "p1");
}

#[test]
fn naming_the_result_a_method_does_not_answer_is_refused() {
    // 🚨 The hazard the narrow types exist to make loud. The schema declares
    // no link between a method and a result, and the obvious guess is wrong on
    // the first plugin that looked: `workspace.move` answers `workspace_list`.
    // So a wrong guess must name both tags rather than parse into silence.
    let server = Server::answering(1, |request| {
        success(request, json!({"type": "workspace_list", "workspaces": []}))
    });

    let error = server
        .client()
        .call::<PaneListAnswer>(RequestMethod::PaneList(PaneListParams {
            workspace_id: None,
        }))
        .expect_err("a workspace_list answer is not a PaneListAnswer");

    let reported = error.to_string();
    assert!(matches!(error, CallError::Protocol(_)));
    assert!(
        reported.contains("pane.list"),
        "the method is missing from {:?}",
        reported
    );
    assert!(
        reported.contains("workspace_list"),
        "the tag that arrived is missing from {:?}",
        reported
    );
    assert!(
        reported.contains("pane_list"),
        "the tag that was expected is missing from {:?}",
        reported
    );
}

#[test]
fn the_union_is_still_callable_for_a_caller_that_wants_any_answer() {
    // ⚠️ The narrow types are an option, not a replacement. A caller that
    // genuinely wants any response still has one, and pays for all 64 on
    // purpose rather than by default.
    let server = Server::answering(1, |request| {
        success(request, json!({"type": "pane_list", "panes": [pane("p1")]}))
    });

    let any = server
        .client()
        .call::<ResponseResult>(RequestMethod::PaneList(PaneListParams {
            workspace_id: None,
        }))
        .expect("the union accepts every result Herdr declares");

    assert!(matches!(any, ResponseResult::PaneList { ref panes } if panes.len() == 1));
}

#[test]
fn an_error_body_comes_back_verbatim() {
    let server = Server::answering(1, |request| {
        Some(json!({
            "id": request["id"],
            "error": {"code": "plugin_not_found", "message": "plugin not found"},
        }))
    });

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("an error body is not a result");

    match error {
        CallError::Server(body) => {
            assert_eq!(body.code, "plugin_not_found");
            assert_eq!(body.message, "plugin not found");
        }
        other => panic!("expected a server error, got {:?}", other),
    }
}

#[test]
fn an_answer_addressed_to_another_request_is_refused() {
    // 🔑 This is what the request counter is for. Without this check the
    // atomic id is decoration.
    let server = Server::answering(1, |_| {
        Some(json!({"id": "somebody-else-99", "result": {"type": "ok"}}))
    });

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("an answer to another request is not an answer to this one");

    let reported = error.to_string();
    assert!(matches!(error, CallError::Protocol(_)));
    assert!(
        reported.contains("somebody-else-99"),
        "the id received is missing from {:?}",
        reported
    );
    assert!(
        reported.contains(PLUGIN),
        "the id sent is missing from {:?}",
        reported
    );
}

#[test]
fn an_answer_that_is_not_json_is_refused() {
    let server = Server::answering(1, |_| Some(json!("not an object at all")));

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("a bare string is not a Herdr response");

    assert!(matches!(error, CallError::Protocol(_)));
}

#[test]
fn an_answer_carrying_neither_a_result_nor_an_error_is_refused() {
    let server = Server::answering(1, |request| Some(json!({"id": request["id"]})));

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("an envelope with nothing in it is not an answer");

    assert!(matches!(error, CallError::Protocol(_)));
    assert!(error.to_string().contains("neither a result nor an error"));
}

#[test]
fn a_result_shape_this_build_does_not_know_is_refused() {
    // Fails closed. A later Herdr inventing a result type must not slip
    // through as a success carrying nothing.
    let server = Server::answering(1, |request| {
        success(request, json!({"type": "invented_by_a_later_herdr"}))
    });

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("an unknown result type is not a result");

    assert!(matches!(error, CallError::Protocol(_)));
}

#[test]
fn an_error_wins_when_an_answer_somehow_carries_both() {
    let server = Server::answering(1, |request| {
        Some(json!({
            "id": request["id"],
            "result": {"type": "ok"},
            "error": {"code": "ui_busy", "message": "a popup pane is already open"},
        }))
    });

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("a contradictory answer fails closed");

    assert!(matches!(error, CallError::Server(_)));
}

#[test]
fn a_server_that_closes_without_answering_is_refused() {
    let server = Server::answering(1, |_| None);

    let error = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("silence followed by a close is not an answer");

    assert!(matches!(error, CallError::Protocol(_)));
    assert!(error.to_string().contains("without answering"));
}

#[test]
fn an_unknown_top_level_key_is_ignored_rather_than_refused() {
    // Forward compatibility on the envelope only. A later Herdr adding a
    // field must not break every plugin built before it.
    let server = Server::answering(1, |request| {
        Some(json!({
            "id": request["id"],
            "result": {"type": "ok"},
            "trace_id": "added-by-a-later-herdr",
        }))
    });

    let result = server
        .client()
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())));

    assert!(matches!(result, Ok(ResponseResult::Ok)));
}

#[test]
#[cfg(unix)]
fn a_socket_that_is_not_there_names_the_path_and_where_it_came_from() {
    let env = Environment::from_pairs(&[("HOME", "/nowhere/at/all")]);

    let error = absent_client(&env)
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("there is no server on that path");

    let reported = error.to_string();
    assert!(matches!(error, CallError::Connect { .. }));
    assert!(
        reported.contains("/nowhere/at/all/.config/herdr/herdr.sock"),
        "the path is missing from {:?}",
        reported
    );
    assert!(
        reported.contains(SOCKET_PATH_VAR),
        "the reader is not told what to set, in {:?}",
        reported
    );
}

#[test]
fn a_wedged_server_times_out_rather_than_hanging() {
    // 🚨 The measurement, not an inference from `set_recv_timeout` existing.
    // The server accepts, reads the request, and then says nothing for
    // `HELD`. A timeout set on the wrong handle would block for all of it.
    let server = Server::wedged();
    let client = Client::new(Socket::at(&server.path), PLUGIN).with_timeout(IMPATIENT);

    let started = Instant::now();
    let error = client
        .call::<ResponseResult>(RequestMethod::Ping(PingParams(serde_json::Map::new())))
        .expect_err("a wedged server answers nothing");
    let took = started.elapsed();

    match error {
        CallError::Timeout { method, after } => {
            assert_eq!(method, "ping");
            assert_eq!(after, IMPATIENT);
        }
        other => panic!("expected a timeout, got {:?}", other),
    }
    assert!(
        took < HELD,
        "the call took {:?}, so it waited for the server rather than for the timeout",
        took
    );
}

// ----------------------------------------------------------------- handshake

#[test]
fn ping_reads_the_version_the_protocol_and_the_capabilities() {
    let server = Server::answering(1, |request| {
        success(
            request,
            json!({
                "type": "pong",
                "version": "0.9.0",
                "protocol": 22,
                "capabilities": {"live_handoff": true},
            }),
        )
    });

    let handshake = server.client().ping().expect("a pong is a handshake");

    assert_eq!(handshake.version, "0.9.0");
    assert_eq!(handshake.protocol, 22);
    assert!(
        handshake
            .capabilities
            .expect("capabilities were sent")
            .live_handoff
    );
}

#[test]
fn capabilities_are_optional() {
    let server = Server::answering(1, |request| pong(request, GENERATED_PROTOCOL));

    let handshake = server.client().ping().expect("a pong without capabilities");

    assert!(handshake.capabilities.is_none());
}

#[test]
fn a_matching_protocol_is_not_a_mismatch() {
    let server = Server::answering(1, |request| pong(request, GENERATED_PROTOCOL));

    let handshake = server.client().ping().expect("a pong is a handshake");

    assert_eq!(handshake.mismatch(), None);
}

#[test]
fn a_protocol_the_kit_was_not_built_for_is_a_mismatch_naming_both_numbers() {
    let moved = GENERATED_PROTOCOL + 1;
    let server = Server::answering(1, move |request| pong(request, moved));

    let handshake = server.client().ping().expect("a pong is still a handshake");
    let mismatch = handshake.mismatch().expect("the protocols differ");

    assert_eq!(mismatch.server, moved);
    assert_eq!(mismatch.generated, GENERATED_PROTOCOL);

    let warning = mismatch.to_string();
    assert!(
        warning.contains(&moved.to_string()),
        "the server's protocol is missing from {:?}",
        warning
    );
    assert!(
        warning.contains(&GENERATED_PROTOCOL.to_string()),
        "the generated protocol is missing from {:?}",
        warning
    );
}

#[test]
fn a_mismatch_is_a_warning_and_never_a_failure() {
    // ⚠️ A plugin that still works has to keep working. The call succeeds,
    // and the diagnosis rides along beside the result.
    let server = Server::answering(1, |request| pong(request, GENERATED_PROTOCOL + 7));

    let handshake = server.client().ping();

    assert!(handshake.is_ok(), "a mismatch must not fail the call");
    assert!(handshake.unwrap().mismatch().is_some());
}

#[test]
fn a_mismatch_is_decided_by_the_numbers_and_nothing_else() {
    let matching = Handshake {
        version: "1.0.0".to_string(),
        protocol: GENERATED_PROTOCOL,
        capabilities: None,
    };
    let moved = Handshake {
        protocol: GENERATED_PROTOCOL + 1,
        ..matching.clone()
    };

    assert_eq!(matching.mismatch(), None);
    assert_eq!(
        moved.mismatch(),
        Some(ProtocolMismatch {
            server: GENERATED_PROTOCOL + 1,
            generated: GENERATED_PROTOCOL,
        })
    );
}

#[test]
fn ping_refuses_an_answer_that_is_not_a_pong() {
    let server = Server::answering(1, |request| success(request, json!({"type": "ok"})));

    let error = server
        .client()
        .ping()
        .expect_err("an ok is not a handshake");

    let reported = error.to_string();
    assert!(matches!(error, CallError::Protocol(_)));
    assert!(
        reported.contains("ping"),
        "the method is missing from {:?}",
        reported
    );
    assert!(
        reported.contains("pong"),
        "the tag expected is missing from {:?}",
        reported
    );
}

// ------------------------------------------------------------- dialog's seam

/// The client standing in for the caller-supplied sender `dialog` used to need.
///
/// SCOPE.md §7.5.6 said the trait was not a workaround for this module being
/// unbuilt, and that when it landed no caller would change. These hold that up.
#[cfg(feature = "dialog")]
mod dialog_seam {
    use super::*;

    use herdr_plugin_kit::api::generated::{
        NotificationShowParams, NotificationShowReason, PluginPaneOpenParams, PluginPanePlacement,
    };
    use herdr_plugin_kit::dialog::{OpenError, Transport, BUSY_CODE};

    fn open_params() -> PluginPaneOpenParams {
        PluginPaneOpenParams {
            cwd: None,
            direction: None,
            entrypoint: "dialog".to_string(),
            env: Default::default(),
            focus: true,
            height: None,
            placement: Some(PluginPanePlacement::Popup),
            plugin_id: PLUGIN.to_string(),
            target_pane_id: None,
            width: None,
            workspace_id: None,
        }
    }

    fn notify_params() -> NotificationShowParams {
        NotificationShowParams {
            body: Some("two panes are running agents".to_string()),
            position: None,
            sound: None,
            title: "Careful".to_string(),
        }
    }

    #[test]
    fn the_client_opens_a_pane_through_the_seam() {
        // ✅ A popup answers `{"type":"ok"}`, measured 2026-09-11 on 0.9.0.
        let mut server = Server::answering(1, |request| success(request, json!({"type": "ok"})));
        let mut client = server.client();

        let opened = client.open_pane(open_params());

        let sent = server.requests();
        assert_eq!(opened, Ok(()));
        assert_eq!(sent[0]["method"], json!("plugin.pane.open"));
        assert_eq!(sent[0]["params"]["placement"], json!("popup"));
    }

    #[test]
    fn an_overlay_style_answer_is_acceptance_too() {
        // The trait promises Herdr accepted the request, not that a popup
        // appeared. An overlay answers `plugin_pane_opened` and is accepted.
        let server = Server::answering(1, |request| {
            success(
                request,
                json!({
                    "type": "plugin_pane_opened",
                    "plugin_pane": {
                        "entrypoint": "dialog",
                        "plugin_id": PLUGIN,
                        "pane": {
                            "agent_status": "unknown",
                            "focused": true,
                            "pane_id": "p1",
                            "revision": 1,
                            "tab_id": "t1",
                            "terminal_id": "term1",
                            "workspace_id": "w1",
                        },
                    },
                }),
            )
        });

        assert_eq!(server.client().open_pane(open_params()), Ok(()));
    }

    #[test]
    fn a_result_shape_this_build_does_not_know_is_acceptance_too() {
        // ⚠️ Wider than the union was, and deliberately. The promise is that
        // Herdr accepted the request, so a future placement answering a future
        // shape is still acceptance. Refusing it would be reading the shape.
        let server = Server::answering(1, |request| {
            success(request, json!({"type": "invented_by_a_later_herdr"}))
        });

        assert_eq!(server.client().open_pane(open_params()), Ok(()));
    }

    #[test]
    fn an_answer_carrying_no_type_at_all_is_not_acceptance() {
        // The floor under the widening. A Herdr result carries a tag, and an
        // answer without one is malformed rather than unread.
        let server = Server::answering(1, |request| success(request, json!({"pane": "p1"})));

        let refused = server.client().open_pane(open_params());

        assert!(matches!(refused, Err(OpenError::Failed(_))));
    }

    #[test]
    fn a_busy_popup_reaches_the_dialog_module_as_busy() {
        // ✅ Reproduced verbatim 2026-09-11 by opening a second popup while
        // one was up. This is the one code the kit matches.
        let server = Server::answering(1, |request| {
            Some(json!({
                "id": request["id"],
                "error": {"code": BUSY_CODE, "message": "a popup pane is already open"},
            }))
        });

        let refused = server.client().open_pane(open_params());

        assert_eq!(refused, Err(OpenError::Busy));
    }

    #[test]
    fn any_other_refusal_reaches_the_dialog_module_as_failed() {
        // ✅ Measured 2026-09-11: a plugin not yet registered answers this.
        let server = Server::answering(1, |request| {
            Some(json!({
                "id": request["id"],
                "error": {"code": "plugin_not_found", "message": "plugin not found"},
            }))
        });

        let refused = server.client().open_pane(open_params());

        match refused {
            Err(OpenError::Failed(detail)) => {
                assert!(detail.contains("plugin_not_found"), "{}", detail);
                assert!(detail.contains("plugin not found"), "{}", detail);
            }
            other => panic!("expected a failure, got {:?}", other),
        }
    }

    #[test]
    fn a_transport_failure_reaches_the_dialog_module_as_failed() {
        let mut client = Client::new(Socket::at("/nowhere/at/all/herdr.sock"), PLUGIN)
            .with_timeout(super::PATIENT);

        let refused = client.open_pane(open_params());

        assert!(matches!(refused, Err(OpenError::Failed(_))));
        assert_ne!(refused, Err(OpenError::Busy));
    }

    #[test]
    fn the_client_hands_back_the_notification_delivery_reason() {
        // 🚨 SCOPE.md §7.2: returning the reason is the contract. An
        // implementation answering a bare success reintroduces the defect the
        // dialog module exists to fix.
        let mut server = Server::answering(1, |request| {
            success(
                request,
                json!({"type": "notification_show", "reason": "rate_limited", "shown": false}),
            )
        });

        let shown = server.client().show_notification(notify_params());

        let sent = server.requests();
        assert_eq!(shown, Ok(NotificationShowReason::RateLimited));
        assert_eq!(sent[0]["method"], json!("notification.show"));
    }

    #[test]
    fn a_delivered_notification_is_told_apart_from_a_dropped_one() {
        let server = Server::answering(1, |request| {
            success(
                request,
                json!({"type": "notification_show", "reason": "shown", "shown": true}),
            )
        });

        assert_eq!(
            server.client().show_notification(notify_params()),
            Ok(NotificationShowReason::Shown)
        );
    }

    #[test]
    fn a_notification_that_cannot_be_sent_is_an_error_rather_than_a_reason() {
        let mut client = Client::new(Socket::at("/nowhere/at/all/herdr.sock"), PLUGIN)
            .with_timeout(super::PATIENT);

        let sent = client.show_notification(notify_params());

        match sent {
            Err(detail) => assert!(detail.contains("cannot reach"), "{}", detail),
            Ok(reason) => panic!("nothing was sent, so there is no reason: {:?}", reason),
        }
    }

    #[test]
    fn a_notification_answered_with_the_wrong_shape_is_an_error() {
        let server = Server::answering(1, |request| success(request, json!({"type": "ok"})));

        let sent = server.client().show_notification(notify_params());

        match sent {
            Err(detail) => assert!(detail.contains("notification_show"), "{}", detail),
            Ok(reason) => panic!("an ok carries no reason, got {:?}", reason),
        }
    }
}
