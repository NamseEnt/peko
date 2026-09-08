use super::SimpleCache;
use anyhow::{Context, Result};
use base64::Engine;
use bytes::Bytes;
use fastwebsockets::{FragmentCollectorRead, Frame, OpCode, Payload, WebSocket};
use fn0::{
    Body, CodeExecutor, Response, WebSocketCommandDispatcher, WebSocketCommandError,
    WebSocketCommandErrorKind, WebSocketCommandFuture, WebSocketConnectFuture,
    WebSocketDeliveryState, WebSocketHijack, WebSocketMessageKind,
};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, HOST, SEC_WEBSOCKET_PROTOCOL, UPGRADE};
use hyper::{Method, Request, StatusCode, Uri};
use rustls::pki_types::ServerName;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio_rustls::TlsConnector;
use url::{Host, Url};
use uuid::Uuid;

const CALLBACK_DEADLINE: Duration = Duration::from_secs(15);
const CLOSE_DEADLINE: Duration = Duration::from_secs(10);
const COMMAND_CAPACITY: usize = 32;

type SharedWriter<S> =
    Arc<tokio::sync::Mutex<fastwebsockets::WebSocketWrite<tokio::io::WriteHalf<S>>>>;

#[derive(Clone)]
pub struct LocalWebSocketService {
    command_sender: mpsc::UnboundedSender<ServiceCommand>,
    connections: Arc<Mutex<HashMap<String, Arc<ConnectionEntry>>>>,
}

struct LocalWebSocketDispatcher {
    service: LocalWebSocketService,
}

enum ServiceCommand {
    Connect {
        caller_project_id: String,
        url: String,
        receive_path: String,
        remaining: Duration,
        response_sender: oneshot::Sender<Result<String, WebSocketCommandError>>,
    },
}

enum SocketCommand {
    Send {
        message_kind: WebSocketMessageKind,
        body: Body,
        response_sender: oneshot::Sender<Result<(), WebSocketCommandError>>,
    },
    Close {
        code: u16,
        info: DisconnectInfo,
        response_sender: Option<oneshot::Sender<Result<(), WebSocketCommandError>>>,
    },
}

enum WriterControl {
    Close(u16, DisconnectInfo),
    TransportLost(DisconnectInfo),
}

struct ConnectionEntry {
    project_id: String,
    command_sender: mpsc::Sender<SocketCommand>,
    closing: AtomicBool,
    complete: AtomicBool,
    completion: Arc<Notify>,
}

#[derive(Clone)]
struct DisconnectInfo {
    close_code: Option<u16>,
    reason: Option<String>,
    cause: &'static str,
}

impl DisconnectInfo {
    fn application() -> Self {
        Self {
            close_code: Some(1000),
            reason: None,
            cause: "application",
        }
    }

    fn deployment() -> Self {
        Self {
            close_code: Some(1012),
            reason: None,
            cause: "deployment",
        }
    }

    fn peer(close_code: Option<u16>, reason: Option<String>) -> Self {
        Self {
            close_code,
            reason,
            cause: "peer",
        }
    }

    fn transport_error() -> Self {
        Self {
            close_code: None,
            reason: None,
            cause: "transport-error",
        }
    }

    fn protocol_error(close_code: u16) -> Self {
        Self {
            close_code: Some(close_code),
            reason: None,
            cause: "protocol-error",
        }
    }
}

impl LocalWebSocketService {
    pub fn start(
        executor: Rc<CodeExecutor<SimpleCache>>,
        websocket_hijack: Option<Arc<WebSocketHijack>>,
    ) -> Arc<Self> {
        let (command_sender, mut command_receiver) = mpsc::unbounded_channel();
        let service = Arc::new(Self {
            command_sender,
            connections: Arc::new(Mutex::new(HashMap::new())),
        });
        if let Some(websocket_hijack) = websocket_hijack {
            websocket_hijack.set_dispatcher(Arc::new(LocalWebSocketDispatcher {
                service: service.as_ref().clone(),
            }));
        }
        let service_for_commands = service.clone();
        tokio::task::spawn_local(async move {
            while let Some(command) = command_receiver.recv().await {
                match command {
                    ServiceCommand::Connect {
                        caller_project_id,
                        url,
                        receive_path,
                        remaining,
                        response_sender,
                    } => {
                        let result = service_for_commands
                            .connect_outbound(
                                executor.clone(),
                                caller_project_id,
                                url,
                                receive_path,
                                remaining,
                            )
                            .await;
                        let _ = response_sender.send(result);
                    }
                }
            }
        });
        service
    }

    pub async fn handle_inbound(
        &self,
        mut request: Request<Incoming>,
        executor: Rc<CodeExecutor<SimpleCache>>,
        project_id: &str,
        client_address: Option<SocketAddr>,
    ) -> Result<Response> {
        let route_uri = route_uri(request.uri(), request.headers())?;
        let connection_id = connection_id();
        let mut callback_request = callback_request(
            &route_uri,
            request.headers(),
            "connect",
            &connection_id,
            None,
            None,
        )?;
        if let Some(client_address) = client_address {
            callback_request.headers_mut().insert(
                "x-fn0-internal-websocket-client-address",
                client_address.to_string().parse()?,
            );
        }
        let connect_response = match tokio::time::timeout(
            CALLBACK_DEADLINE,
            executor.run(project_id, "", callback_request, None),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::warn!(%project_id, %connection_id, %error, "local websocket on_connect failed");
                return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR));
            }
            Err(error) => {
                tracing::warn!(%project_id, %connection_id, %error, "local websocket on_connect timed out");
                return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR));
            }
        };
        let decision = connect_response
            .headers()
            .get("x-fn0-internal-websocket-decision")
            .and_then(|value| value.to_str().ok());
        if connect_response.status() != StatusCode::NO_CONTENT || decision != Some("accept") {
            return Ok(filter_callback_response(connect_response));
        }
        let selected_protocol = connect_response
            .headers()
            .get("x-fn0-internal-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if selected_protocol.as_deref().is_some_and(|selected| {
            !requested_protocols(request.headers())
                .iter()
                .any(|requested| requested == selected)
        }) {
            return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR));
        }
        let (upgrade_response, upgrade_future) = match fastwebsockets::upgrade::upgrade(
            &mut request,
        ) {
            Ok(upgrade) => upgrade,
            Err(error) => {
                tracing::warn!(%project_id, %connection_id, %error, "invalid local websocket upgrade request");
                return Ok(error_response(StatusCode::BAD_REQUEST));
            }
        };
        let mut response = hyper::Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(empty_body())
            .unwrap();
        for (header_name, header_value) in upgrade_response.headers() {
            response
                .headers_mut()
                .append(header_name.clone(), header_value.clone());
        }
        copy_application_headers(&connect_response, response.headers_mut());
        if let Some(selected_protocol) = selected_protocol {
            response
                .headers_mut()
                .insert(SEC_WEBSOCKET_PROTOCOL, selected_protocol.parse()?);
        }
        let route_uri_for_connection = route_uri.clone();
        let project_id = project_id.to_string();
        let service = self.clone();
        let executor_for_connection = executor.clone();
        tokio::task::spawn_local(async move {
            let websocket = match upgrade_future.await {
                Ok(websocket) => websocket,
                Err(error) => {
                    tracing::warn!(%project_id, %connection_id, %error, "local websocket upgrade failed");
                    return;
                }
            };
            service
                .spawn_connection(
                    executor_for_connection,
                    project_id,
                    connection_id,
                    route_uri_for_connection,
                    websocket,
                )
                .await;
        });
        Ok(response)
    }

    pub async fn close_project(&self, project_id: &str) {
        let entries = self
            .connections
            .lock()
            .expect("local websocket connection lock")
            .values()
            .filter(|entry| entry.project_id == project_id)
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            let _ = close_entry(&entry, DisconnectInfo::deployment()).await;
        }
    }

    pub async fn close_all(&self) {
        let entries = self
            .connections
            .lock()
            .expect("local websocket connection lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            let _ = close_entry(&entry, DisconnectInfo::deployment()).await;
        }
    }

    async fn connect_outbound(
        &self,
        executor: Rc<CodeExecutor<SimpleCache>>,
        project_id: String,
        url: String,
        receive_path: String,
        remaining: Duration,
    ) -> Result<String, WebSocketCommandError> {
        let deadline = tokio::time::Instant::now() + remaining;
        let parsed_url = Url::parse(&url)
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?;
        let scheme = parsed_url.scheme();
        if scheme != "ws" && scheme != "wss" {
            return Err(command_error(false, WebSocketCommandErrorKind::Transport));
        }
        let host = parsed_url
            .host_str()
            .ok_or_else(|| command_error(false, WebSocketCommandErrorKind::Transport))?;
        let port = parsed_url
            .port_or_known_default()
            .ok_or_else(|| command_error(false, WebSocketCommandErrorKind::Transport))?;
        let socket = tokio::time::timeout_at(deadline, TcpStream::connect((host, port)))
            .await
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?;
        let (request, expected_accept) = outbound_request(&parsed_url)?;
        let (websocket, response) = if scheme == "wss" {
            let connector = tls_connector()
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?;
            let server_name = ServerName::try_from(host.to_string())
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?;
            let tls_socket =
                tokio::time::timeout_at(deadline, connector.connect(server_name, socket))
                    .await
                    .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
                    .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?;
            tokio::time::timeout_at(
                deadline,
                fastwebsockets::handshake::client(&OutboundExecutor, request, tls_socket),
            )
            .await
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?
        } else {
            tokio::time::timeout_at(
                deadline,
                fastwebsockets::handshake::client(&OutboundExecutor, request, socket),
            )
            .await
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))?
        };
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(command_error(false, WebSocketCommandErrorKind::Transport));
        }
        validate_outbound_handshake(response.headers(), &expected_accept, &[])?;
        let connection_id = connection_id();
        let route_uri = format!("http://fn0-websocket.local{receive_path}")
            .parse::<Uri>()
            .map_err(|_| command_error(false, WebSocketCommandErrorKind::Internal))?;
        self.spawn_connection(
            executor,
            project_id,
            connection_id.clone(),
            route_uri,
            websocket,
        )
        .await;
        Ok(connection_id)
    }

    async fn spawn_connection<S>(
        &self,
        executor: Rc<CodeExecutor<SimpleCache>>,
        project_id: String,
        connection_id: String,
        route_uri: Uri,
        websocket: WebSocket<S>,
    ) where
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let (reader, writer) = websocket.split(tokio::io::split);
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (control_sender, control_receiver) = mpsc::unbounded_channel();
        let completion = Arc::new(Notify::new());
        let entry = Arc::new(ConnectionEntry {
            project_id: project_id.clone(),
            command_sender,
            closing: AtomicBool::new(false),
            complete: AtomicBool::new(false),
            completion: completion.clone(),
        });
        self.connections
            .lock()
            .expect("local websocket connection lock")
            .insert(connection_id.clone(), entry);
        let service = self.clone();
        tokio::task::spawn_local(async move {
            run_connection(
                service,
                executor,
                project_id,
                connection_id,
                route_uri,
                reader,
                writer,
                command_receiver,
                control_receiver,
                control_sender,
            )
            .await;
        });
    }
}

impl WebSocketCommandDispatcher for LocalWebSocketDispatcher {
    fn connect(
        &self,
        caller_project_id: String,
        url: String,
        receive_path: String,
        remaining: Duration,
    ) -> WebSocketConnectFuture {
        let command_sender = self.service.command_sender.clone();
        Box::pin(async move {
            let (response_sender, response_receiver) = oneshot::channel();
            command_sender
                .send(ServiceCommand::Connect {
                    caller_project_id,
                    url,
                    receive_path,
                    remaining,
                    response_sender,
                })
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Internal))?;
            response_receiver
                .await
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Internal))?
        })
    }

    fn send(
        &self,
        caller_project_id: String,
        connection_id: String,
        message_kind: WebSocketMessageKind,
        body: Body,
        remaining: Duration,
    ) -> WebSocketCommandFuture {
        let service = self.service.clone();
        Box::pin(async move {
            let entry = connection_entry(&service, &caller_project_id, &connection_id)?;
            let (response_sender, response_receiver) = oneshot::channel();
            entry
                .command_sender
                .send(SocketCommand::Send {
                    message_kind,
                    body,
                    response_sender,
                })
                .await
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::ConnectionNotFound))?;
            tokio::time::timeout(remaining, response_receiver)
                .await
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::ConnectionNotFound))?
        })
    }

    fn disconnect(
        &self,
        caller_project_id: String,
        connection_id: String,
        remaining: Duration,
    ) -> WebSocketCommandFuture {
        let service = self.service.clone();
        Box::pin(async move {
            let entry = connection_entry(&service, &caller_project_id, &connection_id)?;
            close_entry_with_deadline(&entry, DisconnectInfo::application(), remaining).await
        })
    }
}

struct OutboundExecutor;

impl<Fut> hyper::rt::Executor<Fut> for OutboundExecutor
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, future: Fut) {
        tokio::spawn(future);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_connection<S>(
    service: LocalWebSocketService,
    executor: Rc<CodeExecutor<SimpleCache>>,
    project_id: String,
    connection_id: String,
    route_uri: Uri,
    reader: fastwebsockets::WebSocketRead<tokio::io::ReadHalf<S>>,
    writer: fastwebsockets::WebSocketWrite<tokio::io::WriteHalf<S>>,
    command_receiver: mpsc::Receiver<SocketCommand>,
    control_receiver: mpsc::UnboundedReceiver<WriterControl>,
    control_sender: mpsc::UnboundedSender<WriterControl>,
) where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let disconnect_info = Arc::new(Mutex::new(None));
    let shared_writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut reader_handle = tokio::task::spawn_local(read_loop(
        executor.clone(),
        project_id.clone(),
        connection_id.clone(),
        route_uri.clone(),
        reader,
        shared_writer.clone(),
        disconnect_info.clone(),
        control_sender,
    ));
    let mut writer_handle = tokio::task::spawn_local(write_loop(
        shared_writer,
        command_receiver,
        control_receiver,
        disconnect_info.clone(),
    ));
    tokio::select! {
        _ = &mut reader_handle => {
            writer_handle.abort();
            let _ = writer_handle.await;
        }
        _ = &mut writer_handle => {
            reader_handle.abort();
            let _ = reader_handle.await;
        }
    }
    let connection_entry = service
        .connections
        .lock()
        .expect("local websocket connection lock")
        .remove(&connection_id);
    let info = disconnect_info
        .lock()
        .expect("local websocket disconnect lock")
        .clone()
        .unwrap_or_else(DisconnectInfo::transport_error);
    invoke_disconnect(executor, &project_id, &connection_id, &route_uri, info).await;
    if let Some(connection_entry) = connection_entry {
        connection_entry.complete.store(true, Ordering::Release);
        connection_entry.completion.notify_waiters();
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_loop<S>(
    executor: Rc<CodeExecutor<SimpleCache>>,
    project_id: String,
    connection_id: String,
    route_uri: Uri,
    reader: fastwebsockets::WebSocketRead<tokio::io::ReadHalf<S>>,
    writer: SharedWriter<S>,
    disconnect_info: Arc<Mutex<Option<DisconnectInfo>>>,
    control_sender: mpsc::UnboundedSender<WriterControl>,
) where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let mut reader = FragmentCollectorRead::new(reader);
    let mut send_control_frame = |frame: Frame<'static>| {
        let writer = writer.clone();
        async move {
            let mut writer = writer.lock().await;
            writer
                .write_frame(frame)
                .await
                .map_err(|error| anyhow::anyhow!("write control frame: {error}"))?;
            writer
                .flush()
                .await
                .map_err(|error| anyhow::anyhow!("flush control frame: {error}"))
        }
    };
    loop {
        let frame = match reader.read_frame(&mut send_control_frame).await {
            Ok(frame) => frame,
            Err(error) => {
                set_disconnect_info(&disconnect_info, DisconnectInfo::transport_error());
                let _ = control_sender.send(WriterControl::TransportLost(
                    DisconnectInfo::transport_error(),
                ));
                tracing::debug!(%project_id, %connection_id, %error, "local websocket read failed");
                return;
            }
        };
        match frame.opcode {
            OpCode::Text | OpCode::Binary => {
                let message_kind = if frame.opcode == OpCode::Text {
                    WebSocketMessageKind::Text
                } else {
                    WebSocketMessageKind::Binary
                };
                let message_bytes: Vec<u8> = frame.payload.into();
                if let Err(error) = invoke_message(
                    executor.clone(),
                    &project_id,
                    &connection_id,
                    &route_uri,
                    message_kind,
                    message_bytes,
                )
                .await
                {
                    tracing::warn!(%project_id, %connection_id, %error, "local websocket message callback failed");
                    let info = DisconnectInfo::protocol_error(1011);
                    set_disconnect_info(&disconnect_info, info.clone());
                    let _ = control_sender.send(WriterControl::Close(1011, info));
                    return;
                }
            }
            OpCode::Close => {
                let payload: Vec<u8> = frame.payload.into();
                let close_code =
                    (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                let reason =
                    (payload.len() > 2).then(|| String::from_utf8_lossy(&payload[2..]).to_string());
                let info = DisconnectInfo::peer(close_code, reason);
                set_disconnect_info(&disconnect_info, info.clone());
                let _ = control_sender.send(WriterControl::Close(close_code.unwrap_or(1000), info));
                return;
            }
            _ => {}
        }
    }
}

async fn write_loop<S>(
    writer: SharedWriter<S>,
    mut command_receiver: mpsc::Receiver<SocketCommand>,
    mut control_receiver: mpsc::UnboundedReceiver<WriterControl>,
    disconnect_info: Arc<Mutex<Option<DisconnectInfo>>>,
) where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    loop {
        tokio::select! {
            command = command_receiver.recv() => {
                match command {
                    Some(SocketCommand::Send { message_kind, body, response_sender }) => {
                        let result = send_message(&writer, message_kind, body).await;
                        let should_close = result.as_ref().is_err_and(|error| {
                            error.kind != WebSocketCommandErrorKind::InvalidText
                                || error.delivery == WebSocketDeliveryState::Unknown
                        });
                        let _ = response_sender.send(result);
                        if should_close {
                            let info = DisconnectInfo::protocol_error(1011);
                            set_disconnect_info(&disconnect_info, info.clone());
                            let _ = write_close(&writer, 1011).await;
                            return;
                        }
                    }
                    Some(SocketCommand::Close { code, info, response_sender }) => {
                        set_disconnect_info(&disconnect_info, info);
                        let result = write_close(&writer, code).await;
                        if let Some(response_sender) = response_sender {
                            let _ = response_sender.send(result);
                        }
                        return;
                    }
                    None => return,
                }
            }
            control = control_receiver.recv() => {
                match control {
                    Some(WriterControl::Close(code, info)) => {
                        set_disconnect_info(&disconnect_info, info);
                        let _ = write_close(&writer, code).await;
                        return;
                    }
                    Some(WriterControl::TransportLost(info)) => {
                        set_disconnect_info(&disconnect_info, info);
                        return;
                    }
                    None => return,
                }
            }
        }
    }
}

async fn send_message<S>(
    writer: &SharedWriter<S>,
    message_kind: WebSocketMessageKind,
    mut body: Body,
) -> Result<(), WebSocketCommandError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut body_bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| command_error(false, WebSocketCommandErrorKind::Internal))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        body_bytes.extend_from_slice(&data);
    }
    if message_kind == WebSocketMessageKind::Text && std::str::from_utf8(&body_bytes).is_err() {
        return Err(command_error(false, WebSocketCommandErrorKind::InvalidText));
    }
    let opcode = match message_kind {
        WebSocketMessageKind::Text => OpCode::Text,
        WebSocketMessageKind::Binary => OpCode::Binary,
    };
    let frame = Frame::new(true, opcode, None, Payload::Owned(body_bytes));
    let mut writer = writer.lock().await;
    writer
        .write_frame(frame)
        .await
        .map_err(|_| command_error(true, WebSocketCommandErrorKind::Transport))?;
    writer
        .flush()
        .await
        .map_err(|_| command_error(true, WebSocketCommandErrorKind::Transport))
}

async fn write_close<S>(writer: &SharedWriter<S>, code: u16) -> Result<(), WebSocketCommandError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut writer = writer.lock().await;
    writer
        .write_frame(Frame::close(code, &[]))
        .await
        .map_err(|_| command_error(true, WebSocketCommandErrorKind::Transport))?;
    writer
        .flush()
        .await
        .map_err(|_| command_error(true, WebSocketCommandErrorKind::Transport))
}

async fn invoke_message(
    executor: Rc<CodeExecutor<SimpleCache>>,
    project_id: &str,
    connection_id: &str,
    route_uri: &Uri,
    message_kind: WebSocketMessageKind,
    message_bytes: Vec<u8>,
) -> Result<()> {
    let mut request = callback_request(
        route_uri,
        &hyper::HeaderMap::new(),
        "message",
        connection_id,
        Some(message_kind),
        Some(message_bytes),
    )?;
    request.headers_mut().insert(
        "x-fn0-internal-websocket-message-kind",
        match message_kind {
            WebSocketMessageKind::Text => "text",
            WebSocketMessageKind::Binary => "binary",
        }
        .parse()?,
    );
    let response = tokio::time::timeout(
        CALLBACK_DEADLINE,
        executor.run(project_id, "", request, None),
    )
    .await
    .context("WebSocket message callback timed out")??;
    if response.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("WebSocket message callback returned {}", response.status())
    }
}

async fn invoke_disconnect(
    executor: Rc<CodeExecutor<SimpleCache>>,
    project_id: &str,
    connection_id: &str,
    route_uri: &Uri,
    info: DisconnectInfo,
) {
    let body = Empty::<Bytes>::new()
        .map_err(|never: std::convert::Infallible| match never {})
        .boxed_unsync();
    let mut request = match Request::builder()
        .method(Method::POST)
        .uri(route_uri.clone())
        .body(body)
    {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%project_id, %connection_id, %error, "failed to build disconnect callback request");
            return;
        }
    };
    request.headers_mut().insert(
        "x-fn0-internal-websocket-event",
        "disconnect".parse().expect("static header"),
    );
    request.headers_mut().insert(
        "x-fn0-internal-websocket-connection-id",
        connection_id.parse().expect("connection id header"),
    );
    request.headers_mut().insert(
        "x-fn0-internal-websocket-disconnect-cause",
        info.cause.parse().expect("disconnect cause header"),
    );
    if let Some(close_code) = info.close_code {
        request.headers_mut().insert(
            "x-fn0-internal-websocket-close-code",
            close_code.to_string().parse().expect("close code header"),
        );
    }
    if let Some(reason) = info.reason
        && let Ok(reason_header) = reason.parse()
    {
        request
            .headers_mut()
            .insert("x-fn0-internal-websocket-close-reason", reason_header);
    }
    let callback_result = tokio::time::timeout(
        CALLBACK_DEADLINE,
        executor.run(project_id, "", request, None),
    )
    .await
    .map_err(|_| anyhow::anyhow!("WebSocket disconnect callback timed out"))
    .and_then(|result| result.context("WebSocket disconnect callback failed"));
    if let Err(error) = callback_result {
        tracing::warn!(%project_id, %connection_id, %error, "WebSocket disconnect callback failed");
    }
}

fn callback_request(
    route_uri: &Uri,
    source_headers: &hyper::HeaderMap,
    event_name: &str,
    connection_id: &str,
    message_kind: Option<WebSocketMessageKind>,
    body_bytes: Option<Vec<u8>>,
) -> Result<Request<Body>> {
    let body_bytes = body_bytes.unwrap_or_default();
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(route_uri.clone())
        .body(
            Full::new(Bytes::from(body_bytes))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed_unsync(),
        )?;
    for (header_name, header_value) in source_headers {
        if !header_name.as_str().starts_with("x-fn0-internal-") {
            request
                .headers_mut()
                .append(header_name.clone(), header_value.clone());
        }
    }
    request
        .headers_mut()
        .insert("x-fn0-internal-websocket-event", event_name.parse()?);
    request.headers_mut().insert(
        "x-fn0-internal-websocket-connection-id",
        connection_id.parse()?,
    );
    if let Some(message_kind) = message_kind {
        request.headers_mut().insert(
            "x-fn0-internal-websocket-message-kind",
            match message_kind {
                WebSocketMessageKind::Text => "text",
                WebSocketMessageKind::Binary => "binary",
            }
            .parse()?,
        );
    }
    Ok(request)
}

fn connection_entry(
    service: &LocalWebSocketService,
    project_id: &str,
    connection_id: &str,
) -> Result<Arc<ConnectionEntry>, WebSocketCommandError> {
    let entry = service
        .connections
        .lock()
        .expect("local websocket connection lock")
        .get(connection_id)
        .cloned()
        .ok_or_else(|| command_error(false, WebSocketCommandErrorKind::ConnectionNotFound))?;
    if entry.project_id != project_id || entry.closing.load(Ordering::Acquire) {
        return Err(command_error(
            false,
            WebSocketCommandErrorKind::ConnectionNotFound,
        ));
    }
    Ok(entry)
}

fn set_disconnect_info(disconnect_info: &Arc<Mutex<Option<DisconnectInfo>>>, info: DisconnectInfo) {
    let mut current = disconnect_info
        .lock()
        .expect("local websocket disconnect lock");
    if current.is_none() {
        *current = Some(info);
    }
}

async fn close_entry(
    entry: &Arc<ConnectionEntry>,
    info: DisconnectInfo,
) -> Result<(), WebSocketCommandError> {
    let close_result = close_entry_with_deadline(entry, info, CLOSE_DEADLINE).await;
    let completion_result = wait_for_completion(entry).await;
    close_result.and(completion_result)
}

async fn wait_for_completion(entry: &ConnectionEntry) -> Result<(), WebSocketCommandError> {
    if entry.complete.load(Ordering::Acquire) {
        return Ok(());
    }
    let wait = async {
        loop {
            let notified = entry.completion.notified();
            if entry.complete.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    };
    tokio::time::timeout(CLOSE_DEADLINE, wait)
        .await
        .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))
}

async fn close_entry_with_deadline(
    entry: &Arc<ConnectionEntry>,
    info: DisconnectInfo,
    remaining: Duration,
) -> Result<(), WebSocketCommandError> {
    entry.closing.store(true, Ordering::Release);
    let (response_sender, response_receiver) = oneshot::channel();
    entry
        .command_sender
        .send(SocketCommand::Close {
            code: info.close_code.unwrap_or(1000),
            info,
            response_sender: Some(response_sender),
        })
        .await
        .map_err(|_| command_error(false, WebSocketCommandErrorKind::ConnectionNotFound))?;
    tokio::time::timeout(remaining, response_receiver)
        .await
        .map_err(|_| command_error(false, WebSocketCommandErrorKind::DeadlineExceeded))?
        .map_err(|_| command_error(false, WebSocketCommandErrorKind::ConnectionNotFound))?
}

fn command_error(wrote_frame: bool, kind: WebSocketCommandErrorKind) -> WebSocketCommandError {
    WebSocketCommandError {
        kind,
        delivery: if wrote_frame {
            WebSocketDeliveryState::Unknown
        } else {
            WebSocketDeliveryState::NotSent
        },
    }
}

fn route_uri(uri: &Uri, headers: &hyper::HeaderMap) -> Result<Uri> {
    if uri.authority().is_some() {
        return Ok(uri.clone());
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .context("WebSocket request missing Host header")?;
    format!("http://{host}{uri}")
        .parse()
        .context("invalid WebSocket route URI")
}

fn requested_protocols(headers: &hyper::HeaderMap) -> Vec<String> {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn filter_callback_response(response: Response) -> Response {
    let allowed_headers = response
        .headers()
        .iter()
        .filter(|(header_name, _)| websocket_handshake_header_allowed(header_name))
        .map(|(header_name, header_value)| (header_name.clone(), header_value.clone()))
        .collect::<Vec<_>>();
    let (mut parts, _) = response.into_parts();
    parts.headers.clear();
    let mut response = hyper::Response::from_parts(parts, empty_body());
    for (header_name, header_value) in allowed_headers {
        response.headers_mut().append(header_name, header_value);
    }
    response
}

fn copy_application_headers(response: &Response, destination: &mut hyper::HeaderMap) {
    for (header_name, header_value) in response.headers() {
        if websocket_handshake_header_allowed(header_name) {
            destination.append(header_name.clone(), header_value.clone());
        }
    }
}

fn websocket_handshake_header_allowed(header_name: &hyper::header::HeaderName) -> bool {
    !matches!(
        header_name.as_str(),
        "connection"
            | "upgrade"
            | "sec-websocket-accept"
            | "sec-websocket-protocol"
            | "content-length"
            | "transfer-encoding"
    ) && !header_name.as_str().starts_with("x-fn0-")
        && !header_name.as_str().starts_with("sec-websocket-")
}

fn empty_body() -> Body {
    Full::new(Bytes::new())
        .map_err(|never: std::convert::Infallible| match never {})
        .boxed_unsync()
}

fn error_response(status: StatusCode) -> Response {
    hyper::Response::builder()
        .status(status)
        .body(empty_body())
        .unwrap()
}

fn connection_id() -> String {
    let mut random_bytes = [0u8; 32];
    random_bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    random_bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    format!(
        "v1.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
    )
}

fn outbound_request(url: &Url) -> Result<(Request<Empty<Bytes>>, String), WebSocketCommandError> {
    let port = url
        .port_or_known_default()
        .ok_or_else(|| command_error(false, WebSocketCommandErrorKind::Transport))?;
    let authority = match url.host() {
        Some(Host::Ipv6(address)) => format!("[{address}]:{port}"),
        Some(Host::Ipv4(address)) => format!("{address}:{port}"),
        Some(Host::Domain(domain)) => format!("{domain}:{port}"),
        None => return Err(command_error(false, WebSocketCommandErrorKind::Transport)),
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let path = match url.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    };
    let websocket_key = fastwebsockets::handshake::generate_key();
    let expected_accept = websocket_accept(&websocket_key);
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("http://{authority}{path}"))
        .header(HOST, authority)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Key", websocket_key)
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::new())
        .map_err(|_| command_error(false, WebSocketCommandErrorKind::Internal))?;
    Ok((request, expected_accept))
}

fn websocket_accept(websocket_key: &str) -> String {
    let mut digest = Sha1::new();
    digest.update(websocket_key.as_bytes());
    digest.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(digest.finalize())
}

fn validate_outbound_handshake(
    headers: &hyper::HeaderMap,
    expected_accept: &str,
    requested_protocols: &[String],
) -> Result<(), WebSocketCommandError> {
    let accept_values = headers
        .get_all("sec-websocket-accept")
        .iter()
        .map(|value| {
            value
                .to_str()
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if accept_values.as_slice() != [expected_accept] {
        return Err(command_error(false, WebSocketCommandErrorKind::Transport));
    }
    if headers.contains_key("sec-websocket-extensions") {
        return Err(command_error(false, WebSocketCommandErrorKind::Transport));
    }
    let selected_protocols = headers
        .get_all("sec-websocket-protocol")
        .iter()
        .map(|value| {
            value
                .to_str()
                .map_err(|_| command_error(false, WebSocketCommandErrorKind::Transport))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if selected_protocols.len() > 1 {
        return Err(command_error(false, WebSocketCommandErrorKind::Transport));
    }
    if let Some(selected_protocol) = selected_protocols.first()
        && (!valid_websocket_protocol(selected_protocol)
            || !requested_protocols
                .iter()
                .any(|requested_protocol| requested_protocol == selected_protocol))
    {
        return Err(command_error(false, WebSocketCommandErrorKind::Transport));
    }
    Ok(())
}

fn valid_websocket_protocol(protocol: &str) -> bool {
    !protocol.is_empty()
        && protocol.bytes().all(|byte| {
            matches!(
                byte,
                b'!' | b'#'..=b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'0'..=b'9'
                    | b'A'..=b'Z'
                    | b'^'..=b'z'
                    | b'|'
                    | b'~'
            )
        })
}

fn tls_connector() -> Result<TlsConnector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_uri_uses_host_for_origin_form() {
        let uri: Uri = "/ws/chat?room=1".parse().expect("URI");
        let mut headers = hyper::HeaderMap::new();
        headers.insert(HOST, "localhost:3000".parse().expect("host"));
        assert_eq!(
            route_uri(&uri, &headers).expect("route URI"),
            "http://localhost:3000/ws/chat?room=1"
        );
    }

    #[test]
    fn requested_protocols_are_split_and_trimmed() {
        let mut headers = hyper::HeaderMap::new();
        headers.append(
            SEC_WEBSOCKET_PROTOCOL,
            "chat, superchat".parse().expect("protocol"),
        );
        headers.append(SEC_WEBSOCKET_PROTOCOL, "binary".parse().expect("protocol"));
        assert_eq!(
            requested_protocols(&headers),
            ["chat", "superchat", "binary"]
        );
    }

    #[test]
    fn connection_ids_match_hijack_format() {
        let identifier = connection_id();
        let encoded = identifier.strip_prefix("v1.").expect("version");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("base64");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn handshake_header_filter_keeps_application_headers_only() {
        assert!(websocket_handshake_header_allowed(
            &"set-cookie".parse().expect("header")
        ));
        assert!(!websocket_handshake_header_allowed(
            &"connection".parse().expect("header")
        ));
        assert!(!websocket_handshake_header_allowed(
            &"sec-websocket-extensions".parse().expect("header")
        ));
        assert!(!websocket_handshake_header_allowed(
            &"x-fn0-internal-websocket-decision".parse().expect("header")
        ));
    }

    #[test]
    fn outbound_request_preserves_path_and_query() {
        let url = Url::parse("ws://example.com:8080/socket?room=42").expect("URL");
        let (request, _) = outbound_request(&url).expect("request");
        assert_eq!(request.uri(), "http://example.com:8080/socket?room=42");
        assert_eq!(request.headers()[HOST], "example.com:8080");
    }

    #[test]
    fn outbound_request_brackets_ipv6_authority() {
        let url = Url::parse("ws://[::1]:8080/socket").expect("URL");
        let (request, _) = outbound_request(&url).expect("request");
        assert_eq!(request.uri(), "http://[::1]:8080/socket");
        assert_eq!(request.headers()[HOST], "[::1]:8080");
    }

    #[test]
    fn outbound_handshake_rejects_invalid_accept_protocol_and_extension() {
        let expected_accept = "expected";
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            "sec-websocket-accept",
            expected_accept.parse().expect("header"),
        );
        headers.insert(
            "sec-websocket-protocol",
            "unexpected".parse().expect("header"),
        );
        assert!(validate_outbound_handshake(&headers, expected_accept, &[]).is_err());

        headers.remove("sec-websocket-protocol");
        headers.insert(
            "sec-websocket-extensions",
            "permessage-deflate".parse().expect("header"),
        );
        assert!(validate_outbound_handshake(&headers, expected_accept, &[]).is_err());

        headers.remove("sec-websocket-extensions");
        headers.insert("sec-websocket-accept", "wrong".parse().expect("header"));
        assert!(validate_outbound_handshake(&headers, expected_accept, &[]).is_err());
    }
}
