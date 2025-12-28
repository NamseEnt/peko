# Forte Master Plan v1.0 (Unified - WASM Architecture)

---

## 1. 아키텍처 개요

### 1.1 철학

**"물리적 분리, 논리적 통합"**

- 코드는 백엔드(Rust)와 프론트엔드(React)로 나뉘어 있지만, 라우팅과 데이터 타입은 CLI에 의해 강력하게 결합됩니다.
- 개발자는 "설정(Config)"을 건드리지 않습니다. 오직 "규칙(Convention)"에 맞춰 파일만 만듭니다.
- 프레임워크는 최소한의 기능만 제공하고, 확장은 사용자의 몫입니다.

### 1.2 Core Stack

| 레이어        | 기술                                  | 역할                                           |
| ------------- | ------------------------------------- | ---------------------------------------------- |
| CLI           | Rust + Wasmtime 임베딩                | 파일 감지, 코드 파싱, 코드 생성, 프로세스 관리 |
| Backend       | Rust + wit-bindgen (wasi-http) + wstd | API 서버 (WASM 컴포넌트)                       |
| Frontend      | React + Vite + Node.js                | SSR 렌더링 서버                                |
| Communication | CLI 프록시 → Node / backend.wasm      | 요청 라우팅 및 통신                            |

### 1.3 요청 흐름

```
Browser ──────────────────▶ Forte CLI (3000)
                                  │
                    ┌─────────────┴─────────────┐
                    │                           │
                    ▼                           ▼
            [Static Files]              [Dynamic Routes]
            /client/*, /static/*               │
                    │               ┌──────────┴──────────┐
                    │               │                     │
                    ▼               ▼                     ▼
                 직접 응답    backend.wasm (Wasmtime)   Node.js
                                    │                     │
                                    ▼                     │
                             PageProps (JSON) ───────────▶│
                                                          │
                                                          ▼
Browser ◀─────────────────────────────────────── HTML + Hydration Script
```

**핵심**: Forte CLI가 모든 요청의 진입점이 되어 라우팅을 직접 제어합니다.

### 1.4 CLI 프록시 라우팅 규칙

```
요청 경로                    처리 방식
────────────────────────────────────────────────────────
/client/*                   → 정적 파일 직접 서빙
/static/*                   → 정적 파일 직접 서빙
/__forte/*              → CLI 내부 API (HMR, 상태 등)
/api/*                      → backend.wasm 직접 호출 (JSON 응답)
그 외 모든 경로              → backend.wasm → Node.js SSR
```

---

## 2. 디렉토리 구조

### 2.1 전체 구조

```
my-project/
├── backend/
│   ├── Cargo.toml
│   ├── .cargo/
│   │   └── config.toml         [Generated] wasm32-wasip2 타겟 설정
│   └── src/
│       └── routes/
│           ├── mod.rs              [Generated] CLI가 자동 관리
│           ├── index/
│           │   └── props.rs        [User Code]
│           └── product/
│               └── _id_/           ← Rust는 _id_ 형식
│                   └── props.rs    [User Code]
│
├── frontend/
│   ├── package.json
│   └── src/
│       └── app/
│           ├── layout.tsx          [User Code] 루트 레이아웃
│           ├── index/
│           │   └── page.tsx        [User Code]
│           └── product/
│               └── [id]/           ← Frontend는 [id] 형식
│                   ├── page.tsx    [User Code]
│                   └── props.gen.ts [Generated]
│
├── .generated/                     [Hidden] CLI 생성 코드
│   ├── backend/
│   │   ├── lib.rs
│   │   ├── router.rs
│   │   └── env.rs
│   └── frontend/
│       ├── server.js
│       ├── client.tsx
│       └── routes.ts
│
├── .env
├── .env.development
├── .env.production
├── Forte.toml
└── README.md
```

### 2.2 경로 매핑 규칙

| Backend (Rust)                        | Frontend (React)                   | URL                          |
| ------------------------------------- | ---------------------------------- | ---------------------------- |
| `routes/index/`                       | `app/index/`                       | `/`                          |
| `routes/about/`                       | `app/about/`                       | `/about`                     |
| `routes/product/_id_/`                | `app/product/[id]/`                | `/product/:id`               |
| `routes/user/_userId_/post/_postId_/` | `app/user/[userId]/post/[postId]/` | `/user/:userId/post/:postId` |

### 2.3 라우트 그룹 (URL 미반영)

```
app/
└── (marketing)/        ← 괄호로 감싸면 URL에 미포함
    ├── about/page.tsx      → /about
    └── contact/page.tsx    → /contact
```

---

## 3. CLI 엔진 상세

### 3.1 명령어 체계

```bash
forte init <project-name>   # 프로젝트 생성
forte dev                   # 개발 서버 실행
forte build                 # 프로덕션 빌드
forte serve ./dist          # 프로덕션 결과물 실행
forte test                  # 테스트 실행
```

### 3.2 Wasmtime 임베딩

Forte CLI에 Wasmtime을 임베딩하여 backend.wasm을 실행합니다:

```rust
// forte-cli/src/runtime/mod.rs
use wasmtime::{Config, Engine, Store, component::*};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

pub struct WasmRuntime {
    engine: Engine,
    component: Component,
    linker: Linker<ServerState>,
}

struct ServerState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
}

impl WasiView for ServerState {
    fn ctx(&mut self) -> &mut WasiCtx { &mut self.wasi }
}

impl WasiHttpView for ServerState {
    fn ctx(&mut self) -> &mut WasiHttpCtx { &mut self.http }
}

impl WasmRuntime {
    pub fn new(wasm_path: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, wasm_path)?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)?;
        wasmtime_wasi_http::add_to_linker_sync(&mut linker)?;

        Ok(Self { engine, component, linker })
    }

    pub fn handle_request(&self, req: http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
        let mut store = Store::new(&self.engine, ServerState {
            wasi: WasiCtxBuilder::new()
                .envs(&self.env_vars)
                .build(),
            http: WasiHttpCtx::new(),
        });

        // wasi-http incoming-handler 호출
        let instance = self.linker.instantiate(&mut store, &self.component)?;
        // ... request/response 변환 및 호출
    }
}
```

### 3.3 프록시 서버

CLI가 HTTP 프록시 역할을 수행:

```rust
// forte-cli/src/server/proxy.rs
use hyper::{Server, Request, Response, Body};

pub struct CoRouterProxy {
    wasm_runtime: WasmRuntime,
    node_client: NodeClient,
    static_handler: StaticFileHandler,
}

impl CoRouterProxy {
    pub async fn handle(&self, req: Request<Body>) -> Response<Body> {
        let path = req.uri().path();

        match self.classify_request(path) {
            RequestType::Static => {
                self.static_handler.serve(path).await
            }
            RequestType::Internal => {
                self.handle_internal(req).await
            }
            RequestType::ApiOnly => {
                // backend.wasm 직접 호출, JSON 응답
                self.wasm_runtime.handle_request(req.into()).into()
            }
            RequestType::Page => {
                // 1. backend.wasm에서 PageProps 획득
                let props = self.wasm_runtime.get_props(&req);

                // 2. Node.js SSR로 전달
                self.node_client.render(path, props).await
            }
        }
    }

    fn classify_request(&self, path: &str) -> RequestType {
        if path.starts_with("/client/") || path.starts_with("/static/") {
            RequestType::Static
        } else if path.starts_with("/__forte/") {
            RequestType::Internal
        } else if path.starts_with("/api/") {
            RequestType::ApiOnly
        } else {
            RequestType::Page
        }
    }
}
```

### 3.4 `forte dev` 동작

```bash
forte dev
```

실행 시 동작:

```
1. 환경 변수 로드 (.env.development)
2. rustup target 확인 (wasm32-wasip2 없으면 자동 설치)
3. backend 빌드: cd backend && cargo build
   (.cargo/config.toml에 의해 wasm32-wasip2 타겟)
4. Wasmtime으로 backend.wasm 로드 (메모리에 유지)
5. Node.js SSR 서버 시작 (자식 프로세스, 내부 포트)
6. CLI 프록시 서버 시작 (포트 3000)
7. 파일 감시 시작 (notify)
```

### 3.5 Watcher (감시자)

- **라이브러리:** `notify`
- **감시 대상:** `backend/src/routes/**/props.rs`
- **Debounce:** 300ms (연속 저장 시 불필요한 트리거 방지)
- **동작:** 파일 변경 감지 시 Parser → Generator 파이프라인 실행

### 3.6 Hot Reload 흐름

```
props.rs 변경 감지
       │
       ▼
TypeScript 생성 (props.gen.ts)
       │
       ▼
cd backend && cargo build
       │
       ▼
WasmRuntime.reload(new_wasm_path)  ← 기존 인스턴스 교체
       │
       ▼
WebSocket으로 브라우저에 새로고침 신호
```

**WASM 핫 리로드**: Wasmtime Component를 메모리에서 교체합니다. 프로세스 재시작 없이 새 WASM 모듈 로드가 가능합니다.

### 3.7 Parser (분석기)

- **라이브러리:** `syn`
- **추출 정보:**

| 대상                       | 추출 내용                |
| -------------------------- | ------------------------ |
| `*Path` 구조체             | URL Path Parameter 필드  |
| `*Query` 구조체            | Query String 필드        |
| `*Header` 구조체           | HTTP Header 필드         |
| `PageProps` 구조체         | 응답 데이터 필드 및 타입 |
| `ActionInput` 구조체       | POST 요청 바디 필드      |
| `get_props` 함수           | 인자 목록 및 반환 타입   |
| `post_action` 함수         | 인자 목록 및 반환 타입   |
| `#[serde(...)]` 어트리뷰트 | rename, skip 처리        |

### 3.8 Generator (생성기)

**TypeScript 생성 (`props.gen.ts`):**

```typescript
// [Generated] Do not edit manually
export interface PageProps {
  id: number;
  name: string;
  description: string | null;
  tags: string[];
}

export interface ActionInput {
  title: string;
  content: string;
}
```

**Rust Router 생성 (`router.rs`):**

```rust
// .generated/backend/router.rs
use wstd::http::body::IncomingBody;
use wstd::http::server::{Finished, Responder};
use wstd::http::{Request, Response, StatusCode};
use std::future::Future;
use std::pin::Pin;

pub struct Router {
    routes: Vec<Route>,
}

type Handler = fn(Request<IncomingBody>, Responder) -> Pin<Box<dyn Future<Output = Finished> + Send>>;

struct Route {
    method: &'static str,
    pattern: PathPattern,
    handler: Handler,
}

impl Router {
    pub fn new() -> Self {
        let mut router = Self { routes: vec![] };

        // CLI가 자동 생성
        router.add("GET", "/", |req, res| Box::pin(routes::index::wrapper_get(req, res)));
        router.add("GET", "/product/:id", |req, res| Box::pin(routes::product::_id_::wrapper_get(req, res)));
        router.add("POST", "/product/:id", |req, res| Box::pin(routes::product::_id_::wrapper_post(req, res)));

        router
    }

    pub async fn handle(&self, request: Request<IncomingBody>, res: Responder) -> Finished {
        let path = request.uri().path();
        let method = request.method().as_str();

        match self.match_route(method, path) {
            Some((route, params)) => {
                let mut req = request;
                req.extensions_mut().insert(params);
                (route.handler)(req, res).await
            }
            None => {
                self.not_found(res).await
            }
        }
    }

    async fn not_found(&self, res: Responder) -> Finished {
        let response = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".into())
            .unwrap();
        res.respond(response).await
    }
}
```

---

## 4. Backend 구현

### 4.1 wstd 기반 wasi-http

`wstd` 크레이트를 사용하여 비동기 wasi-http 서버를 구현합니다.

**의존성 (Cargo.toml):**

```toml
[package]
name = "backend"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "forte:backend"
proxy = true

[dependencies]
wit-bindgen-rt = { version = "0.41.0", features = ["bitflags"] }
wstd = "0.5.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
validator = { version = "0.18", features = ["derive"] }
anyhow = "1.0"
```

**Cargo 기본 타겟 설정 (`backend/.cargo/config.toml`):**

```toml
[build]
target = "wasm32-wasip2"

[target.wasm32-wasip2]
runner = "wasmtime -Shttp"
```

이 설정으로 `--target` 플래그 없이도 항상 wasip2로 빌드됩니다:

```bash
cargo build           # → target/wasm32-wasip2/debug/backend.wasm
cargo build --release # → target/wasm32-wasip2/release/backend.wasm
```

**생성되는 메인 엔트리포인트:**

```rust
// .generated/backend/lib.rs
use wstd::http::body::IncomingBody;
use wstd::http::server::{Finished, Responder};
use wstd::http::{Request, Response};

#[wstd::http_server]
async fn main(req: Request<IncomingBody>, res: Responder) -> Finished {
    let router = crate::router::create_router();
    router.handle(req, res).await
}
```

### 4.2 Wrapper 핸들러 생성

```rust
// .generated/backend/routes/product/_id_/wrapper.rs
use crate::routes::product::_id_::props::{ProductPath, PageProps, get_props};
use wstd::http::body::IncomingBody;
use wstd::http::server::{Finished, Responder};
use wstd::http::{Request, Response, StatusCode};

pub async fn wrapper_get(req: Request<IncomingBody>, res: Responder) -> Finished {
    // 1. Path 파라미터 추출
    let params = req.extensions().get::<PathParams>().unwrap();
    let path = ProductPath {
        id: params.get("id").unwrap().parse().unwrap(),
    };

    // 2. 사용자 함수 호출 (비동기)
    let result = get_props(path).await;

    // 3. 응답 생성
    match result {
        Ok(props) => {
            let json = serde_json::to_string(&props).unwrap();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(json.into())
                .unwrap();
            res.respond(response).await
        }
        Err(e) => {
            let error_json = serde_json::to_string(&e).unwrap();
            let response = Response::builder()
                .status(e.status_code())
                .header("content-type", "application/json")
                .body(error_json.into())
                .unwrap();
            res.respond(response).await
        }
    }
}
```

### 4.3 사용자 코드 (비동기 지원)

사용자가 작성하는 코드는 **완전한 async/await**을 사용합니다:

```rust
// backend/src/routes/product/_id_/props.rs
use anyhow::{Context, Result};

pub struct ProductPath {
    pub id: i32,
}

pub struct PageProps {
    pub id: i32,
    pub name: String,
}

// 비동기 함수 사용 가능
pub async fn get_props(path: ProductPath) -> Result<PageProps> {
    let product_name = fetch_product_name(path.id).await?;

    Ok(PageProps {
        id: path.id,
        name: product_name,
    })
}

// wstd를 사용한 HTTP 요청
async fn fetch_product_name(id: i32) -> Result<String> {
    use wstd::http::Client;

    let client = Client::new();
    let response = client
        .get(format!("https://api.example.com/products/{}", id))
        .send()
        .await
        .context("Failed to fetch product")?;

    let body = response.text().await?;
    Ok(body)
}

// sleep, task 등 비동기 유틸리티도 사용 가능
pub async fn slow_operation() -> Result<()> {
    wstd::task::sleep(std::time::Duration::from_secs(1)).await;
    Ok(())
}
```

**wstd가 제공하는 기능:**

- ✅ 완전한 async/await 지원
- ✅ `wstd::http::Client` - HTTP 클라이언트
- ✅ `wstd::task::sleep` - 비동기 sleep
- ✅ `wstd::task::spawn` - 태스크 스폰
- ✅ 스트리밍 요청/응답 바디

---

## 5. 타입 시스템

### 5.1 기본 타입 매핑

| Rust                      | TypeScript          |
| ------------------------- | ------------------- |
| `String`, `&str`          | `string`            |
| `i8`, `i16`, `i32`, `i64` | `number`            |
| `u8`, `u16`, `u32`, `u64` | `number`            |
| `f32`, `f64`              | `number`            |
| `bool`                    | `boolean`           |
| `Option<T>`               | `T \| null`         |
| `Vec<T>`                  | `T[]`               |
| `HashMap<String, T>`      | `Record<string, T>` |

### 5.2 외부 크레이트 타입 매핑

`Forte.toml`에서 설정:

```toml
[type_mappings]
"chrono::DateTime<Utc>" = "string"
"chrono::NaiveDate" = "string"
"uuid::Uuid" = "string"
"rust_decimal::Decimal" = "string"
```

### 5.3 Enum 변환

```rust
// Rust
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Status {
    Pending,
    Active { since: String },
    Inactive,
}
```

```typescript
// Generated TypeScript
export type Status =
  | { type: "Pending" }
  | { type: "Active"; since: string }
  | { type: "Inactive" };
```

### 5.4 중첩 구조체

같은 파일 내 정의된 구조체는 자동으로 함께 변환:

```rust
pub struct Author {
    pub name: String,
}

pub struct PageProps {
    pub title: String,
    pub author: Author,
}
```

```typescript
export interface Author {
  name: string;
}

export interface PageProps {
  title: string;
  author: Author;
}
```

### 5.5 Serde 어트리뷰트 지원

```rust
pub struct PageProps {
    #[serde(rename = "userName")]
    pub user_name: String,

    #[serde(skip)]
    pub internal_id: i32,  // TypeScript에 포함되지 않음
}
```

---

## 6. 에러 핸들링

### 6.1 기본 원칙

Forte는 3단계 에러 시스템을 제공합니다:

1. **Internal Server Error (500)**: `anyhow::Error` - 서버 내부 에러 (DB 연결 실패, 파일 읽기 실패 등)
2. **Validation Error (400)**: `ActionInput`의 `#[validate(...)]` 실패 - 입력값 검증 에러
3. **Business Logic Response/Error (200)**: POST action의 `Result<Response, Error>` - 비즈니스 로직 성공/실패

### 6.2 GET 요청 에러 처리

`get_props`는 `anyhow::Result<PageProps>`를 반환:

```rust
use anyhow::{Context, Result};

pub async fn get_props(path: ProductPath) -> Result<PageProps> {
    let product = db::find_product(path.id)
        .await
        .context("Failed to query database")?;

    Ok(PageProps {
        id: product.id,
        name: product.name,
    })
}
```

**에러 처리:**

- `Err(anyhow::Error)` → **500 Internal Server Error**
- `Ok(PageProps)` → **200 OK + PageProps JSON**

**프론트엔드 (SSR):**

```tsx
// GET 요청은 SSR에서 처리되므로 에러 페이지로 렌더링
// 500 에러 시 error.tsx 표시
```

### 6.3 POST 요청 에러 처리

`post_action`은 3단계 에러를 처리합니다:

```rust
use anyhow::{Context, Result};
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct ActionInput {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

// CLI가 자동 생성 (유저가 variant 추가)
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Response {
    Success { token: String },
}

// CLI가 자동 생성 (유저가 variant 추가)
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Error {
    InvalidCredentials { message: String },
}

pub async fn post_action(input: ActionInput) -> Result<Result<Response, Error>> {
    // 1. Validation은 wrapper에서 자동 처리 (400)

    // 2. Internal error는 ? 로 처리 (500)
    let user = db::find_user(&input.email)
        .await
        .context("Database query failed")?;

    // 3. Business logic result (200)
    match authenticate(&user, &input.password).await {
        Ok(token) => Ok(Ok(Response::Success { token })),
        Err(_) => Ok(Err(Error::InvalidCredentials {
            message: "Invalid email or password".into()
        })),
    }
}
```

### 6.4 Wrapper 에러 처리 로직

CLI가 생성하는 wrapper는 자동으로 3단계 에러를 처리합니다:

```rust
// .generated/backend/routes/login/wrapper.rs
pub async fn wrapper_post(req: Request<IncomingBody>, res: Responder) -> Finished {
    // 1. 요청 바디 파싱
    let body = parse_body(&req).await;

    // 2. ActionInput 역직렬화
    let input: ActionInput = match serde_json::from_slice(&body) {
        Ok(input) => input,
        Err(e) => return send_error_400("Invalid JSON", res).await,
    };

    // 3. Validation 체크 (#[derive(Validate)] 있을 경우)
    if let Err(errors) = input.validate() {
        return send_validation_error_400(errors, res).await;
    }

    // 4. post_action 호출
    match post_action(input).await {
        // Internal error (500)
        Err(anyhow_err) => {
            send_error_500(anyhow_err, res).await
        }

        // Business logic result (200)
        Ok(result) => {
            match result {
                Ok(response) => send_json_200(response, res).await,
                Err(error) => send_json_200(error, res).await,
            }
        }
    }
}
```

### 6.5 에러 응답 포맷

**1. Internal Server Error (500):**

```json
{
  "error": "Internal Server Error",
  "message": "Database query failed"
}
```

**2. Validation Error (400):**

```json
{
  "error": "Validation Error",
  "errors": [
    {
      "field": "email",
      "message": "Invalid email format"
    },
    {
      "field": "password",
      "message": "Must be at least 8 characters"
    }
  ]
}
```

**3. Business Logic Response (200):**

```json
{
  "type": "Success",
  "token": "eyJhbGciOiJIUzI1NiIs..."
}
```

**4. Business Logic Error (200):**

```json
{
  "type": "InvalidCredentials",
  "message": "Invalid email or password"
}
```

### 6.6 TypeScript 에러 처리

프론트엔드에서 POST 요청 처리:

```tsx
import type { ActionInput, Response, Error } from "./props.gen";

async function handleLogin(formData: FormData) {
  const res = await fetch("/login", {
    method: "POST",
    body: formData,
  });

  // 1. Internal Server Error 체크 (500)
  if (res.status === 500) {
    const { error, message } = await res.json();
    alert(`Server Error: ${message}`);
    return;
  }

  // 2. Validation Error 체크 (400)
  if (res.status === 400) {
    const { errors } = await res.json();
    setValidationErrors(errors);
    return;
  }

  // 3. Business Logic Response/Error (200)
  if (res.status === 200) {
    const data: Response | Error = await res.json();

    if (data.type === "Success") {
      localStorage.setItem("token", data.token);
      navigate("/dashboard");
    } else if (data.type === "InvalidCredentials") {
      setError(data.message);
    }
  }
}
```

### 6.7 개발 모드 에러 오버레이

- Rust 컴파일 에러 시 브라우저에 오버레이 표시
- `anyhow::Error` 발생 시 상세 스택트레이스 표시 (500)
- Validation 에러는 일반 JSON 응답 (400)
- WASM 로드 실패 시 재시도 안내

---

## 7. 레이아웃 시스템

### 7.1 기본 구조

```
frontend/src/app/
├── layout.tsx              ← 모든 페이지에 적용
├── page.tsx
└── dashboard/
    ├── layout.tsx          ← /dashboard/* 에 적용
    └── settings/
        └── page.tsx        ← 두 레이아웃 모두 적용
```

### 7.2 레이아웃 컴포넌트

```tsx
// layout.tsx
interface LayoutProps {
  children: React.ReactNode;
}

export default function Layout({ children }: LayoutProps) {
  return (
    <html>
      <body>
        <nav>...</nav>
        <main>{children}</main>
      </body>
    </html>
  );
}
```

### 7.3 특징

- 레이아웃은 `props`를 받지 않음 (서버 데이터 페칭 없음)
- 레이아웃에서 데이터가 필요하면 클라이언트 컴포넌트로 자체 fetch
- 중첩 시 외부 → 내부 순서로 래핑

---

## 8. Post Action

### 8.1 백엔드 정의

```rust
// backend/src/routes/login/props.rs
use anyhow::Result;

pub struct PageProps {
    // GET 요청 시 반환되는 데이터
}

pub struct ActionInput {
    pub email: String,
    pub password: String,
}

// CLI가 자동 생성하는 enum (유저가 variant 추가 가능)
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Response {
    Success { token: String },
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Error {
    InvalidCredentials { message: String },
}

pub async fn get_props() -> Result<PageProps> {
    Ok(PageProps {})
}

// POST action: anyhow::Result<Result<Response, Error>>
pub async fn post_action(input: ActionInput) -> Result<Result<Response, Error>> {
    let user = db::find_user(&input.email).await?;

    match authenticate(&user, &input.password).await {
        Ok(token) => Ok(Ok(Response::Success { token })),
        Err(_) => Ok(Err(Error::InvalidCredentials {
            message: "Invalid email or password".into()
        })),
    }
}
```

### 8.2 프론트엔드 사용

```tsx
// frontend/src/app/login/page.tsx
import type { PageProps, ActionInput, Response, Error } from "./props.gen";
import { useState } from "react";

export default function LoginPage(props: PageProps) {
  const [result, setResult] = useState<Response | Error | null>(null);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const formData = new FormData(e.target as HTMLFormElement);

    const res = await fetch("/login", {
      method: "POST",
      body: formData,
    });

    const data = await res.json();
    setResult(data);
  };

  return (
    <form onSubmit={handleSubmit}>
      <input name="email" type="email" required />
      <input name="password" type="password" required />

      {result && (
        <>
          {result.type === "InvalidCredentials" && (
            <p className="error">{result.message}</p>
          )}
          {result.type === "Success" && (
            <p className="success">Login successful!</p>
          )}
        </>
      )}

      <button type="submit">Login</button>
    </form>
  );
}
```

### 8.3 Validation (선택)

`validator` 크레이트 derive 매크로 지원:

```rust
use validator::Validate;

#[derive(Validate)]
pub struct ActionInput {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}
```

Validation 실패 시 자동으로 400 응답과 에러 메시지 반환.

---

## 9. 클라이언트 내비게이션

### 9.1 기본 동작

기본은 MPA (풀 페이지 리로드). 단순하고 예측 가능.

### 9.2 옵트인 SPA 내비게이션

```tsx
import { Link } from "forte/client";

export default function Navigation() {
  return (
    <nav>
      <Link href="/">Home</Link>
      <Link href="/product/1" prefetch>
        Product
      </Link>
    </nav>
  );
}
```

### 9.3 Link 컴포넌트 동작

1. 클릭 시 `e.preventDefault()`
2. `/api/product/1` (backend.wasm)로 직접 fetch
3. 해당 `page.tsx`를 동적 import
4. 클라이언트에서 렌더링
5. `history.pushState`로 URL 업데이트

### 9.4 Prefetch

`prefetch` 속성이 있으면 viewport에 들어올 때 미리 데이터와 JS 청크를 로드.

---

## 10. 환경 설정

### 10.1 파일 구조

```
my-project/
├── .env                    ← 공통 (gitignore 권장)
├── .env.development        ← 개발 환경
├── .env.production         ← 프로덕션 환경
└── .env.example            ← 템플릿 (git 포함)
```

### 10.2 Forte.toml 스키마

```toml
[env]
required = ["DATABASE_URL", "JWT_SECRET"]
optional = ["REDIS_URL", "LOG_LEVEL", "SENTRY_DSN"]

[env.defaults]
LOG_LEVEL = "info"
PORT = "3000"

[type_mappings]
"chrono::DateTime<Utc>" = "string"
"uuid::Uuid" = "string"

[proxy]
forward_headers = ["Cookie", "Authorization", "Accept-Language"]
timeout_ms = 5000

[build]
output_dir = "dist"
```

### 10.3 타입 안전한 환경 변수 접근

CLI가 생성하는 코드:

```rust
// .generated/backend/env.rs
pub struct Env {
    pub database_url: String,
    pub jwt_secret: String,
    pub redis_url: Option<String>,
    pub log_level: String,
}

lazy_static! {
    pub static ref ENV: Env = Env::load();
}
```

사용:

```rust
use crate::generated::env::ENV;

pub async fn get_props() -> PageProps {
    let conn = db::connect(&ENV.database_url).await;
    // ...
}
```

### 10.4 환경 검증

`forte dev` 또는 `forte build` 실행 시 required 변수 체크:

```
Error: Missing required environment variables:
  - DATABASE_URL
  - JWT_SECRET

Please set them in .env or .env.development
```

---

## 11. 프로덕션 빌드

### 11.1 빌드 명령어

```bash
forte build
```

### 11.2 빌드 프로세스

1. 환경 변수 검증 (`.env.production`)
2. `cd backend && cargo build --release` (config.toml에 의해 wasm32-wasip2 타겟)
3. wasm-opt 최적화 (선택적)
4. `vite build` (클라이언트 에셋)
5. `vite build --ssr` (SSR 번들)
6. 결과물 조립

### 11.3 빌드 결과물

```
dist/
├── backend.wasm            ← WASM 컴포넌트
├── server/
│   └── index.js            ← Vite SSR 번들
├── client/
│   ├── assets/
│   │   ├── index-[hash].js
│   │   └── index-[hash].css
│   └── .vite/
│       └── manifest.json
└── static/                 ← public 폴더 복사본
```

**네이티브 바이너리 없음**: 프로덕션 결과물은 순수하게 wasm + JS 번들만 포함됩니다.

### 11.4 프로덕션 실행 옵션

**옵션 1: forte serve (개발 머신 또는 자체 서버)**

```bash
forte serve ./dist

# 내부 동작:
# 1. backend.wasm을 Wasmtime으로 로드
# 2. Node.js로 server/index.js 실행
# 3. 프록시 서버 시작
```

**옵션 2: 엣지 플랫폼 배포**

```bash
# Cloudflare Workers
wrangler deploy dist/backend.wasm

# Fastly Compute
fastly compute publish
```

**옵션 3: 컨테이너 배포**

```dockerfile
FROM node:20-slim

# Wasmtime 설치
RUN curl https://wasmtime.dev/install.sh -sSf | bash

COPY dist/ /app/
WORKDIR /app

CMD ["node", "run.js"]
```

---

## 12. wasi-sdk 및 빌드 환경

### 12.1 기본 환경 (wasi-sdk 불필요)

순수 Rust 의존성만 사용하는 경우:

```bash
# 타겟 추가 (최초 1회)
rustup target add wasm32-wasip2

# .cargo/config.toml이 타겟을 지정하므로 그냥 빌드
cargo build
```

### 12.2 C 의존성이 있는 경우

`ring`, `openssl` 등 C 코드가 포함된 크레이트 사용 시:

**자동 설치 (forte dev/build 최초 실행 시):**

```rust
// forte-cli/src/toolchain/wasi_sdk.rs
pub fn ensure_wasi_sdk() -> Result<PathBuf> {
    let sdk_path = dirs::cache_dir()?.join("forte/wasi-sdk-29");

    if !sdk_path.exists() {
        let url = detect_wasi_sdk_url();  // OS/arch 감지
        download_and_extract(url, &sdk_path)?;
    }

    Ok(sdk_path)
}

fn detect_wasi_sdk_url() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "aarch64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-arm64-linux.tar.gz",
        ("linux", "x86_64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-x86_64-linux.tar.gz",
        ("macos", "aarch64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-arm64-macos.tar.gz",
        ("macos", "x86_64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-x86_64-macos.tar.gz",
        ("windows", "x86_64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-x86_64-windows.tar.gz",
        ("windows", "aarch64") =>
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-arm64-windows.tar.gz",
        _ => panic!("Unsupported platform"),
    }
}
```

**환경 변수 자동 설정:**

```rust
fn setup_build_env(sdk_path: &Path) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();

    env.insert("CC".into(), sdk_path.join("bin/clang").display().to_string());
    env.insert("CXX".into(), sdk_path.join("bin/clang++").display().to_string());
    env.insert("AR".into(), sdk_path.join("bin/llvm-ar").display().to_string());
    env.insert("CFLAGS".into(), format!("--sysroot={}", sdk_path.join("share/wasi-sysroot").display()));

    env
}
```

---

## 13. WASM 환경 제약사항

### 13.1 비동기 처리

**wstd를 사용하면 완전한 async/await을 지원합니다!**

```rust
// 외부 API 호출
use wstd::http::Client;

pub async fn fetch_external(url: &str) -> Result<Vec<u8>> {
    let client = Client::new();
    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    Ok(bytes)
}

// 병렬 요청
pub async fn fetch_multiple() -> Result<Vec<String>> {
    use wstd::task;

    let task1 = task::spawn(async { fetch_user(1).await });
    let task2 = task::spawn(async { fetch_user(2).await });

    let (user1, user2) = tokio::try_join!(task1, task2)?;
    Ok(vec![user1, user2])
}

// 타임아웃
pub async fn fetch_with_timeout(url: &str) -> Result<String> {
    use wstd::time::{timeout, Duration};

    timeout(Duration::from_secs(5), async {
        let client = Client::new();
        let response = client.get(url).send().await?;
        response.text().await
    })
    .await
    .map_err(|_| anyhow::anyhow!("Request timeout"))?
}
```

### 13.2 지원되지 않는 기능

| 기능                 | 대안                                     |
| -------------------- | ---------------------------------------- |
| 직접 TCP/UDP 소켓    | `wstd::http::Client` 사용 (HTTP만)       |
| 파일시스템 직접 접근 | CLI에서 파일 내용을 환경변수로 주입      |
| 네이티브 스레딩      | `wstd::task::spawn` 사용 (green threads) |

### 13.3 호환 크레이트 가이드

**완벽 호환 (권장):**

- ✅ `serde`, `serde_json` - 완벽 작동
- ✅ `validator` - 완벽 작동
- ✅ `chrono` - 대부분 기능 작동
- ✅ `uuid` - 완벽 작동
- ✅ `regex` - 완벽 작동
- ✅ `wstd` - WASI 환경용 async 런타임
- ✅ `anyhow` - 에러 처리

**HTTP 기반으로 전환 필요:**

- ⚠️ `sqlx` → HTTP 기반 DB 사용 (PlanetScale, Turso, Neon 등)
- ⚠️ `reqwest` → `wstd::http::Client` 사용
- ⚠️ `redis` → HTTP 기반 Redis (Upstash) 사용

**사용 불가 (대안 있음):**

- ❌ `tokio` → `wstd` 사용 (동일한 async/await 문법)
- ❌ `diesel` → HTTP 기반 DB 드라이버 사용
- ❌ 직접 소켓 라이브러리 → HTTP API로 변환

**C 의존성 크레이트 (wasi-sdk 필요):**

- `ring`, `openssl` - 가능하지만 wasi-sdk 자동 설치 필요

---

## 14. 개발 경험 최적화

### 14.1 Hot Reload 전략

**Rust (백엔드):**

- 파일 변경 감지 시 증분 컴파일
- `.generated/backend`를 별도 크레이트로 분리하여 컴파일 범위 최소화
- WASM 핫 리로드: Wasmtime Component를 메모리에서 교체 (프로세스 재시작 불필요)
- 컴파일 중 상태를 프론트엔드에 전달

**React (프론트엔드):**

- Vite HMR 활용
- `props.gen.ts` 변경 시 자동 리로드

### 14.2 동기화 메커니즘

```
CLI 내부 상태:
- backend_compiling: bool
- backend_ready: bool
- last_error: Option<String>
```

백엔드 컴파일 중 동작:

1. 새 요청 수신
2. `backend_compiling == true` 확인
3. 503 응답 + "Compiling..." 메시지
4. 컴파일 완료 시 WebSocket으로 클라이언트에 새로고침 신호

### 14.3 에러 표시

컴파일 에러 발생 시 브라우저 오버레이:

```
┌─────────────────────────────────────┐
│  ❌ Rust Compilation Error          │
│                                     │
│  backend/src/routes/product/        │
│  _id_/props.rs:15:9                 │
│                                     │
│  error[E0308]: mismatched types     │
│    expected `String`                │
│    found `i32`                      │
│                                     │
│  [Waiting for file changes...]      │
└─────────────────────────────────────┘
```

---

## 15. 테스트 전략

### 15.1 CLI 테스트

```rust
// CLI 내부 테스트
#[test]
fn test_rust_to_ts_conversion() {
    let rust = r#"pub struct PageProps { pub name: String }"#;
    let ts = parser::convert_to_typescript(rust);
    assert_eq!(ts, "export interface PageProps {\n  name: string;\n}");
}

#[test]
fn test_route_path_parsing() {
    let path = "routes/user/_userId_/post/_postId_";
    let url = router::path_to_url(path);
    assert_eq!(url, "/user/:userId/post/:postId");
}
```

### 15.2 사용자 프로젝트 테스트

```bash
forte test
# 내부적으로 실행:
# 1. cargo test (backend)
# 2. vitest run (frontend)
```

**백엔드 테스트:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_props() {
        let path = ProductPath { id: 1 };
        let result = get_props(path).await;
        assert!(result.is_ok());
    }
}
```

**프론트엔드 테스트:**

```tsx
import { render, screen } from "@testing-library/react";
import Page from "./page";

test("renders product name", () => {
  render(<Page id={1} name="Test Product" />);
  expect(screen.getByText("Test Product")).toBeInTheDocument();
});
```

---

## 16. 개발 로드맵

### Phase 0: 프로젝트 스캐폴딩 (1주)

- [ ] `forte init` 명령어 구현
- [ ] 템플릿 파일 임베딩 (`include_str!`)
- [ ] 기본 디렉토리 구조 생성
- [ ] `Forte.toml` 기본 설정
- [ ] `.cargo/config.toml` 생성 (wasm32-wasip2 타겟 설정)

**완료 기준:** `forte init my-app`으로 빈 프로젝트 생성 가능

### Phase 1: 타입 생성기 (2주)

- [ ] `syn`으로 Rust 구조체 파싱
- [ ] TypeScript 인터페이스 변환
- [ ] `notify`로 파일 감시
- [ ] Snake_case → camelCase 변환
- [ ] `Option`, `Vec` 타입 처리
- [ ] `#[serde(rename)]`, `#[serde(skip)]` 지원

**완료 기준:** `props.rs` 저장 시 `props.gen.ts` 자동 생성

### Phase 2: 백엔드 서버 (2주)

- [ ] 라우트 스캔 (`_id_` → `:id` 변환)
- [ ] `wstd` 기반 비동기 wasi-http 서버 생성
- [ ] 비동기 라우터 구현 (wstd 위)
- [ ] Wrapper 핸들러 코드 생성
- [ ] `router.rs`, `lib.rs` 생성
- [ ] Wasmtime 임베딩 및 런타임 관리
- [ ] 환경 변수 로딩 및 검증

**완료 기준:** `forte dev`로 API 서버 실행, JSON 응답 확인

### Phase 2.5: CLI 프록시 (1주)

- [ ] Hyper 기반 프록시 서버 구현
- [ ] 요청 분류 로직 (Static/API/Page)
- [ ] Wasmtime ↔ HTTP 변환 레이어
- [ ] WASM 핫 리로드 메커니즘

**완료 기준:** CLI가 모든 요청을 적절히 라우팅

### Phase 3: 프론트엔드 SSR (3주)

- [ ] Vite 설정 템플릿
- [ ] Node.js 렌더링 서버
- [ ] CLI 프록시 ↔ Node.js 통신
- [ ] Header 전달 로직
- [ ] `renderToString` 연동
- [ ] Hydration 스크립트
- [ ] `layout.tsx` 지원
- [ ] `error.tsx` 지원

**완료 기준:** 브라우저에서 SSR 페이지 렌더링 및 하이드레이션

### Phase 4: 인터랙션 (2주)

- [ ] Post Action 지원 (`post_action` 함수)
- [ ] `ActionInput` 타입 생성
- [ ] `Response`, `Error` enum 처리
- [ ] Validation 연동
- [ ] `<Link>` 컴포넌트 (클라이언트 내비게이션)
- [ ] Prefetch 기능

**완료 기준:** 폼 제출 및 클라이언트 내비게이션 동작

### Phase 5: 프로덕션 (2주)

- [ ] `forte build` 명령어
- [ ] `cargo build --release` (wasm32-wasip2 타겟)
- [ ] wasm-opt 최적화 (선택적)
- [ ] Vite 빌드 통합
- [ ] Manifest 기반 에셋 주입
- [ ] 정적 파일 서빙
- [ ] `forte serve` 명령어 추가
- [ ] Dockerfile 템플릿
- [ ] `forte test` 명령어

**완료 기준:** 프로덕션 배포 가능한 결과물 생성

---

## 17. 사용자 워크플로우 예시

### 17.1 프로젝트 시작

```bash
# 1. 프로젝트 생성
forte init my-blog
cd my-blog

# 2. 개발 서버 실행
forte dev
```

### 17.2 새 페이지 추가

```bash
# 3. 백엔드 로직 작성
mkdir -p backend/src/routes/post/_id_
```

```rust
// backend/src/routes/post/_id_/props.rs
use anyhow::Result;

pub struct PostPath {
    pub id: i32,
}

pub struct PageProps {
    pub id: i32,
    pub title: String,
    pub content: String,
    pub author: String,
}

pub async fn get_props(path: PostPath) -> Result<PageProps> {
    // 실제로는 DB 조회
    Ok(PageProps {
        id: path.id,
        title: "Hello World".into(),
        content: "This is my first post.".into(),
        author: "Alice".into(),
    })
}
```

```bash
# 4. 저장하면 자동 생성됨
# frontend/src/app/post/[id]/props.gen.ts
```

```tsx
// 5. UI 작성
// frontend/src/app/post/[id]/page.tsx
import type { PageProps } from "./props.gen";

export default function PostPage({ title, content, author }: PageProps) {
  return (
    <article>
      <h1>{title}</h1>
      <p>By {author}</p>
      <div>{content}</div>
    </article>
  );
}
```

```
# 6. 브라우저에서 확인
http://localhost:3000/post/1
```

### 17.3 폼 추가

```rust
// backend/src/routes/post/new/props.rs
use anyhow::Result;

pub struct PageProps {
    pub categories: Vec<String>,
}

pub struct ActionInput {
    pub title: String,
    pub content: String,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Response {
    Success { id: i32 },
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Error {
    ValidationFailed { message: String },
}

pub async fn get_props() -> Result<PageProps> {
    Ok(PageProps {
        categories: vec!["Tech".into(), "Life".into()],
    })
}

pub async fn post_action(input: ActionInput) -> Result<Result<Response, Error>> {
    match create_post(&input.title, &input.content).await {
        Ok(post) => Ok(Ok(Response::Success { id: post.id })),
        Err(e) => Ok(Err(Error::ValidationFailed {
            message: e.to_string(),
        })),
    }
}
```

```tsx
// frontend/src/app/post/new/page.tsx
import type { PageProps, Response, Error } from "./props.gen";
import { useState } from "react";

export default function NewPostPage({ categories }: PageProps) {
  const [result, setResult] = useState<Response | Error | null>(null);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const formData = new FormData(e.target as HTMLFormElement);
    const res = await fetch("/post/new", { method: "POST", body: formData });
    setResult(await res.json());
  };

  return (
    <form onSubmit={handleSubmit}>
      <input name="title" placeholder="Title" required />
      <textarea name="content" placeholder="Content" required />

      {result?.type === "ValidationFailed" && (
        <p className="error">{result.message}</p>
      )}
      {result?.type === "Success" && (
        <p className="success">Post created! ID: {result.id}</p>
      )}

      <button type="submit">Create Post</button>
    </form>
  );
}
```

### 17.4 프로덕션 배포

```bash
# 빌드
forte build

# 실행 옵션 1: forte serve
forte serve ./dist

# 실행 옵션 2: Docker
docker build -t my-blog .
docker run -p 3000:3000 my-blog

# 실행 옵션 3: 엣지 배포
wrangler deploy dist/backend.wasm
```

---

## 18. 배포 옵션

WASM 기반이므로 다양한 플랫폼 배포 가능:

| 플랫폼             | 지원 방식                                   |
| ------------------ | ------------------------------------------- |
| 자체 서버          | `forte serve ./dist` 실행                   |
| Docker             | Wasmtime + Node.js 컨테이너                 |
| Cloudflare Workers | backend.wasm 직접 배포 (Node SSR 분리 필요) |
| Fastly Compute     | backend.wasm 직접 배포                      |
| Fermyon Cloud      | Spin 호환 모드 제공 시                      |

**참고**: 엣지 배포 시 Node.js SSR을 어떻게 처리할지는 추가 설계가 필요합니다. 클라이언트 사이드 렌더링 전용 모드 또는 별도의 SSR 서비스 분리 등을 고려할 수 있습니다.

---

## 19. 제약 사항 및 명시적 비지원

다음 기능은 의도적으로 지원하지 않습니다:

| 기능                           | 이유                                    |
| ------------------------------ | --------------------------------------- |
| Catch-all 라우팅 (`[...slug]`) | 복잡도 증가, 대부분의 케이스에서 불필요 |
| 내장 미들웨어 시스템           | 사용자가 Rust 레벨에서 직접 구현        |
| GraphQL                        | REST JSON API에 집중                    |
| 다국어(i18n) 내장              | 사용자 구현 영역                        |
| 내장 ORM                       | 사용자가 선호하는 라이브러리 사용       |
| 직접 TCP/UDP 소켓              | WASM 환경 제약, HTTP API로 대체         |
| 파일시스템 직접 접근           | 환경변수 주입으로 대체                  |

---

## 20. 기술 스택 요약

| 구성 요소          | 기술             | 버전  |
| ------------------ | ---------------- | ----- |
| CLI                | Rust + Wasmtime  | 1.75+ |
| Parser             | syn              | 2.x   |
| File Watcher       | notify           | 6.x   |
| Backend Runtime    | wstd (wasi-http) | 0.5+  |
| Backend Bindings   | wit-bindgen      | 0.41+ |
| Frontend Framework | React            | 18+   |
| Bundler            | Vite             | 5.x   |
| SSR Runtime        | Node.js          | 20+   |
| Validation         | validator        | 0.18+ |
| Error Handling     | anyhow           | 1.0+  |

---
