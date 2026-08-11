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
        let mut responses = responses.into_iter().peekable();
        let mut requests = Vec::new();
        let mut initial_creation_moves_remaining = 0;
        while responses.peek().is_some() {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            if request.head.starts_with("GET /tasks/witness-") {
                let project = if request
                    .head
                    .starts_with("GET /tasks/witness-project-mismatch")
                {
                    "other-project"
                } else {
                    "project-1"
                };
                let section = if request
                    .head
                    .starts_with("GET /tasks/witness-section-mismatch")
                {
                    "other-section"
                } else if request.head.starts_with("GET /tasks/witness-ready") {
                    "section-ready"
                } else {
                    "section-backlog"
                };
                let body = format!(
                    r#"{{"data":{{"gid":"witness","memberships":[{{"project":{{"gid":"{project}"}},"section":{{"gid":"{section}"}}}}]}}}}"#
                );
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                let is_mismatch = request
                    .head
                    .starts_with("GET /tasks/witness-project-mismatch")
                    || request
                        .head
                        .starts_with("GET /tasks/witness-section-mismatch");
                requests.push(request);
                if is_mismatch {
                    break;
                }
                continue;
            }
            if request
                .head
                .starts_with("POST /sections/section-backlog/addTask ")
                && initial_creation_moves_remaining > 0
            {
                let body = r#"{"data":{}}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                requests.push(request);
                initial_creation_moves_remaining -= 1;
                continue;
            }
            while responses.peek().is_some_and(|response| {
                (response.body.contains(r#""name":"Backlog""#)
                    || response.body.contains(r#""name":"Ready For Spec""#))
                    && !request.head.starts_with("GET /projects/")
            }) {
                responses.next();
            }
            let response = responses.next().expect("unexpected request");
            if response.status == "201 Created" {
                initial_creation_moves_remaining += 1;
            }
            if response.status != "DISCONNECT" {
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
            }
            requests.push(request);
        }
        requests
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
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".flashy-factory/sources/asana")
}

fn client() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".flashy-factory/clients/asana")
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

fn run_batch(api_base: &str, mut manifest: serde_json::Value) -> Output {
    manifest["batch_creation_id"] = serde_json::json!("test-batch");
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("batch.json");
    fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (token_info_url, token_server) = serve(vec![Response {
        status: "200 OK",
        body: r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write projects:read tags:read","client_id":1217184666380172}"#,
        headers: &[],
    }]);
    let mut command = Command::new(client());
    command.args(["batch-create", "--input", input.to_str().unwrap()]);
    configured(&mut command, api_base);
    command
        .env("ASANA_AUTH_MODE", "oauth")
        .env("ASANA_OAUTH_ACCESS_TOKEN", "test-oauth-secret")
        .env("ASANA_OAUTH_CLIENT_ID", "1217184666380172")
        .env("ASANA_BACKLOG_SECTION_GID", "section-backlog")
        .env("ASANA_BACKLOG_SECTION_WITNESS_TASK_GID", "witness-backlog")
        .env("ASANA_READY_FOR_SPEC_SECTION_GID", "section-ready")
        .env(
            "ASANA_READY_FOR_SPEC_SECTION_WITNESS_TASK_GID",
            "witness-ready",
        )
        .env(
            "ASANA_TOKEN_INFO_URL",
            format!("{token_info_url}/-/token_info"),
        );
    let output = command.output().unwrap();
    let requests = token_server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].head.starts_with("POST /-/token_info "));
    assert_eq!(requests[0].body, "token=test-oauth-secret");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-oauth-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-oauth-secret"));
    output
}

fn run_invalid_batch(mut manifest: serde_json::Value) -> Output {
    manifest["batch_creation_id"] = serde_json::json!("test-batch");
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("batch.json");
    fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut command = Command::new(client());
    command.args(["batch-create", "--input", input.to_str().unwrap()]);
    command
        .env("ASANA_AUTH_MODE", "oauth")
        .env("ASANA_OAUTH_ACCESS_TOKEN", "test-oauth-secret")
        .env("ASANA_OAUTH_CLIENT_ID", "oauth-client")
        .env("ASANA_PROJECT_GID", "project-1")
        .env("ASANA_WORKSPACE_GID", "workspace-1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    command.output().unwrap()
}

fn missing_external_task() -> Response {
    Response {
        status: "404 Not Found",
        body: r#"{"errors":[{"message":"not found"}]}"#,
        headers: &[],
    }
}

const EXTERNAL_TASK_A_BACKLOG: &str = r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"e3ff484a2539a167515a7e19c900b5d224d02b136dbc0565869ee1c3cc4433cf\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"custom_fields":[]}}"#;

const EXTERNAL_TASK_A_READY: &str = r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"e3ff484a2539a167515a7e19c900b5d224d02b136dbc0565869ee1c3cc4433cf\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"custom_fields":[]}}"#;

const EXTERNAL_TASK_A_CHAIN: &str = r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"0876b7b11dd6e6c495c8c293748422e34e0781aab9841bbd9ac1de7f0a700215\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"custom_fields":[]}}"#;

const EXTERNAL_TASK_A_INDEPENDENT_TWO: &str = r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"18145784629476ebf30fac250814ab48cf96d7358d004289c0664eeee9581e29\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"custom_fields":[]}}"#;

fn task(reference: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"ref": reference, "name": name})
}

fn computed_batch_hash(manifest: serde_json::Value) -> String {
    let program = r#"import json,runpy,sys
module=runpy.run_path(sys.argv[1])
manifest=json.loads(sys.argv[2])
policy,tasks,edges=module['parse_batch_manifest'](manifest)
print(module['batch_definition_hash'](policy,tasks,edges))"#;
    let client_path = client();
    let manifest_json = manifest.to_string();
    let output = Command::new("python3")
        .args(["-c", program, client_path.to_str().unwrap(), &manifest_json])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
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
    assert!(
        requests[1]
            .head
            .starts_with("GET /sections/section-1/tasks?")
    );
    assert!(!requests[1].head.contains("project="));
    assert!(!requests[1].head.contains("section="));
    assert_eq!(
        requests.len(),
        2,
        "manual/non-autonomous polling does not read dependencies"
    );
    assert!(requests.iter().all(|request| {
        request
            .head
            .contains("Authorization: Bearer test-secret-token")
    }));
}

fn run_waiting_adapter(api_base: &str) -> Output {
    let mut command = Command::new(adapter());
    command.args([
        "--state",
        "Approved - Waiting On Dependencies",
        "--label",
        "factory:auto-to-pr",
    ]);
    configured(&mut command, api_base);
    command.output().unwrap()
}

#[test]
fn waiting_autonomous_observations_reconcile_completion_changes_without_repeating_unchanged_work() {
    let waiting_task = r#"{"data":[{"gid":"task-waiting","name":"Wait","notes":"Spec","completed":false,"modified_at":"2026-08-05T12:00:00.000Z","tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"}]}],"next_page":null}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: waiting_task,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"blocker","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: waiting_task,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"blocker","completed":true,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
    ]);
    let blocked = run_waiting_adapter(&api_base);
    let released = run_waiting_adapter(&api_base);
    assert!(blocked.status.success());
    assert!(released.status.success());
    let blocked: serde_json::Value = serde_json::from_slice(&blocked.stdout).unwrap();
    let released: serde_json::Value = serde_json::from_slice(&released.stdout).unwrap();
    assert_eq!(
        blocked["issues"][0]["dependencies"],
        serde_json::json!(["blocker"])
    );
    assert_ne!(
        blocked["issues"][0]["revision"],
        released["issues"][0]["revision"]
    );
    assert_ne!(
        blocked["issues"][0]["dependency_revision"],
        released["issues"][0]["dependency_revision"]
    );
    server.join().unwrap();
}

#[test]
fn waiting_reconciliation_revision_ignores_comment_only_modified_at_changes() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-waiting","name":"Wait","completed":false,"modified_at":"2026-08-05T12:00:00.000Z","tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"}]}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"blocker","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-waiting","name":"Wait","completed":false,"modified_at":"2026-08-05T12:01:00.000Z","tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"}]}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"blocker","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
    ]);
    let first = run_waiting_adapter(&api_base);
    let second = run_waiting_adapter(&api_base);
    assert!(first.status.success());
    assert!(second.status.success());
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        first["issues"][0]["revision"],
        second["issues"][0]["revision"]
    );
    server.join().unwrap();
}

#[test]
fn waiting_reconciliation_keeps_manual_and_unsafe_graphs_out_of_the_implementation_lane() {
    let manual = r#"{"data":[{"gid":"manual","name":"Manual","completed":false,"tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}]}],"next_page":null}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: manual,
            headers: &[],
        },
    ]);
    let output = run_waiting_adapter(&api_base);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["issues"],
        serde_json::json!([])
    );
    assert_eq!(
        server.join().unwrap().len(),
        2,
        "manual work is never revalidated"
    );

    for unsafe_root in [
        r#"{"data":{"gid":"task-unsafe","completed":false,"dependencies":"malformed","memberships":[{"project":{"gid":"project-1"}}]}}"#,
        r#"{"data":{"gid":"task-unsafe","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}},{"project":{"gid":"other-project"}}]}}"#,
        r#"{"errors":[{"message":"not found"}]}"#,
    ] {
        let (api_base, server) = serve(vec![
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"task-unsafe","name":"Unsafe","completed":false,"modified_at":"2026-08-05T12:00:00.000Z","tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"}]}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: if unsafe_root.contains("errors") {
                    "404 Not Found"
                } else {
                    "200 OK"
                },
                body: unsafe_root,
                headers: &[],
            },
        ]);
        let output = run_waiting_adapter(&api_base);
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["issues"].as_array().unwrap().len(), 1);
        assert!(
            value["issues"][0]["revision"]
                .as_str()
                .unwrap()
                .contains(":unsafe:")
        );
        server.join().unwrap();
    }
}

#[test]
fn waiting_reconciliation_observes_cyclic_dependencies_as_a_decision_boundary() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-cycle","name":"Cycle","completed":false,"modified_at":"2026-08-05T12:00:00.000Z","tags":[{"gid":"tag-auto","name":"factory:auto-to-pr"}]}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-cycle","completed":false,"dependencies":[{"gid":"blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"blocker","completed":false,"dependencies":[{"gid":"task-cycle"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_waiting_adapter(&api_base);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["issues"][0]["revision"]
            .as_str()
            .unwrap()
            .contains(":unsafe:")
    );
    server.join().unwrap();
}

#[test]
fn dependency_state_routes_unblocked_and_blocked_tasks_and_revalidates_changes() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-free","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"task-blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-blocker","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-waiting","completed":false,"dependencies":[{"gid":"task-blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-blocker","completed":true,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
    ]);
    let free = run_client(&api_base, &["dependency-state", "task-free"]);
    let blocked = run_client(&api_base, &["dependency-state", "task-waiting"]);
    let released = run_client(&api_base, &["dependency-state", "task-waiting"]);
    for output in [&free, &blocked, &released] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let free: serde_json::Value = serde_json::from_slice(&free.stdout).unwrap();
    let blocked: serde_json::Value = serde_json::from_slice(&blocked.stdout).unwrap();
    let released: serde_json::Value = serde_json::from_slice(&released.stdout).unwrap();
    assert_eq!(free["dependencies"], serde_json::json!([]));
    assert_eq!(blocked["dependencies"], serde_json::json!(["task-blocker"]));
    assert_eq!(blocked["blocked"], true);
    assert_eq!(released["blocked"], false);
    assert_ne!(
        blocked["dependency_revision"],
        released["dependency_revision"]
    );
    server.join().unwrap();
}

#[test]
fn dependency_state_fails_closed_for_malformed_cyclic_and_cross_project_graphs() {
    let cases = [
        r#"{"data":{"gid":"task-42","completed":false,"dependencies":"invalid","memberships":[{"project":{"gid":"project-1"}}]}}"#,
        r#"{"data":{"gid":"task-42","completed":false,"dependencies":[{"gid":"task-other"}],"memberships":[{"project":{"gid":"project-1"}},{"project":{"gid":"other-project"}}]}}"#,
    ];
    for body in cases {
        let (api_base, server) = serve(vec![Response {
            status: "200 OK",
            body,
            headers: &[],
        }]);
        let output = run_client(&api_base, &["dependency-state", "task-42"]);
        assert!(!output.status.success());
        server.join().unwrap();
    }
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","completed":false,"dependencies":[{"gid":"task-43"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-43","completed":false,"dependencies":[{"gid":"task-42"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_client(&api_base, &["dependency-state", "task-42"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cycle"));
    server.join().unwrap();
}

#[test]
fn terminal_pr_reconciliation_completes_merged_manual_and_routes_contradictory_tasks() {
    let parent = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:manual"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: parent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let manual = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "merged"],
    );
    assert!(
        manual.status.success(),
        "{}",
        String::from_utf8_lossy(&manual.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&manual.stdout).trim(),
        r#"{"status":"merged","released":[]}"#
    );
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("POST /sections/done/addTask"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("PUT /tasks/task-42"))
    );

    let (api_base, server) = serve(vec![Response {
        status: "200 OK",
        body: parent,
        headers: &[],
    }]);
    let closed_manual = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "closed"],
    );
    assert!(closed_manual.status.success());
    assert_eq!(
        String::from_utf8_lossy(&closed_manual.stdout).trim(),
        r#"{"status":"manual_untouched","released":[]}"#
    );
    assert_eq!(server.join().unwrap().len(), 1);

    let contradictory = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:manual"},{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: contradictory,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"decision","name":"Needs Decision"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let unsafe_task = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "unsafe"],
    );
    assert!(
        unsafe_task.status.success(),
        "{}",
        String::from_utf8_lossy(&unsafe_task.stderr)
    );
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("POST /sections/decision/addTask"))
    );
}

#[test]
fn merged_pr_completion_and_eligible_dependent_release_are_atomic_after_reads() {
    let parent = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let dependent = r#"{"data":{"gid":"task-43","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let dependency_state = r#"{"data":{"gid":"task-43","completed":false,"dependencies":[{"gid":"task-42"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#;
    let terminal_parent_dependency = r#"{"data":{"gid":"task-42","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: parent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-43"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: dependent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: dependency_state,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: terminal_parent_dependency,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"},{"gid":"ready","name":"Ready To Implement"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"},{"gid":"ready","name":"Ready To Implement"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let output = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "merged"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"{"status":"merged","released":["task-43"]}"#
    );
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("POST /sections/done/addTask"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("PUT /tasks/task-42"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("POST /sections/ready/addTask"))
    );
}

#[test]
fn unsafe_dependent_routes_to_decision_and_ineligible_work_is_untouched() {
    let parent = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let unsafe_dependent = r#"{"data":{"gid":"task-43","completed":false,"tags":[{"name":"factory:auto-to-pr"},{"name":"factory:manual"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: parent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-43"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: unsafe_dependent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"},{"gid":"decision","name":"Needs Decision"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"},{"gid":"decision","name":"Needs Decision"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let output = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "merged"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.head.starts_with("POST /sections/decision/addTask"))
    );

    let (api_base, server) = serve(vec![Response {
        status: "200 OK",
        body: r#"{"data":{"gid":"task-42","completed":true,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Done"}}]}}"#,
        headers: &[],
    }]);
    let output = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "closed"],
    );
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"{"status":"completed_untouched","released":[]}"#
    );
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn dependency_read_failure_leaves_parent_and_dependents_unmodified() {
    let parent = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let dependent = r#"{"data":{"gid":"task-43","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: parent,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-43"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: dependent,
            headers: &[],
        },
        Response {
            status: "DISCONNECT",
            body: "",
            headers: &[],
        },
    ]);
    let output = run_client(
        &api_base,
        &["reconcile-pr", "task-42", "--outcome", "merged"],
    );
    assert!(!output.status.success());
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.head.starts_with("POST ") && !request.head.starts_with("PUT "))
    );
}

#[test]
fn merged_pr_replay_observes_live_state_without_duplicate_asana_mutations() {
    let waiting = r#"{"data":{"gid":"task-42","completed":false,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Approved - Waiting On Dependencies"}}]}}"#;
    let completed = r#"{"data":{"gid":"task-42","completed":true,"tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Done"}}]}}"#;
    let no_dependents = r#"{"data":[],"next_page":null}"#;
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: waiting,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: no_dependents,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"done","name":"Done"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: completed,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: no_dependents,
            headers: &[],
        },
    ]);
    for _ in 0..2 {
        let output = run_client(
            &api_base,
            &["reconcile-pr", "task-42", "--outcome", "merged"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let requests = server.join().unwrap();
    let mutations = requests
        .iter()
        .filter(|request| request.head.starts_with("POST ") || request.head.starts_with("PUT "))
        .collect::<Vec<_>>();
    assert_eq!(mutations.len(), 2);
    assert!(mutations[0].head.starts_with("POST /sections/done/addTask"));
    assert!(mutations[1].head.starts_with("PUT /tasks/task-42"));
}

#[test]
fn autonomous_workflows_route_decisions_and_preserve_manual_approval() {
    let triage = include_str!("../.flashy-factory/workflows/triage.md");
    let implement = include_str!("../.flashy-factory/workflows/implement.md");
    let reconcile = include_str!("../.flashy-factory/workflows/reconcile-dependencies.md");
    assert!(triage.contains("Approved - Waiting On\nDependencies"));
    assert!(triage.contains("Needs Decision"));
    assert!(triage.contains("factory:manual"));
    assert!(triage.contains("Awaiting Approval"));
    assert!(triage.contains("dependency-review"));
    assert!(triage.contains("apply-spec-approval"));
    assert!(triage.contains("advisory only"));
    assert!(implement.contains("dependency-state"));
    assert!(implement.contains("Needs Decision"));
    assert!(reconcile.contains("Approved - Waiting On Dependencies"));
    assert!(reconcile.contains("Ready To Implement"));
    assert!(reconcile.contains("Needs Decision"));
    assert!(reconcile.contains("factory:manual"));
    assert!(reconcile.contains("Epic custom"));
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
fn dependency_review_is_advisory_and_ranks_planned_work_with_rationale() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-current","name":"Add payment schema","notes":"Create payment API schema","memberships":[{"project":{"gid":"project-1"},"section":{"name":"Creating Spec"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"task-schema","name":"Create payment API schema","notes":"Shared payment contract","completed":false,"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Ready To Implement"}}]},{"gid":"task-unrelated","name":"Update dashboard colors","notes":"Visual refresh","completed":false,"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Implementing"}}]}],"next_page":null}"#,
            headers: &[],
        },
    ]);
    let output = run_client(&api_base, &["dependency-review", "task-current"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let review: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(review["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(review["candidates"][0]["gid"], "task-schema");
    assert_eq!(review["candidates"][0]["confidence"], "high");
    assert!(
        review["candidates"][0]["rationale"]
            .as_str()
            .unwrap()
            .contains("payment")
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| !request.head.starts_with("POST "))
    );
}

#[test]
fn spec_approval_writes_only_confirmed_native_dependencies_before_routing() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","tags":[{"name":"factory:auto-to-pr"}],"memberships":[{"project":{"gid":"project-1"},"section":{"name":"Awaiting Approval"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-blocker","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-blocker","dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","completed":false,"dependencies":[{"gid":"task-blocker"}],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-blocker","completed":false,"dependencies":[],"memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-42","memberships":[{"project":{"gid":"project-1"}}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"waiting","name":"Approved - Waiting On Dependencies"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
    ]);
    let temp = tempfile::tempdir().unwrap();
    let approval = temp.path().join("approval.json");
    fs::write(&approval, r#"{"confirmed_dependencies":["task-blocker"]}"#).unwrap();
    let output = run_client(
        &api_base,
        &[
            "apply-spec-approval",
            "task-42",
            "--input",
            approval.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["added_dependencies"],
        serde_json::json!(["task-blocker"])
    );
    assert_eq!(result["destination"], "Approved - Waiting On Dependencies");
    let requests = server.join().unwrap();
    let mutations = requests
        .iter()
        .filter(|request| request.head.starts_with("POST "))
        .collect::<Vec<_>>();
    assert_eq!(mutations.len(), 2);
    assert!(
        mutations[0]
            .head
            .starts_with("POST /tasks/task-42/addDependencies ")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&mutations[0].body).unwrap(),
        serde_json::json!({"data": {"dependencies": ["task-blocker"]}})
    );
    assert!(
        mutations[1]
            .head
            .starts_with("POST /sections/waiting/addTask ")
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
            body: r#"{"data":{"gid":"task-42","name":"Fix it","notes":"Acceptance criteria","custom_fields":[{"gid":"priority","name":"Priority","display_value":"High"}],"memberships":[{"project":{"gid":"project-1","name":"Flashy Factory"},"section":{"gid":"ready","name":"Ready To Implement"}}]}}"#,
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
    assert_eq!(value["data"]["custom_fields"][0]["display_value"], "High");
    assert_eq!(value["stories"].as_array().unwrap().len(), 1);
    assert_eq!(value["stories"][0]["text"], "Human clarification");
    let requests = server.join().unwrap();
    assert!(requests[0].head.starts_with("GET /tasks/task-42?"));
    assert!(requests[0].head.contains("custom_fields.name"));
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

#[test]
fn asana_batch_creates_and_authorizes_three_independent_tasks() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-a"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-b"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-c"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-b","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-c","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "Schema"), task("b", "API"), task("c", "Docs")]
        }),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["created_tasks"].as_array().unwrap().len(), 3);
    assert!(
        report["components"]
            .as_array()
            .unwrap()
            .iter()
            .all(|component| component["status"] == "ready_for_spec")
    );

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 24);
    assert!(requests[0].head.starts_with("GET /tasks/witness-backlog?"));
    assert!(requests[1].head.starts_with("GET /tasks/witness-ready?"));
    assert!(
        !requests
            .iter()
            .any(|request| request.head.starts_with("GET /projects/project-1/sections"))
    );
    let creation_requests = requests
        .iter()
        .filter(|request| request.head.starts_with("POST /tasks "))
        .collect::<Vec<_>>();
    assert_eq!(creation_requests.len(), 3);
    for request in creation_requests {
        assert!(request.head.starts_with("POST /tasks "));
        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["data"]["projects"][0], "project-1");
    }
    assert_eq!(
        requests
            .iter()
            .filter(|request| request
                .head
                .starts_with("POST /sections/section-backlog/addTask "))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.head.contains("/addTag "))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.head.contains("/removeTag "))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request
                .head
                .starts_with("POST /sections/section-ready/addTask "))
            .count(),
        3
    );
}

#[test]
fn asana_batch_fails_closed_when_witness_configuration_is_missing_or_mismatched() {
    let manifest = serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "backlog_only",
        "dependencies": "independent",
        "tasks": [task("a", "A")]
    });
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("batch.json");
    fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();

    for (witness, expected) in [
        (
            None,
            "ASANA_BACKLOG_SECTION_WITNESS_TASK_GID must be a valid Asana GID",
        ),
        (
            Some("witness-project-mismatch"),
            "must have exactly one membership",
        ),
        (
            Some("witness-section-mismatch"),
            "must have exactly one membership",
        ),
    ] {
        let (api_base, api_server) = serve(vec![Response {
            status: "200 OK",
            body: r#"{"data":[]}"#,
            headers: &[],
        }]);
        let (token_info_url, token_server) = serve(vec![Response {
            status: "200 OK",
            body: r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write projects:read tags:read","client_id":1217184666380172}"#,
            headers: &[],
        }]);
        let mut command = Command::new(client());
        command.args(["batch-create", "--input", input.to_str().unwrap()]);
        configured(&mut command, &api_base);
        command
            .env("ASANA_AUTH_MODE", "oauth")
            .env("ASANA_OAUTH_ACCESS_TOKEN", "test-oauth-secret")
            .env("ASANA_OAUTH_CLIENT_ID", "1217184666380172")
            .env(
                "ASANA_TOKEN_INFO_URL",
                format!("{token_info_url}/-/token_info"),
            )
            .env("ASANA_BACKLOG_SECTION_GID", "section-backlog");
        if let Some(witness) = witness {
            command.env("ASANA_BACKLOG_SECTION_WITNESS_TASK_GID", witness);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        if witness.is_some() {
            let requests = api_server.join().unwrap();
            assert!(
                !requests
                    .iter()
                    .any(|request| request.head.starts_with("POST "))
            );
        }
        assert_eq!(token_server.join().unwrap().len(), 1);
    }
}

#[test]
fn asana_batch_keeps_manual_tasks_in_backlog_with_the_manual_tag() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-manual"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-manual","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "backlog_only",
            "dependencies": "independent",
            "tasks": [task("manual", "Review manually")]
        }),
    );
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["components"][0]["status"], "manual_backlog");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 8);
    assert!(
        requests[6]
            .head
            .starts_with("POST /tasks/task-manual/addTag ")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[6].body).unwrap(),
        serde_json::json!({"data":{"tag":"tag-manual"}})
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request
                .head
                .starts_with("POST /sections/section-backlog/addTask "))
            .count(),
        1
    );
}

#[test]
fn asana_batch_writes_then_verifies_a_listed_chain() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-api"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-schema"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api","memberships":[{"project":{"gid":"project-1"}}],"dependencies":[{"gid":"task-schema"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-schema","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "listed_chain",
            "tasks": [task("schema", "Schema"), task("api", "API")]
        }),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["verified_edges"][0]["edge"], "api -> schema");
    assert_eq!(report["missing_edges"], serde_json::json!([]));
    let requests = server.join().unwrap();
    assert!(
        requests[9]
            .head
            .starts_with("POST /tasks/task-api/addDependencies ")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&requests[9].body).unwrap(),
        serde_json::json!({"data":{"dependencies":["task-schema"]}})
    );
    assert!(requests[10].head.starts_with("GET /tasks/task-api?"));
    assert!(requests[10].head.contains("dependencies.gid"));
}

#[test]
fn asana_batch_partial_creation_leaves_every_created_task_in_backlog() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-api"}}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"create failed"}]}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": {
                "mode": "explicit_edges",
                "edges": [{"dependent": "api", "blocker": "schema"}]
            },
            "tasks": [task("schema", "Schema"), task("api", "API")]
        }),
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["created_tasks"][0]["ref"], "api");
    assert_eq!(report["created_tasks"][0]["gid"], "task-api");
    assert_eq!(report["failed_tasks"][0]["ref"], "schema");
    assert_eq!(
        report["unreconciled_external_identities"][0]["external_gid"],
        "flashy-factory:test-batch:task:schema"
    );
    assert_eq!(
        report["missing_edges"],
        serde_json::json!(["api -> schema"])
    );
    let requests = server.join().unwrap();
    assert_eq!(
        requests.len(),
        15,
        "partial creation must stop before wiring and verify manual fallback"
    );
    assert_eq!(
        report["components"][0]["status"],
        "manual_backlog_partial_create"
    );
    assert!(
        requests[13]
            .head
            .starts_with("POST /tasks/task-api/addTag ")
    );
    assert!(requests[5].head.starts_with("POST /tasks "));
    let created_anchor: serde_json::Value = serde_json::from_str(&requests[5].body).unwrap();
    assert_eq!(
        created_anchor["data"]["external"]["gid"],
        "flashy-factory:test-batch:batch"
    );
}

#[test]
fn asana_batch_partial_edge_failure_authorizes_only_unaffected_components() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-schema"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-api"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-docs"}}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"edge failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api","memberships":[{"project":{"gid":"project-1"}}],"dependencies":[]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-schema","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-api","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-docs","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": {
                "mode": "explicit_edges",
                "edges": ["api -> schema"]
            },
            "tasks": [task("schema", "Schema"), task("api", "API"), task("docs", "Docs")]
        }),
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["missing_edges"][0]["edge"], "api -> schema");
    assert_eq!(
        report["components"][0]["status"],
        "manual_backlog_unverified"
    );
    assert_eq!(report["components"][1]["status"], "ready_for_spec");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 26);
    assert!(
        requests[23]
            .head
            .starts_with("POST /tasks/task-docs/addTag ")
    );
    assert!(
        requests[24]
            .head
            .starts_with("POST /sections/section-ready/addTask ")
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| {
                (request.head.contains("task-schema/addTag")
                    || request.head.contains("task-api/addTag"))
                    && request.body.contains("tag-manual")
            })
            .count(),
        2
    );
    assert!(!requests.iter().any(|request| {
        request
            .head
            .starts_with("POST /sections/section-ready/addTask ")
            && (request.body.contains("task-schema") || request.body.contains("task-api"))
    }));
}

#[test]
fn asana_batch_rolls_a_failed_authorization_back_to_backlog() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-a"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"move failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "A")]
        }),
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["components"][0]["status"],
        "backlog_authorization_failed"
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 13);
    assert!(
        requests[9]
            .head
            .starts_with("POST /sections/section-backlog/addTask ")
    );
    assert!(
        requests[10]
            .head
            .starts_with("POST /tasks/task-a/removeTag ")
    );
}

#[test]
fn asana_batch_detects_dual_tags_and_verifies_the_safe_downgrade() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-a"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"},{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "A")]
        }),
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["components"][0]["status"],
        "backlog_authorization_failed"
    );
    assert_eq!(
        report["components"][0]["unsafe_task_gids"],
        serde_json::json!([])
    );
    assert!(
        report["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| {
                failure["operation"] == "verify_authorization:a"
                    && failure["error"].as_str().unwrap().contains("tag-manual")
            })
    );
    assert_eq!(server.join().unwrap().len(), 14);
}

#[test]
fn asana_batch_reports_exact_unsafe_gids_when_multitask_rollback_cannot_be_verified() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        missing_external_task(),
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-a"}}"#,
            headers: &[],
        },
        Response {
            status: "201 Created",
            body: r#"{"data":{"gid":"task-b"}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-b","memberships":[{"project":{"gid":"project-1"}}],"dependencies":[{"gid":"task-a"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"second move failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"rollback section failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"rollback auto tag failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"rollback manual tag failed"}]}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"tags":[{"gid":"tag-manual"}]}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-b","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "listed_chain",
            "tasks": [task("a", "A"), task("b", "B")]
        }),
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["components"][0]["status"],
        "authorization_state_unsafe"
    );
    assert_eq!(
        report["components"][0]["unsafe_task_gids"],
        serde_json::json!(["task-b"])
    );
    assert_eq!(server.join().unwrap().len(), 25);
}

#[test]
fn asana_batch_recovers_an_accepted_create_after_the_connection_drops() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        missing_external_task(),
        Response {
            status: "DISCONNECT",
            body: "",
            headers: &[],
        },
        Response {
            status: "500 Internal Server Error",
            body: r#"{"errors":[{"message":"external lookup not ready"}]}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: EXTERNAL_TASK_A_BACKLOG,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "A")]
        }),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["created_tasks"][0]["source"], "recovered");
    assert_eq!(report["creation_recoveries"][0]["gid"], "task-a");
    let requests = server.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.head.starts_with("POST /tasks "))
            .count(),
        1,
        "an ambiguous create must never be retried"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.head.starts_with("GET /tasks/external:"))
            .count(),
        3
    );
}

#[test]
fn asana_batch_reentry_reuses_exact_identity_and_rejects_task_or_content_changes() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: EXTERNAL_TASK_A_READY,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{}}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":{"gid":"task-a","memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-ready"}}],"tags":[{"gid":"tag-auto"}]}}"#,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "A")]
        }),
    );
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["created_tasks"][0]["source"], "reused");
    let requests = server.join().unwrap();
    assert!(
        !requests
            .iter()
            .any(|request| request.head.starts_with("POST /tasks "))
    );

    for (existing, tasks) in [
        (
            EXTERNAL_TASK_A_INDEPENDENT_TWO,
            serde_json::json!([task("a", "A")]),
        ),
        (
            EXTERNAL_TASK_A_BACKLOG,
            serde_json::json!([task("a", "Changed")]),
        ),
    ] {
        let (api_base, server) = serve(vec![
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: existing,
                headers: &[],
            },
        ]);
        let output = run_batch(
            &api_base,
            serde_json::json!({
                "delivery_policy": "autonomous_to_pr",
                "dependencies": "independent",
                "tasks": tasks
            }),
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched content"));
        assert!(
            !server
                .join()
                .unwrap()
                .iter()
                .any(|request| request.head.starts_with("POST /tasks "))
        );
    }
}

#[test]
fn asana_batch_hash_is_semantic_and_binds_task_set_and_content() {
    let listed_chain = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": "listed_chain",
        "tasks": [task("a", "A"), task("b", "B")]
    }));
    let reordered_explicit_chain = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": {
            "mode": "explicit_edges",
            "edges": [{"dependent": "b", "blocker": "a"}]
        },
        "tasks": [task("b", "B"), task("a", "A")]
    }));
    assert_eq!(listed_chain, reordered_explicit_chain);

    let edges_first_order = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": {
            "mode": "explicit_edges",
            "edges": ["c -> b", "b -> a"]
        },
        "tasks": [task("a", "A"), task("b", "B"), task("c", "C")]
    }));
    let edges_second_order = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": {
            "mode": "explicit_edges",
            "edges": ["b -> a", "c -> b"]
        },
        "tasks": [task("c", "C"), task("a", "A"), task("b", "B")]
    }));
    assert_eq!(edges_first_order, edges_second_order);

    let removed_non_anchor = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": "independent",
        "tasks": [task("a", "A")]
    }));
    let full_task_set = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": "independent",
        "tasks": [task("a", "A"), task("b", "B")]
    }));
    let changed_content = computed_batch_hash(serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "autonomous_to_pr",
        "dependencies": "independent",
        "tasks": [task("a", "Changed")]
    }));
    assert_ne!(removed_non_anchor, full_task_set);
    assert_ne!(removed_non_anchor, changed_content);
}

#[test]
fn asana_batch_reentry_rejects_changed_policy_graph_or_disjoint_task_set() {
    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: EXTERNAL_TASK_A_BACKLOG,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "backlog_only",
            "dependencies": "independent",
            "tasks": [task("a", "A")]
        }),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched content"));
    assert!(
        !server
            .join()
            .unwrap()
            .iter()
            .any(|request| request.head.starts_with("POST /tasks "))
    );

    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: EXTERNAL_TASK_A_CHAIN,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("a", "A"), task("b", "B")]
        }),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched content"));
    assert!(
        !server
            .join()
            .unwrap()
            .iter()
            .any(|request| request.head.starts_with("POST /tasks "))
    );

    let (api_base, server) = serve(vec![
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
            headers: &[],
        },
        Response {
            status: "200 OK",
            body: EXTERNAL_TASK_A_BACKLOG,
            headers: &[],
        },
    ]);
    let output = run_batch(
        &api_base,
        serde_json::json!({
            "delivery_policy": "autonomous_to_pr",
            "dependencies": "independent",
            "tasks": [task("b", "B")]
        }),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched content"));
    let requests = server.join().unwrap();
    assert!(
        requests[3]
            .head
            .starts_with("GET /tasks/external:flashy-factory%3Atest-batch%3Abatch?")
    );
    assert!(
        !requests
            .iter()
            .any(|request| request.head.starts_with("POST /tasks "))
    );
}

#[test]
fn asana_batch_rejects_mismatched_or_cross_project_external_identities_before_create() {
    for existing in [
        r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"different"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}}],"custom_fields":[]}}"#,
        r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"e3ff484a2539a167515a7e19c900b5d224d02b136dbc0565869ee1c3cc4433cf\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"other-project"},"section":{"gid":"section-backlog"}}],"custom_fields":[]}}"#,
        r#"{"data":{"gid":"task-a","name":"A","notes":"","completed":false,"external":{"gid":"flashy-factory:test-batch:batch","data":"{\"batch_creation_id\":\"test-batch\",\"batch_definition_sha256\":\"e3ff484a2539a167515a7e19c900b5d224d02b136dbc0565869ee1c3cc4433cf\",\"content_sha256\":\"a5ce28b82ad58d6612fde2268008ec295a41d57fbcf8f8ca56e26ca9e9597a66\",\"project_gid\":\"project-1\",\"section_gid\":\"section-backlog\",\"task_ref\":\"a\",\"version\":1}"},"memberships":[{"project":{"gid":"project-1"},"section":{"gid":"section-backlog"}},{"project":{"gid":"other-project"},"section":{"gid":"other-section"}}],"custom_fields":[]}}"#,
    ] {
        let (api_base, server) = serve(vec![
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"section-backlog","name":"Backlog"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"section-ready","name":"Ready For Spec"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: r#"{"data":[{"gid":"tag-auto","name":"factory:auto-to-pr"},{"gid":"tag-manual","name":"factory:manual"}],"next_page":null}"#,
                headers: &[],
            },
            Response {
                status: "200 OK",
                body: existing,
                headers: &[],
            },
        ]);
        let output = run_batch(
            &api_base,
            serde_json::json!({
                "delivery_policy": "autonomous_to_pr",
                "dependencies": "independent",
                "tasks": [task("a", "A")]
            }),
        );
        assert!(!output.status.success());
        let requests = server.join().unwrap();
        assert!(
            !requests
                .iter()
                .any(|request| request.head.starts_with("POST /tasks "))
        );
    }
}

#[test]
fn asana_batch_requires_the_configured_oauth_app_and_scopes() {
    let manifest = serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "backlog_only",
        "dependencies": "independent",
        "tasks": [task("a", "A")]
    });
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("batch.json");
    fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let mut wrong_mode = Command::new(client());
    wrong_mode.args(["batch-create", "--input", input.to_str().unwrap()]);
    wrong_mode.env("ASANA_ACCESS_TOKEN", "test-secret-token");
    let output = wrong_mode.output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("batch-create requires ASANA_AUTH_MODE=oauth")
    );

    for (token_info, expected) in [
        (
            r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write","client_id":"oauth-client"}"#,
            "missing scopes",
        ),
        (
            r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write projects:read tags:read","client_id":"other-client"}"#,
            "does not belong",
        ),
        (
            r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write projects:read tags:read custom_fields:read","client_id":"oauth-client"}"#,
            "disallowed scopes",
        ),
        (
            r#"{"active":true,"token_type":"bearer","expires_in":299,"scope":"tasks:read tasks:write projects:read tags:read","client_id":"oauth-client"}"#,
            "at least five minutes",
        ),
    ] {
        let (token_info_url, token_server) = serve(vec![Response {
            status: "200 OK",
            body: token_info,
            headers: &[],
        }]);
        let mut invalid_token = Command::new(client());
        invalid_token.args(["batch-create", "--input", input.to_str().unwrap()]);
        invalid_token
            .env("ASANA_AUTH_MODE", "oauth")
            .env("ASANA_OAUTH_ACCESS_TOKEN", "test-oauth-secret")
            .env("ASANA_OAUTH_CLIENT_ID", "oauth-client")
            .env("ASANA_PROJECT_GID", "project-1")
            .env("ASANA_WORKSPACE_GID", "workspace-1")
            .env("ASANA_ALLOW_INSECURE_LOCALHOST", "1")
            .env(
                "ASANA_TOKEN_INFO_URL",
                format!("{token_info_url}/-/token_info"),
            );
        let output = invalid_token.output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "expected {expected:?} in {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(token_server.join().unwrap().len(), 1);
    }
}

#[test]
fn asana_batch_rejects_disallowed_scopes_before_contacting_the_asana_api() {
    let manifest = serde_json::json!({
        "batch_creation_id": "test-batch",
        "delivery_policy": "backlog_only",
        "dependencies": "independent",
        "tasks": [task("a", "A")]
    });
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("batch.json");
    fs::write(&input, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let api_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    api_listener.set_nonblocking(true).unwrap();
    let api_base = format!("http://{}/api/1.0", api_listener.local_addr().unwrap());
    let (token_info_url, token_server) = serve(vec![Response {
        status: "200 OK",
        body: r#"{"active":true,"token_type":"bearer","expires_in":3600,"scope":"tasks:read tasks:write projects:read tags:read custom_fields:read","client_id":"oauth-client"}"#,
        headers: &[],
    }]);
    let mut command = Command::new(client());
    command.args(["batch-create", "--input", input.to_str().unwrap()]);
    configured(&mut command, &api_base);
    command
        .env("ASANA_AUTH_MODE", "oauth")
        .env("ASANA_OAUTH_ACCESS_TOKEN", "test-oauth-secret")
        .env("ASANA_OAUTH_CLIENT_ID", "oauth-client")
        .env(
            "ASANA_TOKEN_INFO_URL",
            format!("{token_info_url}/-/token_info"),
        );
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("disallowed scopes"));
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert_eq!(token_server.join().unwrap().len(), 1);
}

#[test]
fn asana_batch_rejects_invalid_graphs_and_batch_size_before_mutation() {
    let cases = [
        (
            serde_json::json!({
                "delivery_policy": "autonomous_to_pr",
                "dependencies": {"mode": "explicit_edges", "edges": ["missing -> a"]},
                "tasks": [task("a", "A")]
            }),
            "unknown dependent",
        ),
        (
            serde_json::json!({
                "delivery_policy": "autonomous_to_pr",
                "dependencies": {"mode": "explicit_edges", "edges": ["a -> a"]},
                "tasks": [task("a", "A")]
            }),
            "self-dependency",
        ),
        (
            serde_json::json!({
                "delivery_policy": "autonomous_to_pr",
                "dependencies": {"mode": "explicit_edges", "edges": ["a -> b", "b -> a"]},
                "tasks": [task("a", "A"), task("b", "B")]
            }),
            "contain a cycle",
        ),
        (
            serde_json::json!({
                "delivery_policy": "backlog_only",
                "dependencies": "independent",
                "tasks": (0..26).map(|index| task(&format!("t{index}"), "Task")).collect::<Vec<_>>()
            }),
            "at most 25 tasks",
        ),
    ];
    for (manifest, expected) in cases {
        let output = run_invalid_batch(manifest);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "expected {expected:?} in {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn asana_batch_rejects_custom_fields_without_requesting_a_broader_scope() {
    let output = run_invalid_batch(serde_json::json!({
        "delivery_policy": "backlog_only",
        "dependencies": "independent",
        "tasks": [{
            "ref": "a",
            "name": "A",
            "custom_fields": {"field-1": "option-1"}
        }]
    }));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("custom_fields:read"));
}
