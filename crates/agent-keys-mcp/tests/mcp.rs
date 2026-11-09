//! The agent's MCP tool: the JSON-RPC side, the tool arguments, the replies it gives the agent,
//! and (on a Mac) the socket client against a fake app in a temp folder. Every value below is an
//! obviously fake placeholder.

use std::cell::RefCell;

use agent_keys_mcp::{
    KINDS, MCP_VERSIONS, Route, RouteFailure, Server, VALUE_NAMES, describe, error_text,
    route_request, tools,
};
use serde_json::{Value, json};

/// Records what would go to the app and answers with a fixed reply.
struct FakeRoute {
    sent: RefCell<Vec<(Value, bool)>>,
    reply: Result<Value, RouteFailure>,
}

impl FakeRoute {
    fn answering(reply: Value) -> FakeRoute {
        FakeRoute {
            sent: RefCell::new(Vec::new()),
            reply: Ok(reply),
        }
    }
}

impl Route for &FakeRoute {
    fn send(&self, request: &Value, wait_for_owner: bool) -> Result<Value, RouteFailure> {
        self.sent
            .borrow_mut()
            .push((request.clone(), wait_for_owner));
        self.reply.clone()
    }
}

fn call(server: &Server<&FakeRoute>, message: Value) -> Value {
    serde_json::from_str(&server.handle_line(&message.to_string()).unwrap()).unwrap()
}

fn tool_call(server: &Server<&FakeRoute>, name: &str, arguments: Value) -> (String, bool) {
    let reply = call(
        server,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
    );
    let result = &reply["result"];
    (
        result["content"][0]["text"].as_str().unwrap().to_owned(),
        result["isError"].as_bool().unwrap(),
    )
}

// ---------- JSON-RPC ----------

#[test]
fn initialize_echoes_a_known_version_or_offers_the_newest() {
    let route = FakeRoute::answering(json!({}));
    let server = Server::new(&route);
    for version in MCP_VERSIONS {
        let reply = call(
            &server,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"fake-client","version":"0"}}}),
        );
        assert_eq!(reply["id"], json!(1));
        assert_eq!(reply["result"]["protocolVersion"], json!(version));
        assert_eq!(
            reply["result"]["serverInfo"]["name"],
            json!("enote-agent-keys")
        );
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }
    let reply = call(
        &server,
        json!({"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
    );
    assert_eq!(reply["result"]["protocolVersion"], json!("2025-06-18"));
    assert!(route.sent.borrow().is_empty());
}

#[test]
fn notifications_and_responses_get_no_answer() {
    let route = FakeRoute::answering(json!({}));
    let server = Server::new(&route);
    for message in [
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}),
        json!({"jsonrpc":"2.0","id":3,"result":{}}),
    ] {
        assert_eq!(server.handle_line(&message.to_string()), None, "{message}");
    }
}

#[test]
fn ping_unknown_methods_and_bad_messages() {
    let route = FakeRoute::answering(json!({}));
    let server = Server::new(&route);
    assert_eq!(
        call(&server, json!({"jsonrpc":"2.0","id":2,"method":"ping"})),
        json!({"jsonrpc":"2.0","id":2,"result":{}})
    );
    let code = |line: &str| -> Value {
        let reply: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        reply["error"]["code"].clone()
    };
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#),
        json!(-32601)
    );
    assert_eq!(code("not json"), json!(-32700));
    assert_eq!(
        code(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#),
        json!(-32600)
    );
    assert_eq!(code(r#"{"id":1,"method":"ping"}"#), json!(-32600));
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#),
        json!(-32600)
    );
    assert_eq!(
        code(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"show_key","arguments":{}}}"#
        ),
        json!(-32602)
    );
    assert!(route.sent.borrow().is_empty());
}

#[test]
fn the_five_tools_take_no_extra_arguments_and_say_they_never_see_a_value() {
    let tools = tools();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "store_key",
            "list_keys",
            "edit_key_details",
            "ask_to_replace_value",
            "ask_to_delete"
        ]
    );
    for tool in tools.as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], json!("object"));
        assert_eq!(tool["inputSchema"]["additionalProperties"], json!(false));
        for name in VALUE_NAMES {
            assert!(tool["inputSchema"]["properties"].get(name).is_none());
        }
    }
    for name in ["store_key", "edit_key_details", "ask_to_replace_value"] {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == json!(name))
            .unwrap();
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .contains("never sees or returns one"),
            "{name}"
        );
    }
    let route = FakeRoute::answering(json!({}));
    let listed = call(
        &Server::new(&route),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
    );
    assert_eq!(listed["result"]["tools"], tools);
}

// ---------- tool arguments ----------

#[test]
fn each_tool_becomes_one_route_request() {
    assert_eq!(
        route_request(
            "store_key",
            &json!({"kind":"private_key","name":"Fake wallet","details":{"chain":"evm","address":"0x00000000000000000000000000000000000000a1"}})
        ),
        Ok((
            json!({"v":1,"op":"store","kind":"private_key","title":"Fake wallet","details":{"chain":"evm","address":"0x00000000000000000000000000000000000000a1"}}),
            true
        ))
    );
    assert_eq!(
        route_request("store_key", &json!({"kind":"text","name":"Fake note"})),
        Ok((
            json!({"v":1,"op":"store","kind":"text","title":"Fake note","details":{}}),
            true
        ))
    );
    assert_eq!(
        route_request("list_keys", &json!({})),
        Ok((json!({"v":1,"op":"list"}), false))
    );
    assert_eq!(
        route_request("list_keys", &Value::Null),
        Ok((json!({"v":1,"op":"list"}), false))
    );
    assert_eq!(
        route_request(
            "edit_key_details",
            &json!({"id":"fake-id","name":"Fake renamed","details":{"purpose":""}})
        ),
        Ok((
            json!({"v":1,"op":"edit","id":"fake-id","title":"Fake renamed","details":{"purpose":""}}),
            true
        ))
    );
    assert_eq!(
        route_request("ask_to_replace_value", &json!({"id":"fake-id"})),
        Ok((json!({"v":1,"op":"replace","id":"fake-id"}), true))
    );
    assert_eq!(
        route_request("ask_to_delete", &json!({"id":"fake-id"})),
        Ok((json!({"v":1,"op":"delete","id":"fake-id"}), true))
    );
}

#[test]
fn a_value_is_refused_before_anything_is_sent() {
    for name in VALUE_NAMES {
        for spelled in [
            name.to_owned(),
            name.to_ascii_uppercase(),
            format!(" {name} "),
        ] {
            for tool in [
                "store_key",
                "edit_key_details",
                "ask_to_replace_value",
                "list_keys",
            ] {
                let arguments = json!({"kind":"api_key","name":"Fake","id":"fake-id", spelled.clone(): "fake-value-7801"});
                let refused = route_request(tool, &arguments).unwrap_err();
                assert!(
                    refused.contains("looks like a value"),
                    "{tool} {spelled}: {refused}"
                );
                assert!(!refused.contains("fake-value-7801"));
            }
            for tool in ["store_key", "edit_key_details"] {
                let arguments = json!({"kind":"api_key","name":"Fake","id":"fake-id","details":{spelled.clone(): "fake-value-7802"}});
                let refused = route_request(tool, &arguments).unwrap_err();
                assert!(
                    refused.contains("is a value"),
                    "{tool} {spelled}: {refused}"
                );
                assert!(!refused.contains("fake-value-7802"));
            }
        }
    }
}

#[test]
fn details_must_be_the_kinds_visible_text_fields() {
    for (kind, fields) in KINDS {
        for field in fields {
            let arguments = json!({"kind":kind,"name":"Fake","details":{*field:"fake-detail"}});
            assert!(
                route_request("store_key", &arguments).is_ok(),
                "{kind} {field}"
            );
        }
    }
    for arguments in [
        json!({"kind":"login","name":"Fake","details":{"service":"fake"}}),
        json!({"kind":"text","name":"Fake","details":{"purpose":"fake"}}),
        json!({"kind":"seed_phrase","name":"Fake","details":{"word_count":"12"}}),
        json!({"kind":"api_key","name":"Fake","details":{"purpose":5}}),
        json!({"kind":"api_key","name":"Fake","details":"service=fake"}),
        json!({"kind":"wallet","name":"Fake"}),
        json!({"kind":"api_key"}),
        json!({"name":"Fake"}),
        json!({"kind":"api_key","name":7}),
        json!({"kind":"api_key","name":"Fake","note":"fake"}),
    ] {
        assert!(
            route_request("store_key", &arguments).is_err(),
            "{arguments}"
        );
    }
    for arguments in [
        json!({"id":"fake-id"}),
        json!({"name":"Fake"}),
        json!({"id":"fake-id","details":{"api_key":"fake"}}),
    ] {
        assert!(
            route_request("edit_key_details", &arguments).is_err(),
            "{arguments}"
        );
    }
    assert!(route_request("ask_to_delete", &json!({})).is_err());
    assert!(route_request("ask_to_delete", &json!(["fake-id"])).is_err());
    assert!(route_request("list_keys", &json!({"all": true})).is_err());
}

#[test]
fn a_refused_call_never_reaches_the_app() {
    let route = FakeRoute::answering(json!({"v":1,"ok":true,"result":"saved","id":"fake-id"}));
    let server = Server::new(&route);
    let (text, is_error) = tool_call(
        &server,
        "store_key",
        json!({"kind":"login","name":"Fake","details":{"password":"fake-value-7803"}}),
    );
    assert!(is_error);
    assert!(!text.contains("fake-value-7803"));
    assert!(route.sent.borrow().is_empty());
}

// ---------- replies ----------

#[test]
fn replies_become_plain_results() {
    let route = FakeRoute::answering(json!({"v":1,"ok":true,"result":"saved","id":"fake-id-7804"}));
    let server = Server::new(&route);
    let (text, is_error) = tool_call(
        &server,
        "store_key",
        json!({"kind":"api_key","name":"Fake","details":{"service":"fake-svc"}}),
    );
    assert!(!is_error);
    assert!(text.starts_with("Saved."), "{text}");
    assert!(text.contains("fake-id-7804"));
    assert!(route.sent.borrow()[0].1);

    let keys = json!([{"id":"fake-id","kind":"api_key","title":"Fake","visible":{"service":"fake-svc"},"created_at":1,"updated_at":2}]);
    let (text, is_error) = describe("list_keys", &json!({"v":1,"ok":true,"keys":keys}));
    assert!(!is_error);
    assert!(text.starts_with("1 key you added"), "{text}");
    assert!(text.contains("fake-svc"));
    assert!(!describe("list_keys", &json!({"v":1,"ok":true,"keys":[]})).1);
    assert_eq!(
        describe(
            "edit_key_details",
            &json!({"v":1,"ok":true,"result":"done"})
        ),
        ("Done.".to_owned(), false)
    );
    assert!(
        !describe(
            "ask_to_delete",
            &json!({"v":1,"ok":true,"result":"deleted"})
        )
        .1
    );
    assert!(
        !describe(
            "ask_to_replace_value",
            &json!({"v":1,"ok":true,"result":"saved","id":"x"})
        )
        .1
    );
    // A result that doesn't fit the tool is an error, not a success.
    assert!(describe("ask_to_delete", &json!({"v":1,"ok":true,"result":"saved"})).1);
    assert!(describe("store_key", &json!({"v":1})).1);
}

#[test]
fn every_error_code_has_its_own_words() {
    let codes = [
        "locked",
        "off",
        "not_found",
        "not_allowed",
        "busy",
        "not_now",
        "cancelled",
        "invalid",
        "internal",
    ];
    let mut seen = std::collections::BTreeSet::new();
    for code in codes {
        let (text, is_error) = describe("list_keys", &json!({"v":1,"error":code}));
        assert!(is_error);
        assert_eq!(text, error_text(code));
        assert!(seen.insert(text), "{code}");
    }
    assert_eq!(error_text("anything-else"), error_text("internal"));
}

#[test]
fn a_route_failure_is_an_error_result() {
    for failure in [
        RouteFailure::NoOwner,
        RouteFailure::NotRunning,
        RouteFailure::WrongServer,
        RouteFailure::Broken,
    ] {
        let route = FakeRoute {
            sent: RefCell::new(Vec::new()),
            reply: Err(failure),
        };
        let (text, is_error) = tool_call(&Server::new(&route), "list_keys", json!({}));
        assert!(is_error);
        assert!(!text.is_empty());
    }
}

// ---------- waiting for the owner ----------

/// Answers "saved" after a while, as the app does once the owner chooses Save.
struct SlowRoute(std::time::Duration);

impl Route for SlowRoute {
    fn send(&self, _request: &Value, _wait_for_owner: bool) -> Result<Value, RouteFailure> {
        std::thread::sleep(self.0);
        Ok(json!({"v":1,"ok":true,"result":"saved","id":"fake-id-7806"}))
    }
}

fn store_call(meta: Option<Value>) -> String {
    let mut params = json!({"name":"store_key","arguments":{"kind":"text","name":"Fake"}});
    if let Some(meta) = meta {
        params["_meta"] = meta;
    }
    json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":params}).to_string()
}

#[test]
fn a_call_waiting_for_the_owner_sends_progress_when_asked() {
    use std::sync::Mutex;
    use std::time::Duration;
    let server = Server::new(SlowRoute(Duration::from_millis(400)))
        .with_progress_every(Duration::from_millis(50));
    let sent = Mutex::new(Vec::new());
    let notify = |line: String| sent.lock().unwrap().push(line);
    let reply = server
        .handle_line_notifying(
            &store_call(Some(json!({"progressToken": "fake-token-1"}))),
            &notify,
        )
        .unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["id"], json!(9));
    assert_eq!(reply["result"]["isError"], json!(false));
    let progress: Vec<Value> = sent
        .lock()
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(progress.len() >= 2, "{progress:?}");
    for (at, note) in progress.iter().enumerate() {
        assert_eq!(note["method"], json!("notifications/progress"));
        assert!(note.get("id").is_none());
        assert_eq!(note["params"]["progressToken"], json!("fake-token-1"));
        assert_eq!(note["params"]["progress"], json!(at + 1));
        assert_eq!(
            note["params"]["message"],
            json!(agent_keys_mcp::WAITING_TEXT)
        );
    }
    // None without a token, and none for a call that doesn't wait (list_keys).
    sent.lock().unwrap().clear();
    server.handle_line_notifying(&store_call(None), &notify);
    let list = json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_keys","arguments":{},"_meta":{"progressToken":7}}});
    server.handle_line_notifying(&list.to_string(), &notify);
    assert!(sent.lock().unwrap().is_empty());
}

#[test]
fn the_calls_that_wait_say_what_to_do_after_a_client_timeout() {
    for tool in tools().as_array().unwrap() {
        let waits = tool["name"] != json!("list_keys");
        let says = tool["description"]
            .as_str()
            .unwrap()
            .contains("check with list_keys");
        assert_eq!(says, waits, "{}", tool["name"]);
    }
    assert!(agent_keys_mcp::is_tool_call(&store_call(None)));
    assert!(!agent_keys_mcp::is_tool_call(
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
    ));
    assert!(!agent_keys_mcp::is_tool_call("not json"));
}

// ---------- the socket client (macOS) ----------

#[cfg(target_os = "macos")]
mod socket {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use agent_keys_mcp::{Config, Route, RouteFailure, SocketRoute, peer};
    use serde_json::{Value, json};

    /// A fake app: answers one connection with `reply` and returns what it was sent.
    fn fake_app(reply: Option<&'static str>) -> (tempfile::TempDir, PathBuf, JoinHandle<String>) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            // Fails if the client already went away, which some tests do on purpose.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut got = String::new();
            let mut reader = BufReader::new(&stream);
            let _ = reader.read_line(&mut got);
            if let Some(reply) = reply {
                let _ = (&stream).write_all(reply.as_bytes());
            } else {
                // Never answer; hold the connection until the client gives up.
                let _ = reader.read_to_end(&mut Vec::new());
            }
            got
        });
        (temp, path, server)
    }

    fn config(path: PathBuf, owner_uid: Option<u32>) -> Config {
        Config {
            socket: path,
            owner_uid,
            owner_wait: Duration::from_secs(10),
            quick_wait: Duration::from_millis(500),
        }
    }

    #[test]
    fn the_owners_app_gets_one_line_and_answers_one_line() {
        let (_temp, path, server) = fake_app(Some("{\"v\":1,\"ok\":true,\"keys\":[]}\n"));
        let route = SocketRoute(config(path, Some(peer::own_uid())));
        let request = json!({"v":1,"op":"list"});
        assert_eq!(
            route.send(&request, false),
            Ok(json!({"v":1,"ok":true,"keys":[]}))
        );
        let got = server.join().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&got).unwrap(), request);
        assert!(got.ends_with('\n'));
    }

    #[test]
    fn a_socket_run_by_anyone_else_is_never_sent_a_byte() {
        let (_temp, path, server) = fake_app(Some("{\"v\":1,\"ok\":true,\"keys\":[]}\n"));
        let route = SocketRoute(config(path, Some(peer::own_uid() + 1)));
        assert_eq!(
            route.send(&json!({"v":1,"op":"list"}), false),
            Err(RouteFailure::WrongServer)
        );
        drop(route);
        assert_eq!(server.join().unwrap(), "");
    }

    #[test]
    fn no_owner_no_app_and_bad_answers() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("agent.sock");
        let request = json!({"v":1,"op":"list"});
        assert_eq!(
            SocketRoute(config(missing.clone(), None)).send(&request, false),
            Err(RouteFailure::NoOwner)
        );
        assert_eq!(
            SocketRoute(config(missing, Some(peer::own_uid()))).send(&request, false),
            Err(RouteFailure::NotRunning)
        );
        for reply in [
            "not json\n",
            "{\"v\":2,\"ok\":true}\n",
            "{\"v\":1,\"ok\":true}",
        ] {
            let (_temp, path, server) = fake_app(Some(reply));
            assert_eq!(
                SocketRoute(config(path, Some(peer::own_uid()))).send(&request, false),
                Err(RouteFailure::Broken),
                "{reply}"
            );
            server.join().unwrap();
        }
        // Never answered within the wait.
        let (_temp, path, server) = fake_app(None);
        assert_eq!(
            SocketRoute(config(path, Some(peer::own_uid()))).send(&request, false),
            Err(RouteFailure::Broken)
        );
        server.join().unwrap();
    }

    /// The binary end to end: MCP on stdin and stdout, the fake app on the socket.
    #[test]
    fn the_binary_serves_mcp_on_stdio() {
        use std::process::{Command, Stdio};
        let (_temp, path, server) = fake_app(Some(
            "{\"v\":1,\"ok\":true,\"result\":\"saved\",\"id\":\"fake-id-7805\"}\n",
        ));
        let mut child = Command::new(env!("CARGO_BIN_EXE_enote-agent-keys"))
            .args(["--owner-uid", &peer::own_uid().to_string(), "--socket"])
            .arg(&path)
            .env_remove("ENOTE_OWNER_UID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let too_long = "x".repeat(agent_keys_mcp::MAX_LINE_BYTES + 10);
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#).unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
        )
        .unwrap();
        writeln!(stdin, "{too_long}").unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"store_key","arguments":{{"kind":"api_key","name":"Fake","details":{{"service":"fake-svc"}}}}}}}}"#).unwrap();
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let lines: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0]["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(lines[1]["error"]["code"], json!(-32700));
        assert_eq!(lines[2]["id"], json!(2));
        assert_eq!(lines[2]["result"]["isError"], json!(false));
        assert!(
            lines[2]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("fake-id-7805")
        );
        let sent: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"v":1,"op":"store","kind":"api_key","title":"Fake","details":{"service":"fake-svc"}})
        );
    }

    /// A store waiting for the owner doesn't hold up the next message: a client that gave up on
    /// it can still ping and list.
    #[test]
    fn the_binary_answers_other_calls_while_one_waits_for_the_owner() {
        use std::process::{Command, Stdio};
        // A fake app that takes the store and never answers it: the owner hasn't chosen Save.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (got, request) = std::sync::mpsc::channel::<String>();
        let app = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            got.send(line).unwrap();
            // Held until the tool goes away.
            let _ = reader.read_to_end(&mut Vec::new());
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_enote-agent-keys"))
            .args(["--owner-uid", &peer::own_uid().to_string(), "--socket"])
            .arg(&path)
            .env_remove("ENOTE_OWNER_UID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        // If the binary hangs, end it rather than the test run.
        let (done, watchdog) = std::sync::mpsc::channel::<()>();
        let watcher = std::thread::spawn(move || {
            let _ = watchdog.recv_timeout(Duration::from_secs(20));
            let _ = child.kill();
            child.wait().unwrap()
        });
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"store_key","arguments":{{"kind":"text","name":"Fake"}}}}}}"#).unwrap();
        // The store is with the app, waiting for the owner.
        let sent: Value =
            serde_json::from_str(&request.recv_timeout(Duration::from_secs(20)).unwrap()).unwrap();
        assert_eq!(sent["op"], json!("store"));
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"ping"}}"#).unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], json!(3), "{line}");
        assert_eq!(reply["result"], json!({}));
        drop(done);
        watcher.join().unwrap();
        app.join().unwrap();
    }
}
