use crate::http::{Body, Client, HeaderMap, Method, Request, StatusCode, Uri};

const MESSAGE_KIND_HEADER: &str = "x-fn0-websocket-message-kind";
const DELIVERY_STATE_HEADER: &str = "x-fn0-websocket-delivery-state";
const CONNECT_URL_HEADER: &str = "x-fn0-websocket-connect-url";
const CONNECT_PATH_HEADER: &str = "x-fn0-websocket-receive-path";
const SINGLETON_PROJECT_HEADER: &str = "x-fn0-websocket-singleton-project";
const SINGLETON_ID_HEADER: &str = "x-fn0-websocket-singleton-id";
const SINGLETON_ROUTE_HEADER: &str = "x-fn0-websocket-singleton-route";

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub enum WebSocketMessage {
    Text(Body),
    Binary(Body),
}

impl WebSocketMessage {
    pub fn text(body: impl Into<Body>) -> Self {
        Self::Text(body.into())
    }

    pub fn binary(body: impl Into<Body>) -> Self {
        Self::Binary(body.into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncomingMessage {
    Text(String),
    Binary(Vec<u8>),
}

pub struct ConnectEvent {
    pub connection_id: ConnectionId,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub client_address: Option<String>,
    pub requested_protocols: Vec<String>,
}

pub struct MessageEvent {
    pub connection_id: ConnectionId,
    pub message: IncomingMessage,
}

pub struct SingletonConnectionOptions {
    pub url: String,
    pub headers: HeaderMap,
    pub protocols: Vec<String>,
}

impl SingletonConnectionOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HeaderMap::new(),
            protocols: Vec::new(),
        }
    }
}

pub struct SingletonConnectEvent {
    pub connection_id: ConnectionId,
    pub protocol: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisconnectCause {
    Peer,
    Application,
    Deployment,
    HeartbeatTimeout,
    ProtocolError,
    TransportError,
    InternalError,
}

pub struct DisconnectEvent {
    pub connection_id: ConnectionId,
    pub close_code: Option<u16>,
    pub reason: Option<String>,
    pub cause: DisconnectCause,
}

pub enum ConnectDecision {
    Accept {
        protocol: Option<String>,
        headers: HeaderMap,
    },
    Reject {
        status: StatusCode,
        headers: HeaderMap,
    },
}

impl ConnectDecision {
    pub fn accept() -> Self {
        Self::Accept {
            protocol: None,
            headers: HeaderMap::new(),
        }
    }

    pub fn accept_with_protocol(protocol: impl Into<String>) -> Self {
        Self::Accept {
            protocol: Some(protocol.into()),
            headers: HeaderMap::new(),
        }
    }

    pub fn reject(status: StatusCode) -> Self {
        Self::Reject {
            status,
            headers: HeaderMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebSocketDeliveryState {
    NotSent,
    Unknown,
}

#[derive(Debug)]
pub enum WebSocketSendError {
    ConnectionNotFound,
    Backpressure,
    DeadlineExceeded { delivery: WebSocketDeliveryState },
    Transport { delivery: WebSocketDeliveryState },
    InvalidText { delivery: WebSocketDeliveryState },
    Internal { delivery: WebSocketDeliveryState },
}

impl WebSocketSendError {
    pub fn delivery_state(&self) -> WebSocketDeliveryState {
        match self {
            Self::ConnectionNotFound | Self::Backpressure => WebSocketDeliveryState::NotSent,
            Self::DeadlineExceeded { delivery }
            | Self::Transport { delivery }
            | Self::InvalidText { delivery }
            | Self::Internal { delivery } => *delivery,
        }
    }
}

impl std::fmt::Display for WebSocketSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionNotFound => formatter.write_str("websocket connection not found"),
            Self::Backpressure => formatter.write_str("websocket send backpressure limit reached"),
            Self::DeadlineExceeded { delivery } => {
                write!(formatter, "websocket send deadline exceeded ({delivery:?})")
            }
            Self::Transport { delivery } => {
                write!(formatter, "websocket transport failed ({delivery:?})")
            }
            Self::InvalidText { delivery } => {
                write!(
                    formatter,
                    "websocket text is not valid UTF-8 ({delivery:?})"
                )
            }
            Self::Internal { delivery } => {
                write!(formatter, "websocket send failed internally ({delivery:?})")
            }
        }
    }
}

impl std::error::Error for WebSocketSendError {}

#[derive(Debug)]
pub enum WebSocketDisconnectError {
    DeadlineExceeded,
    Transport,
    Internal,
}

impl std::fmt::Display for WebSocketDisconnectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeadlineExceeded => formatter.write_str("websocket disconnect deadline exceeded"),
            Self::Transport => formatter.write_str("websocket disconnect transport failed"),
            Self::Internal => formatter.write_str("websocket disconnect failed internally"),
        }
    }
}

impl std::error::Error for WebSocketDisconnectError {}

#[derive(Debug)]
pub enum WebSocketConnectError {
    InvalidUrl,
    DeadlineExceeded,
    Transport,
    Internal,
}

impl std::fmt::Display for WebSocketConnectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl => formatter.write_str("invalid websocket URL"),
            Self::DeadlineExceeded => formatter.write_str("websocket connect deadline exceeded"),
            Self::Transport => formatter.write_str("websocket connect transport failed"),
            Self::Internal => formatter.write_str("websocket connect failed internally"),
        }
    }
}

impl std::error::Error for WebSocketConnectError {}

pub async fn connect(
    url: impl Into<String>,
    receive_path: &str,
) -> Result<ConnectionId, WebSocketConnectError> {
    let endpoint =
        std::env::var("FN0_WEBSOCKET_URL").map_err(|_| WebSocketConnectError::Internal)?;
    let target_url = url.into();
    if !target_url.starts_with("ws://") && !target_url.starts_with("wss://") {
        return Err(WebSocketConnectError::InvalidUrl);
    }
    if receive_path != "/ws_out" && !receive_path.starts_with("/ws_out/") {
        return Err(WebSocketConnectError::InvalidUrl);
    }
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/connect", endpoint.trim_end_matches('/')))
        .header(CONNECT_URL_HEADER, target_url)
        .header(CONNECT_PATH_HEADER, receive_path)
        .body(Body::empty())
        .map_err(|_| WebSocketConnectError::Internal)?;
    let response = Client::new()
        .send(request)
        .await
        .map_err(|_| WebSocketConnectError::Transport)?;
    match response.status() {
        StatusCode::CREATED => {
            let connection_id = response
                .into_body()
                .bytes()
                .await
                .map_err(|_| WebSocketConnectError::Transport)?;
            let connection_id = String::from_utf8(connection_id.to_vec())
                .map_err(|_| WebSocketConnectError::Internal)?;
            if connection_id.is_empty() {
                return Err(WebSocketConnectError::Internal);
            }
            Ok(ConnectionId::new(connection_id))
        }
        StatusCode::BAD_REQUEST => Err(WebSocketConnectError::InvalidUrl),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(WebSocketConnectError::DeadlineExceeded)
        }
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => {
            Err(WebSocketConnectError::Transport)
        }
        _ => Err(WebSocketConnectError::Internal),
    }
}

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub async fn connect_singleton(
    project_id: &str,
    singleton_id: &str,
    url: impl Into<String>,
    route_path: &str,
    headers: &HeaderMap,
    protocols: &[String],
    claim_token: &str,
    initial_lease_deadline: i64,
) -> Result<ConnectionId, WebSocketConnectError> {
    let endpoint =
        std::env::var("FN0_WEBSOCKET_URL").map_err(|_| WebSocketConnectError::Internal)?;
    let target_url = url.into();
    if project_id.is_empty() || singleton_id.is_empty() || claim_token.is_empty() {
        return Err(WebSocketConnectError::Internal);
    }
    if !target_url.starts_with("ws://") && !target_url.starts_with("wss://") {
        return Err(WebSocketConnectError::InvalidUrl);
    }
    if route_path != "/ws_singleton" && !route_path.starts_with("/ws_singleton/") {
        return Err(WebSocketConnectError::InvalidUrl);
    }
    let mut serialized_headers = Vec::new();
    for (header_name, header_value) in headers {
        if singleton_system_header(header_name.as_str()) {
            return Err(WebSocketConnectError::Internal);
        }
        let header_value = header_value
            .to_str()
            .map_err(|_| WebSocketConnectError::Internal)?;
        serialized_headers.push((header_name.as_str(), header_value));
    }
    if protocols.iter().any(|protocol| {
        protocol.is_empty() || protocol.contains(',') || protocol.chars().any(char::is_whitespace)
    }) {
        return Err(WebSocketConnectError::Internal);
    }
    let body = serde_json::to_vec(&serde_json::json!({
        "url": target_url,
        "headers": serialized_headers,
        "protocols": protocols,
        "claim_token": claim_token,
        "initial_lease_deadline": initial_lease_deadline,
    }))
    .map_err(|_| WebSocketConnectError::Internal)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!(
            "{}/connect-singleton",
            endpoint.trim_end_matches('/')
        ))
        .header(SINGLETON_PROJECT_HEADER, project_id)
        .header(SINGLETON_ID_HEADER, singleton_id)
        .header(SINGLETON_ROUTE_HEADER, route_path)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|_| WebSocketConnectError::Internal)?;
    let response = Client::new()
        .send(request)
        .await
        .map_err(|_| WebSocketConnectError::Transport)?;
    match response.status() {
        StatusCode::CREATED => {
            let connection_id = response
                .into_body()
                .bytes()
                .await
                .map_err(|_| WebSocketConnectError::Transport)?;
            let connection_id = String::from_utf8(connection_id.to_vec())
                .map_err(|_| WebSocketConnectError::Internal)?;
            if connection_id.is_empty() {
                return Err(WebSocketConnectError::Internal);
            }
            Ok(ConnectionId::new(connection_id))
        }
        StatusCode::BAD_REQUEST => Err(WebSocketConnectError::InvalidUrl),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(WebSocketConnectError::DeadlineExceeded)
        }
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => {
            Err(WebSocketConnectError::Transport)
        }
        _ => Err(WebSocketConnectError::Internal),
    }
}

#[doc(hidden)]
pub async fn activate_singleton(
    project_id: &str,
    singleton_id: &str,
    claim_token: &str,
    connection_id: &ConnectionId,
) -> Result<(), WebSocketConnectError> {
    singleton_lifecycle_command(
        "activate-singleton",
        project_id,
        singleton_id,
        claim_token,
        connection_id,
    )
    .await
}

#[doc(hidden)]
pub async fn abort_singleton(
    project_id: &str,
    singleton_id: &str,
    claim_token: &str,
    connection_id: &ConnectionId,
) -> Result<(), WebSocketConnectError> {
    singleton_lifecycle_command(
        "abort-singleton",
        project_id,
        singleton_id,
        claim_token,
        connection_id,
    )
    .await
}

async fn singleton_lifecycle_command(
    command: &str,
    project_id: &str,
    singleton_id: &str,
    claim_token: &str,
    connection_id: &ConnectionId,
) -> Result<(), WebSocketConnectError> {
    let endpoint =
        std::env::var("FN0_WEBSOCKET_URL").map_err(|_| WebSocketConnectError::Internal)?;
    if project_id.is_empty() || singleton_id.is_empty() || claim_token.is_empty() {
        return Err(WebSocketConnectError::Internal);
    }
    let body = serde_json::to_vec(&serde_json::json!({
        "claim_token": claim_token,
        "connection_id": connection_id.as_str(),
    }))
    .map_err(|_| WebSocketConnectError::Internal)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/{command}", endpoint.trim_end_matches('/')))
        .header(SINGLETON_PROJECT_HEADER, project_id)
        .header(SINGLETON_ID_HEADER, singleton_id)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|_| WebSocketConnectError::Internal)?;
    let response = Client::new()
        .send(request)
        .await
        .map_err(|_| WebSocketConnectError::Transport)?;
    match response.status() {
        StatusCode::NO_CONTENT => Ok(()),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(WebSocketConnectError::DeadlineExceeded)
        }
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => {
            Err(WebSocketConnectError::Transport)
        }
        _ => Err(WebSocketConnectError::Internal),
    }
}

fn singleton_system_header(header_name: &str) -> bool {
    matches!(
        header_name,
        "host"
            | "upgrade"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-extensions"
            | "sec-websocket-accept"
            | "sec-websocket-protocol"
    ) || header_name.starts_with("x-fn0-")
}

pub async fn send(
    connection_id: &ConnectionId,
    message: WebSocketMessage,
) -> Result<(), WebSocketSendError> {
    let endpoint =
        std::env::var("FN0_WEBSOCKET_URL").map_err(|_| WebSocketSendError::Internal {
            delivery: WebSocketDeliveryState::NotSent,
        })?;
    let (message_kind, body) = match message {
        WebSocketMessage::Text(body) => ("text", body),
        WebSocketMessage::Binary(body) => ("binary", body),
    };
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!(
            "{}/send/{}",
            endpoint.trim_end_matches('/'),
            connection_id.as_str()
        ))
        .header(MESSAGE_KIND_HEADER, message_kind)
        .body(body)
        .map_err(|_| WebSocketSendError::Internal {
            delivery: WebSocketDeliveryState::NotSent,
        })?;
    let response =
        Client::new()
            .send(request)
            .await
            .map_err(|_| WebSocketSendError::Transport {
                delivery: WebSocketDeliveryState::Unknown,
            })?;
    map_send_response(response.status(), response.headers())
}

pub async fn disconnect(connection_id: &ConnectionId) -> Result<(), WebSocketDisconnectError> {
    let endpoint =
        std::env::var("FN0_WEBSOCKET_URL").map_err(|_| WebSocketDisconnectError::Internal)?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!(
            "{}/disconnect/{}",
            endpoint.trim_end_matches('/'),
            connection_id.as_str()
        ))
        .body(Body::empty())
        .map_err(|_| WebSocketDisconnectError::Internal)?;
    let response = Client::new()
        .send(request)
        .await
        .map_err(|_| WebSocketDisconnectError::Transport)?;
    match response.status() {
        StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => Ok(()),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(WebSocketDisconnectError::DeadlineExceeded)
        }
        status if status.is_server_error() => Err(WebSocketDisconnectError::Transport),
        _ => Err(WebSocketDisconnectError::Internal),
    }
}

fn map_send_response(status: StatusCode, headers: &HeaderMap) -> Result<(), WebSocketSendError> {
    if status == StatusCode::NO_CONTENT {
        return Ok(());
    }
    let delivery = match headers
        .get(DELIVERY_STATE_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some("not-sent") => WebSocketDeliveryState::NotSent,
        _ => WebSocketDeliveryState::Unknown,
    };
    match status {
        StatusCode::NOT_FOUND => Err(WebSocketSendError::ConnectionNotFound),
        StatusCode::TOO_MANY_REQUESTS => Err(WebSocketSendError::Backpressure),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            Err(WebSocketSendError::DeadlineExceeded { delivery })
        }
        StatusCode::UNPROCESSABLE_ENTITY => Err(WebSocketSendError::InvalidText { delivery }),
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => {
            Err(WebSocketSendError::Transport { delivery })
        }
        _ => Err(WebSocketSendError::Internal { delivery }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definite_errors_are_not_sent() {
        assert_eq!(
            WebSocketSendError::ConnectionNotFound.delivery_state(),
            WebSocketDeliveryState::NotSent
        );
        assert_eq!(
            WebSocketSendError::Backpressure.delivery_state(),
            WebSocketDeliveryState::NotSent
        );
    }

    #[test]
    fn response_delivery_header_is_preserved() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DELIVERY_STATE_HEADER,
            "not-sent".parse().expect("valid header"),
        );
        let error = map_send_response(StatusCode::GATEWAY_TIMEOUT, &headers)
            .expect_err("timeout must fail");
        assert_eq!(error.delivery_state(), WebSocketDeliveryState::NotSent);
    }
}
