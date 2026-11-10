use axum::{Json, Router, extract::State, routing::post};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(project: &Path, vault: &Path, port: u16) -> Server {
    Server(
        Command::new(env!("CARGO_BIN_EXE_baton-server"))
            .env("BATON_PROJECT_DIR", project)
            .env("BATON_CREDENTIAL_DIR", vault)
            .env("BATON_PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}

#[derive(Clone)]
struct Provider {
    requests: Arc<AtomicUsize>,
    received: mpsc::UnboundedSender<Value>,
}

async fn chat(
    State(provider): State<Provider>,
    Json(request): Json<Value>,
) -> ([(axum::http::header::HeaderName, &'static str); 1], String) {
    let attempt = provider.requests.fetch_add(1, Ordering::SeqCst);
    let request_text = request.to_string();
    provider.received.send(request).unwrap();
    if attempt == 0 {
        std::future::pending::<()>().await;
    }
    let chunk = if attempt == 1 {
        let suffix = request_text
            .split(".baton/work/")
            .nth(1)
            .expect("run memory path");
        let id = suffix.split(".md").next().unwrap();
        let arguments =
            json!({"path":format!(".baton/work/{id}.md"),"offset":null,"limit":null}).to_string();
        json!({"id":"fake", "object":"chat.completion.chunk", "created":1, "model":"fake", "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_memory","type":"function","function":{"name":"read","arguments":arguments}}]},"finish_reason":"tool_calls"}]})
    } else {
        json!({"id":"fake", "object":"chat.completion.chunk", "created":1, "model":"fake", "choices":[{"index":0,"delta":{"role":"assistant","content":"Work complete"},"finish_reason":"stop"}]})
    };
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
    )
}

async fn wait_until<F, Fut>(mut predicate: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if predicate().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("timed out waiting for server state");
}

fn git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_resumes_interrupted_rig_run_after_server_restart() {
    let temp = tempfile::tempdir().unwrap();
    let project: PathBuf = temp.path().join("project");
    std::fs::create_dir_all(project.join(".baton")).unwrap();
    git(&project, &["init", "-q"]);
    git(
        &project,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    std::fs::write(project.join(".baton/config.json"), json!({
        "schemaVersion":7,
        "agents":[{"id":"worker", "systemPrompt":"Complete the task", "runtimeKind":"rig", "model":{"provider":"ollama", "model":"fake"}, "credentialId":"fake", "skillIds":[]}],
        "pipelines":[{"id":"factory", "steps":[
            {"id":"build", "name":"Build", "agentId":"worker"},
            {"id":"approval", "name":"Human approval", "human":true}
        ]}]
    }).to_string()).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_address = listener.local_addr().unwrap();
    let (sender, mut received) = mpsc::unbounded_channel();
    let provider = Provider {
        requests: Arc::new(AtomicUsize::new(0)),
        received: sender,
    };
    let provider_task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(chat))
                .fallback(
                    |request: axum::http::Request<axum::body::Body>| async move {
                        eprintln!(
                            "Unexpected provider request: {} {}",
                            request.method(),
                            request.uri()
                        );
                        axum::http::StatusCode::NOT_FOUND
                    },
                )
                .with_state(provider),
        )
        .await
        .unwrap();
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let vault = temp.path().join("vault");
    let mut server = start_server(&project, &vault, port);
    wait_until(|| async { client.get(format!("{base}/api/runs")).send().await.is_ok() }).await;

    let response = client.put(format!("{base}/api/credentials/fake")).json(&json!({
        "name":"Fake", "credentialType":"ollama_api_key", "configuration":{"baseUrl":format!("http://{provider_address}")}, "secret":"fake"
    })).send().await.unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    let task: Value = client
        .post(format!("{base}/api/tasks"))
        .json(&json!({
            "title":"Restart test", "description":"Finish after restart", "pipelineId":"factory"
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let task_id = task["id"].as_str().unwrap().to_owned();
    let start_client = client.clone();
    let start_url = format!("{base}/api/tasks/{task_id}/runs");
    let original_request = tokio::spawn(async move { start_client.post(start_url).send().await });
    if tokio::time::timeout(Duration::from_secs(10), received.recv())
        .await
        .is_err()
    {
        let runs = client
            .get(format!("{base}/api/runs"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        panic!("provider was not called; runs: {runs}");
    }
    let runs: Vec<Value> = client
        .get(format!("{base}/api/runs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run_id = runs[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(runs[0]["status"], "running");
    let worktree = runs[0]["worktree"]["path"].as_str().unwrap().to_owned();
    std::fs::write(Path::new(&worktree).join("preserved.txt"), "keep").unwrap();
    std::fs::write(
        Path::new(&worktree).join(format!(".baton/work/{run_id}.md")),
        "Memory: continue parser integration",
    )
    .unwrap();

    server.0.kill().unwrap();
    server.0.wait().unwrap();
    drop(server);
    let _server = start_server(&project, &vault, port);
    wait_until(|| async { client.get(format!("{base}/api/runs")).send().await.is_ok() }).await;
    let resumed_request = match tokio::time::timeout(Duration::from_secs(10), received.recv()).await
    {
        Ok(Some(request)) => request,
        other => {
            let body = client
                .get(format!("{base}/api/runs/{run_id}"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            panic!("No resumed request: {other:?}; run: {body}");
        }
    };
    assert!(
        resumed_request
            .to_string()
            .contains("do not repeat side effects blindly")
    );
    let after_read = tokio::time::timeout(Duration::from_secs(10), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        after_read
            .to_string()
            .contains("Memory: continue parser integration")
    );
    let run_url = format!("{base}/api/runs/{run_id}");
    wait_until(|| async {
        match client.get(&run_url).send().await {
            Ok(response) => response
                .json::<Value>()
                .await
                .is_ok_and(|run| run["status"] == "waitingHuman"),
            Err(_) => false,
        }
    })
    .await;
    let run: Value = client
        .get(&run_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(run["id"], run_id);
    assert_eq!(run["worktree"]["path"], worktree);
    assert_eq!(
        std::fs::read_to_string(Path::new(&worktree).join("preserved.txt")).unwrap(),
        "keep"
    );
    assert_eq!(run["steps"][0]["status"], "failed");
    assert_eq!(run["steps"][1]["status"], "waitingHuman");
    assert!(
        run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["stepId"] == "build"
                && step["attempt"] == 2
                && step["status"] == "succeeded")
    );
    let completed: Value = client
        .post(format!("{run_url}/actions"))
        .json(&json!({"outcome":"approved", "response":"Approved by test"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(completed["status"], "succeeded");
    original_request.abort();
    provider_task.abort();
}
