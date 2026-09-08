use assert_cmd::cargo;
use bytes::Bytes;
use fastwebsockets::{Frame, OpCode, Payload};
use http_body_util::Empty;
use hyper::header::{CONNECTION, HOST, SEC_WEBSOCKET_PROTOCOL, UPGRADE};
use hyper::{Method, Request};
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;
use tokio::net::TcpStream;

struct WebSocketTestExecutor;

impl<Fut> hyper::rt::Executor<Fut> for WebSocketTestExecutor
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, future: Fut) {
        tokio::spawn(future);
    }
}

static ONLY_ONE_DEV_SERVER_AT_A_TIME: Mutex<()> = Mutex::new(());

fn take_the_only_dev_server_slot() -> MutexGuard<'static, ()> {
    ONLY_ONE_DEV_SERVER_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn get_forte_bin_path() -> std::path::PathBuf {
    cargo::cargo_bin!("forte").to_path_buf()
}

struct DevServer {
    child: Child,
    port: u16,
    _stdout_thread: std::thread::JoinHandle<()>,
}

impl DevServer {
    fn start(project_dir: &std::path::Path) -> Self {
        let forte_bin = get_forte_bin_path();

        let mut child = std::process::Command::new(&forte_bin)
            .args(["dev"])
            .current_dir(project_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Failed to start forte dev");

        let stdout = child.stdout.take().expect("Failed to get stdout");
        let (tx, rx) = mpsc::channel::<u16>();

        let stdout_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut sent = false;

            for line in reader.lines() {
                let Ok(line) = line else { break };
                eprintln!("[dev server] {}", line);

                if !sent
                    && line.contains("Listening on")
                    && let Some(port_str) = line.split(':').next_back()
                    && let Ok(forte_port) = port_str.trim().parse()
                {
                    let _ = tx.send(forte_port);
                    sent = true;
                }
            }
        });

        let port = match rx.recv_timeout(Duration::from_secs(600)) {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Timeout waiting for server to start: {error}");
            }
        };

        Self {
            child,
            port,
            _stdout_thread: stdout_thread,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn init_project(temp_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    cargo::cargo_bin_cmd!("forte")
        .args(["init", name])
        .current_dir(temp_dir)
        .assert()
        .success();

    temp_dir.join(name)
}

fn init_dev_project(temp_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    cargo::cargo_bin_cmd!("forte")
        .args(["init", name, "--dev"])
        .current_dir(temp_dir)
        .assert()
        .success();

    temp_dir.join(name)
}

fn install_npm_deps(project_dir: &std::path::Path) {
    std::process::Command::new("npm")
        .arg("install")
        .current_dir(project_dir.join("fe"))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to run npm install");
}

const DEV_WEBSOCKET_ROUTE: &str = r#"
use forte_sdk::anyhow::Result;
use forte_sdk::http::HeaderMap;
use forte_sdk::websocket::{ConnectDecision, ConnectEvent, DisconnectEvent, IncomingMessage, MessageEvent, WebSocketMessage};

pub async fn on_connect(_event: ConnectEvent) -> Result<ConnectDecision> {
    let mut headers = HeaderMap::new();
    headers.insert("x-app-handshake", "accepted".parse()?);
    Ok(ConnectDecision::Accept {
        protocol: Some("chat.v1".to_string()),
        headers,
    })
}

pub async fn on_message(event: MessageEvent) -> Result<()> {
    let connection_id = event.connection_id;
    let response = match event.message {
        IncomingMessage::Text(text) => WebSocketMessage::text(format!("echo:{text}")),
        IncomingMessage::Binary(bytes) => WebSocketMessage::binary(bytes),
    };
    forte_sdk::websocket::send(&connection_id, response).await?;
    Ok(())
}

pub async fn on_disconnect(_event: DisconnectEvent) -> Result<()> {
    Ok(())
}
"#;

#[test]
fn test_dev_websocket_accepts_frames_and_echoes() {
    let _dev_server_slot = take_the_only_dev_server_slot();
    let temp = tempfile::tempdir().unwrap();
    let project_dir = init_dev_project(temp.path(), "test-app-websocket");

    install_npm_deps(&project_dir);
    let websocket_dir = project_dir.join("rs/src/ws_in");
    std::fs::create_dir_all(&websocket_dir).unwrap();
    std::fs::write(websocket_dir.join("index.rs"), DEV_WEBSOCKET_ROUTE).unwrap();

    let server = DevServer::start(&project_dir);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let socket = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        let authority = format!("127.0.0.1:{}", server.port);
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("http://{authority}/ws"))
            .header(HOST, &authority)
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "Upgrade")
            .header(
                "Sec-WebSocket-Key",
                fastwebsockets::handshake::generate_key(),
            )
            .header("Sec-WebSocket-Version", "13")
            .header(SEC_WEBSOCKET_PROTOCOL, "chat.v1")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let (mut websocket, response) =
            fastwebsockets::handshake::client(&WebSocketTestExecutor, request, socket)
                .await
                .unwrap();
        assert_eq!(response.status(), 101);
        assert_eq!(response.headers()["x-app-handshake"], "accepted");
        assert_eq!(response.headers()[SEC_WEBSOCKET_PROTOCOL], "chat.v1");

        websocket
            .write_frame(Frame::text(Payload::Owned(b"hello".to_vec())))
            .await
            .unwrap();
        websocket.flush().await.unwrap();
        let text_frame = tokio::time::timeout(Duration::from_secs(10), websocket.read_frame())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text_frame.opcode, OpCode::Text);
        assert_eq!(text_frame.payload.to_vec(), b"echo:hello");

        websocket
            .write_frame(Frame::binary(Payload::Owned(vec![0, 1, 2, 255])))
            .await
            .unwrap();
        websocket.flush().await.unwrap();
        let binary_frame = tokio::time::timeout(Duration::from_secs(10), websocket.read_frame())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binary_frame.opcode, OpCode::Binary);
        assert_eq!(binary_frame.payload.to_vec(), &[0, 1, 2, 255]);

        websocket
            .write_frame(Frame::close(1000, b"done"))
            .await
            .unwrap();
        websocket.flush().await.unwrap();
        let close_frame = tokio::time::timeout(Duration::from_secs(10), websocket.read_frame())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(close_frame.opcode, OpCode::Close);
    });
}

#[test]
fn test_dev_server_starts_and_responds() {
    let _dev_server_slot = take_the_only_dev_server_slot();
    let temp = tempfile::tempdir().unwrap();
    let project_dir = init_project(temp.path(), "test-app");

    install_npm_deps(&project_dir);

    let index_page_path = project_dir.join("fe/src/pages/index/page.tsx");
    let index_page = std::fs::read_to_string(&index_page_path).unwrap();
    std::fs::write(
        &index_page_path,
        format!(
            "{index_page}\nexport function head(_props: Props) {{\n    return [\n        {{ title: \"Index Head Title\" }},\n        {{ name: \"description\", content: \"index description\" }},\n    ];\n}}\n"
        ),
    )
    .unwrap();

    let server = DevServer::start(&project_dir);

    std::thread::sleep(Duration::from_secs(1));

    let response = reqwest::blocking::get(server.url());

    match response {
        Ok(resp) => {
            assert!(
                resp.status().is_success(),
                "Expected success status, got {}",
                resp.status()
            );
            let body = resp.text().unwrap();
            assert!(body.contains("html"), "Expected HTML response");
            assert!(
                body.contains("<title>Index Head Title</title>"),
                "Expected page head title, got: {body}"
            );
            assert!(
                !body.contains("<title>Forte App</title>"),
                "App default title must be overridden by the page head, got: {body}"
            );
            assert!(
                body.contains("name=\"viewport\""),
                "App head defaults without page overrides must be kept, got: {body}"
            );
            assert!(
                body.contains("name=\"description\" content=\"index description\""),
                "Expected page head meta, got: {body}"
            );
        }
        Err(e) => {
            panic!("Failed to connect to dev server: {}", e);
        }
    }
}

#[test]
fn test_dev_auto_selects_port_if_busy() {
    let _dev_server_slot = take_the_only_dev_server_slot();
    let temp = tempfile::tempdir().unwrap();
    let project_dir = init_project(temp.path(), "test-app-2");

    install_npm_deps(&project_dir);

    let _listener = std::net::TcpListener::bind("0.0.0.0:3000").unwrap();

    let server = DevServer::start(&project_dir);

    std::thread::sleep(Duration::from_secs(1));

    assert_ne!(server.port, 3000, "Should have selected a different port");

    let response = reqwest::blocking::get(server.url());
    assert!(response.is_ok(), "Server should respond on alternate port");
}

const RAW_RESPONSE_WEBHOOK_API: &str = r#"
use anyhow::Result;
use forte_sdk::http::{Body, Response};
use forte_sdk::{ForteRequest, ForteResponse};

pub type Props = ForteResponse;

pub async fn handler(req: ForteRequest<'_>) -> Result<Props> {
    if req.headers.get("x-webhook-signature").is_none() {
        return Ok(Response::builder().status(401).body(Body::empty())?);
    }
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .header("x-fn0-next", "js")
        .body(Body::from(serde_json::to_vec(
            &serde_json::json!({ "type": 1 }),
        )?))?)
}
"#;

const RAW_RESPONSE_STREAM_API: &str = r#"
use anyhow::Result;
use forte_sdk::http::{Body, Response};
use forte_sdk::{ForteRequest, ForteResponse};

pub type Props = ForteResponse;

pub async fn handler(_req: ForteRequest<'_>) -> Result<Props> {
    let (mut writer, body) = Body::channel();
    forte_sdk::runtime::spawn(async move {
        let _leftover = writer.write_all(b"hello ".to_vec()).await;
        let _leftover = writer.write_all(b"stream".to_vec()).await;
        drop(writer);
    });
    Ok(Response::builder().status(200).body(body)?)
}
"#;

#[test]
fn test_dev_raw_response_api() {
    let _dev_server_slot = take_the_only_dev_server_slot();
    let temp = tempfile::tempdir().unwrap();
    let project_dir = init_dev_project(temp.path(), "test-app-raw-response");

    install_npm_deps(&project_dir);

    let apis_dir = project_dir.join("rs/src/apis");
    std::fs::create_dir_all(&apis_dir).unwrap();
    std::fs::write(apis_dir.join("webhook.rs"), RAW_RESPONSE_WEBHOOK_API).unwrap();
    std::fs::write(apis_dir.join("stream.rs"), RAW_RESPONSE_STREAM_API).unwrap();

    let server = DevServer::start(&project_dir);

    std::thread::sleep(Duration::from_secs(1));

    let client = reqwest::blocking::Client::new();

    let unauthorized = client
        .get(format!("{}/api/webhook", server.url()))
        .send()
        .unwrap();
    assert_eq!(unauthorized.status().as_u16(), 401);

    let authorized = client
        .get(format!("{}/api/webhook", server.url()))
        .header("x-webhook-signature", "sig")
        .send()
        .unwrap();
    assert_eq!(authorized.status().as_u16(), 200);
    assert_eq!(
        authorized
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_string()),
        Some("application/json".to_string())
    );
    assert!(
        authorized.headers().get("x-fn0-next").is_none(),
        "x-fn0-* headers must be stripped from raw responses"
    );
    assert_eq!(authorized.text().unwrap(), "{\"type\":1}");

    let streamed = client
        .get(format!("{}/api/stream", server.url()))
        .send()
        .unwrap();
    assert_eq!(streamed.status().as_u16(), 200);
    assert_eq!(streamed.text().unwrap(), "hello stream");
}

fn vite_ssr_server_running(project_dir: &std::path::Path) -> bool {
    let pattern = project_dir.join("fe/.forte/dev/vite-ssr-server-");
    std::process::Command::new("pgrep")
        .args(["-f", &pattern.to_string_lossy()])
        .output()
        .expect("Failed to run pgrep")
        .status
        .success()
}

#[test]
fn test_vite_ssr_exits_when_forte_dies() {
    let _dev_server_slot = take_the_only_dev_server_slot();
    let temp = tempfile::tempdir().unwrap();
    let project_dir = init_project(temp.path(), "test-app-vite-ssr-exit");

    install_npm_deps(&project_dir);

    let mut server = DevServer::start(&project_dir);

    std::thread::sleep(Duration::from_secs(1));

    assert!(
        vite_ssr_server_running(&project_dir),
        "Vite SSR adapter process should be running"
    );

    server.child.kill().expect("Failed to kill forte");
    server.child.wait().expect("Failed to wait for forte");

    let mut vite_ssr_closed = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if !vite_ssr_server_running(&project_dir) {
            vite_ssr_closed = true;
            break;
        }
    }

    assert!(
        vite_ssr_closed,
        "Vite SSR adapter should have exited after forte died"
    );
}
