#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Request {
    head: String,
    body: String,
}

struct Response {
    status: &'static str,
    body: &'static str,
    headers: &'static [(&'static str, &'static str)],
}

fn serve(responses: Vec<Response>) -> (String, thread::JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        responses
            .into_iter()
            .map(|response| {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    response.body.len()
                )
                .unwrap();
                for (name, value) in response.headers {
                    write!(stream, "{name}: {value}\r\n").unwrap();
                }
                write!(stream, "\r\n{}", response.body).unwrap();
                request
            })
            .collect()
    });
    (address, handle)
}

fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "connection closed before request headers");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let content_length = head
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "connection closed before request body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    Request {
        head,
        body: String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap(),
    }
}

fn adapter() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".factory/sources/asana")
}

fn client() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".factory/clients/asana")
}

fn configured(command: &mut Command, api_base: &str) {
    command
        .env("ASANA_ACCESS_TOKEN", "test-secret-token")
        .env("ASANA_PROJECT_GID", "project-1")
        .env("ASANA_WORKSPACE_GID", "workspace-1")
        .env("ASANA_API_BASE_URL", api_base)
        .env("ASANA_ALLOW_INSECURE_LOCALHOST", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
}

fn run_client(api_base: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new(client());
    command.args(arguments);
    configured(&mut command, api_base);
    command.output().unwrap()
}

#[test]
fn asana_adapter_discovers_exact_section_and_tag_matches() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-1","name":"Ready To Implement"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-42","name":"Fix polling","notes":"The daemon misses work.","completed":false,"modified_at":"2026-07-28T12:00:00.000Z","permalink_url":"https://app.asana.com/0/project-1/task-42","created_by":{"name":"Maintainer"},"tags":[{"gid":"tag-1","name":"flashy-factory:ready"}]},{"gid":"task-43","name":"Not authorized","completed":false,"tags":[]}],"next_page":null}"#,
            headers: &[],
        },
    ]);
    let mut command = Command::new(adapter());
    command.args([
        "--state",
        "Ready To Implement",
        "--label",
        "flashy-factory:ready",
    ]);
    configured(&mut command, &api_base);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["issues"].as_array().unwrap().len(), 1);
    assert_eq!(value["issues"][0]["key"], "task-42");
    assert_eq!(value["issues"][0]["title"], "Fix polling");
    assert_eq!(value["issues"][0]["state"], "Ready To Implement");
    assert_eq!(value["issues"][0]["labels"][0], "flashy-factory:ready");
    assert_eq!(value["issues"][0]["author"], "Maintainer");
    assert!(
        value["issues"][0]["revision"]
            .as_str()
            .unwrap()
            .contains("section-1")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-secret-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-secret-token"));

    let requests = server.join().unwrap();
    assert!(
        requests[0]
            .head
            .starts_with("GET /projects/project-1/sections?")
    );
    assert!(requests[1].head.starts_with("GET /tasks?"));
    assert!(requests[1].head.contains("project=project-1"));
    assert!(requests[1].head.contains("section=section-1"));
    assert!(requests.iter().all(|request| {
        request
            .head
            .contains("Authorization: Bearer test-secret-token")
    }));
}

#[test]
fn asana_client_supports_focused_agent_updates() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"story-1"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-review","name":"Review"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-ready","name":"flashy-factory:ready"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.md");
    let comment = temp.path().join("comment.md");
    fs::write(&notes, "Updated acceptance criteria\n").unwrap();
    fs::write(&comment, "PR is ready for review.\n").unwrap();

    let update = run_client(
        &api_base,
        &["update", "task-42", "--notes-file", notes.to_str().unwrap()],
    );
    let comment_output = run_client(
        &api_base,
        &[
            "comment",
            "task-42",
            "--text-file",
            comment.to_str().unwrap(),
        ],
    );
    let move_output = run_client(&api_base, &["move", "task-42", "--section", "Review"]);
    let tag_output = run_client(
        &api_base,
        &["add-tag", "task-42", "--tag", "flashy-factory:ready"],
    );
    for output in [update, comment_output, move_output, tag_output] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let requests = server.join().unwrap();
    assert!(requests[0].head.starts_with("GET /tasks/task-42?"));
    assert!(requests[1].head.starts_with("PUT /tasks/task-42 "));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[1].body).unwrap(),
        serde_json::json!({"data":{"notes":"Updated acceptance criteria\n"}})
    );
    assert!(requests[2].head.starts_with("GET /tasks/task-42?"));
    assert!(requests[3].head.starts_with("POST /tasks/task-42/stories "));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[3].body).unwrap(),
        serde_json::json!({"data":{"text":"PR is ready for review.\n"}})
    );
    assert!(
        requests[6]
            .head
            .starts_with("POST /sections/section-review/addTask ")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[6].body).unwrap(),
        serde_json::json!({"data":{"task":"task-42"}})
    );
    assert!(requests[9].head.starts_with("POST /tasks/task-42/addTag "));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[9].body).unwrap(),
        serde_json::json!({"data":{"tag":"tag-ready"}})
    );
}

#[test]
fn asana_client_creates_tasks_in_an_existing_section() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-new","name":"New defect"}}"#,
            headers: &[],
        },
    ]);
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("new-task.md");
    fs::write(&notes, "Evidence and acceptance criteria.\n").unwrap();
    let output = run_client(
        &api_base,
        &[
            "create",
            "--name",
            "New defect",
            "--section",
            "Ready For Spec",
            "--notes-file",
            notes.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap();
    assert!(requests[1].head.starts_with("POST /tasks "));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[1].body).unwrap(),
        serde_json::json!({
            "data": {
                "name": "New defect",
                "notes": "Evidence and acceptance criteria.\n",
                "memberships": [{
                    "project": "project-1",
                    "section": "section-ready"
                }]
            }
        })
    );
}

#[test]
fn asana_client_get_includes_bounded_task_comments() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","name":"Fix it","notes":"Acceptance criteria","memberships":[{"project":{"gid":"project-1","name":"Flashy Factory"},"section":{"gid":"ready","name":"Ready To Implement"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"story-comment","type":"comment","text":"Human clarification","created_by":{"name":"Maintainer"}},{"gid":"story-system","type":"system","text":"moved this task"}],"next_page":null}"#,
            headers: &[],
        },
    ]);
    let output = run_client(&api_base, &["get", "task-42"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["gid"], "task-42");
    assert_eq!(value["stories"].as_array().unwrap().len(), 1);
    assert_eq!(value["stories"][0]["text"], "Human clarification");
    let requests = server.join().unwrap();
    assert!(requests[0].head.starts_with("GET /tasks/task-42?"));
    assert!(requests[1].head.starts_with("GET /tasks/task-42/stories?"));
}

#[test]
fn asana_client_rejects_mutation_outside_the_configured_project() {
    let (api_base, server) = serve(vec![Response {
        status: "200 OK",
        body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"other-project"}}]}}"#,
        headers: &[],
    }]);
    let output = run_client(&api_base, &["update", "task-42", "--completed", "true"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("does not belong to ASANA_PROJECT_GID project-1"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].head.starts_with("GET /tasks/task-42?"));
}

#[test]
fn asana_adapter_preserves_rate_limit_metadata_without_exposing_the_token() {
    let (api_base, server) = serve(vec![Response {
        status: "429 Too Many Requests",
        body: r#"{"errors":[{"message":"try later"}]}"#,
        headers: &[("Retry-After", "60")],
    }]);
    let mut command = Command::new(adapter());
    command.args(["--state", "Ready"]);
    configured(&mut command, &api_base);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["kind"], "rate_limited");
    assert!(value["error"]["retry_at"].as_str().unwrap().ends_with('Z'));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-secret-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-secret-token"));
    server.join().unwrap();
}

#[test]
fn asana_client_never_forwards_the_bearer_token_through_a_redirect() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_url = format!("http://{}/stolen", target.local_addr().unwrap());
    let target_handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            match target.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{{\"data\":{{}}}}"
                    )
                    .unwrap();
                    return Some(request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("redirect target failed: {error}"),
            }
        }
        None
    });

    let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
    let api_base = format!("http://{}", redirect.local_addr().unwrap());
    let redirect_handle = thread::spawn(move || {
        let (mut stream, _) = redirect.accept().unwrap();
        let request = read_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        request
    });

    let output = run_client(&api_base, &["get", "task-42"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("HTTP 302"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_request = redirect_handle.join().unwrap();
    assert!(
        first_request
            .head
            .contains("Authorization: Bearer test-secret-token")
    );
    assert!(
        target_handle.join().unwrap().is_none(),
        "redirect target received the bearer token"
    );
}

#[test]
fn asana_client_refuses_to_send_credentials_over_non_loopback_http() {
    let output = Command::new(client())
        .args(["get", "task-42"])
        .env("ASANA_ACCESS_TOKEN", "test-secret-token")
        .env("ASANA_API_BASE_URL", "http://example.com/api/1.0")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("official HTTPS Asana endpoint"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-secret-token"));
}
