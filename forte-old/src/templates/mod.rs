pub mod error_boundary;
pub mod frontend_ssr;

// Root configuration files

pub const FORTE_TOML: &str = r#"[env]
required = ["DATABASE_URL"]
optional = ["REDIS_URL", "LOG_LEVEL"]

[env.defaults]
LOG_LEVEL = "info"
PORT = "3000"
RUST_PORT = "8080"

[type_mappings]
"chrono::DateTime<Utc>" = "string"
"chrono::NaiveDate" = "string"
"uuid::Uuid" = "string"

[proxy]
forward_headers = ["Cookie", "Authorization", "Accept-Language"]
timeout_ms = 5000

[build]
output_dir = "dist"
"#;

pub const ENV_TEMPLATE: &str = r#"# Add your environment variables here
# This file should be in .gitignore
DATABASE_URL=
"#;

pub const ENV_DEV: &str = r#"# Development environment variables
DATABASE_URL=postgres://localhost/myapp_dev
LOG_LEVEL=debug
PORT=3000
RUST_PORT=8080
"#;

pub const ENV_PROD: &str = r#"# Production environment variables
DATABASE_URL=
LOG_LEVEL=info
PORT=3000
RUST_PORT=8080
"#;

pub const GITIGNORE: &str = r#"# Rust
target/
Cargo.lock

# Node
node_modules/
dist/
.vite/

# Environment
.env
.env.local

# Generated
.generated/

# IDE
.vscode/
.idea/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db
"#;

pub const DOCKERFILE: &str = r#"# Multi-stage Docker build for Forte application
# Stage 1: Build Rust WASM backend
FROM rust:1.85-slim as rust-builder

# Install WASM target
RUN rustup target add wasm32-wasip2

WORKDIR /app

# Copy backend source
COPY backend/ ./backend/
COPY Forte.toml ./.
COPY .generated/ ./.generated/

# Build WASM in release mode
WORKDIR /app/backend
RUN cargo build --release --target wasm32-wasip2

# Stage 2: Build Node.js frontend
FROM node:20-slim as node-builder

WORKDIR /app

# Copy frontend source
COPY frontend/ ./frontend/
COPY .generated/ ./.generated/

# Install dependencies and build
WORKDIR /app/frontend
RUN npm ci --only=production
RUN npm run build

# Stage 3: Runtime image
FROM node:20-slim

# Install wasmtime for running WASM
RUN apt-get update && \
    apt-get install -y curl && \
    curl https://wasmtime.dev/install.sh -sSf | bash && \
    mv /root/.wasmtime/bin/wasmtime /usr/local/bin/ && \
    apt-get remove -y curl && \
    apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy built artifacts
COPY --from=rust-builder /app/backend/target/wasm32-wasip2/release/backend.wasm ./backend.wasm
COPY --from=node-builder /app/frontend/dist ./public
COPY .env.production ./.env.production

# Expose port (configure via environment)
ENV PORT=3000
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD node -e "require('http').get('http://localhost:' + process.env.PORT + '/health', (r) => process.exit(r.statusCode === 200 ? 0 : 1))"

# Run the WASM backend
CMD ["wasmtime", "run", "--wasi", "preview2", "backend.wasm"]
"#;

pub const DOCKERIGNORE: &str = r#"# Build artifacts
target/
dist/
node_modules/

# Development files
.git/
.gitignore
README.md
*.md

# Generated (will be copied separately)
.generated/

# Environment (copy production only)
.env
.env.local
.env.development

# IDE
.vscode/
.idea/
*.swp
*.swo

# OS
.DS_Store
Thumbs.db
"#;

pub fn readme(project_name: &str) -> String {
    format!(
        r#"# {}

A Forte project - Full-stack Rust+React with type-safe routing.

## Getting Started

```bash
# Start development server
forte dev

# Build for production
forte build

# Run tests
forte test
```

## Project Structure

- `backend/` - Rust backend with Axum
- `frontend/` - React frontend with Vite
- `.generated/` - Auto-generated code (do not edit)

## Adding a New Page

1. Create a new route in `backend/src/routes/`:
   ```
   backend/src/routes/about/props.rs
   ```

2. Define your data types:
   ```rust
   pub struct PageProps {{
       pub message: String,
   }}

   pub async fn get_props() -> PageProps {{
       PageProps {{
           message: "Hello from Forte!".into(),
       }}
   }}
   ```

3. Create the UI in `frontend/src/app/`:
   ```tsx
   // frontend/src/app/about/page.tsx
   import type {{ PageProps }} from "./props.gen";

   export default function AboutPage({{ message }}: PageProps) {{
       return <h1>{{message}}</h1>;
   }}
   ```

4. The CLI will automatically:
   - Generate TypeScript types
   - Create routing configuration
   - Hot reload your changes

## Learn More

See the [Forte documentation](https://github.com/yourusername/forte) for more details.
"#,
        project_name
    )
}

// Backend templates

pub fn backend_cargo_toml(_project_name: &str) -> String {
    r#"[package]
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
"#.to_string()
}

pub const BACKEND_CARGO_CONFIG: &str = r#"[build]
target = "wasm32-wasip2"

[target.wasm32-wasip2]
runner = "wasmtime -Shttp"
"#;

pub const BACKEND_LIB: &str = r#"// [Generated] WASM backend entry point
// This file is auto-managed by Forte CLI

use wstd::http::body::IncomingBody;
use wstd::http::server::{Finished, Responder};
use wstd::http::Request;

pub mod routes;
mod router;

#[wstd::http_server]
async fn main(req: Request<IncomingBody>, res: Responder) -> Finished {
    // Create router and register routes
    let mut router = router::Router::new();
    router.add("GET", "/", routes::index::wrapper_get);

    // Handle the request
    router.handle(req, res).await
}
"#;

pub const BACKEND_ROUTES_MOD: &str = r#"// [Generated] Do not edit manually
// This file will be managed by the Forte CLI

pub mod index;
"#;

pub const BACKEND_INDEX_MOD: &str = r#"pub mod props;
pub mod wrapper;

pub use props::*;
pub use wrapper::*;
"#;

pub const BACKEND_ROUTER: &str = r#"// [Generated] Dynamic router for WASM backend
// This file is auto-managed by Forte CLI

use std::collections::HashMap;
use wstd::http::body::{BoundedBody, IncomingBody};
use wstd::http::server::{Finished, Responder};
use wstd::http::{IntoBody, Request, Response, StatusCode};

pub type Handler = fn(Request<IncomingBody>, Responder) -> std::pin::Pin<Box<dyn std::future::Future<Output = Finished>>>;

pub struct Router {
    routes: Vec<Route>,
}

struct Route {
    method: &'static str,
    pattern: PathPattern,
    handler: Handler,
}

#[derive(Debug)]
struct PathPattern {
    segments: Vec<Segment>,
}

#[derive(Debug)]
enum Segment {
    Static(String),
    Param(String),
}

impl PathPattern {
    fn from_path(path: &str) -> Self {
        let segments = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with(':') {
                    Segment::Param(s[1..].to_string())
                } else {
                    Segment::Static(s.to_string())
                }
            })
            .collect();

        PathPattern { segments }
    }

    fn matches(&self, path: &str) -> Option<HashMap<String, String>> {
        let path_segments: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if path_segments.len() != self.segments.len() {
            if path_segments.is_empty() && self.segments.is_empty() {
                return Some(HashMap::new());
            }
            return None;
        }

        let mut params = HashMap::new();

        for (pattern_seg, path_seg) in self.segments.iter().zip(path_segments.iter()) {
            match pattern_seg {
                Segment::Static(expected) => {
                    if expected != path_seg {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    params.insert(name.clone(), path_seg.to_string());
                }
            }
        }

        Some(params)
    }
}

impl Router {
    pub fn new() -> Self {
        Router { routes: vec![] }
    }

    pub fn add(&mut self, method: &'static str, path: &'static str, handler: Handler) {
        let pattern = PathPattern::from_path(path);
        self.routes.push(Route {
            method,
            pattern,
            handler,
        });
    }

    pub async fn handle(self, req: Request<IncomingBody>, res: Responder) -> Finished {
        let path = req.uri().path();
        let method = req.method().as_str();

        for route in &self.routes {
            if route.method != method {
                continue;
            }

            if let Some(params) = route.pattern.matches(path) {
                let (mut parts, body) = req.into_parts();
                parts.extensions.insert(params);
                let req = Request::from_parts(parts, body);

                return (route.handler)(req, res).await;
            }
        }

        let response: Response<BoundedBody<Vec<u8>>> = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".into_body())
            .unwrap();

        res.respond(response).await
    }
}

pub struct PathParams(pub HashMap<String, String>);

impl PathParams {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }
}

pub fn extract_path_params(req: &Request<IncomingBody>) -> PathParams {
    req.extensions()
        .get::<HashMap<String, String>>()
        .cloned()
        .map(PathParams)
        .unwrap_or_else(|| PathParams(HashMap::new()))
}
"#;


pub const BACKEND_INDEX_WRAPPER: &str = r#"// [Generated] Wrapper handlers
// This file is auto-managed by Forte CLI

use wstd::http::body::{BoundedBody, IncomingBody};
use wstd::http::server::{Finished, Responder};
use wstd::http::{IntoBody, Request, Response, StatusCode};
use std::pin::Pin;

pub fn wrapper_get(req: Request<IncomingBody>, res: Responder) -> Pin<Box<dyn std::future::Future<Output = Finished>>> {
    Box::pin(wrapper_get_impl(req, res))
}

async fn wrapper_get_impl(_req: Request<IncomingBody>, res: Responder) -> Finished {
    // Call user's get_props function
    let result = super::props::get_props().await;

    // Handle Result (anyhow::Result<PageProps>)
    match result {
        Ok(props) => {
            // Serialize to JSON
            let json = serde_json::to_string(&props).unwrap();

            // Return JSON response
            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(json.into_body())
                .unwrap();

            res.respond(response).await
        }
        Err(e) => {
            // System error - return 500 Internal Server Error
            let error_msg = format!("{{\"error\": \"Internal Server Error\", \"message\": \"{}\"}}", e);

            let response: Response<BoundedBody<Vec<u8>>> = Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(error_msg.into_body())
                .unwrap();

            res.respond(response).await
        }
    }
}
"#;

pub const BACKEND_INDEX_PROPS: &str = r#"use anyhow::Result;
use serde::{Deserialize, Serialize};

// Example of using enum Props for business logic states
// The frontend can pattern match on the "type" field
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum PageProps {
    // Success state with data
    Ok { message: String },
    // You can add other business logic states:
    // NotFound,
    // Unauthorized { reason: String },
}

// Async function - you can use .await here
// Returns anyhow::Result<PageProps>
// - Ok(PageProps::Ok { ... }) for success
// - Ok(PageProps::NotFound) for business logic "errors"
// - Err(e) for system errors (returns HTTP 500)
pub async fn get_props() -> Result<PageProps> {
    // Example: fetch data from external APIs
    // let response = wstd::http::Client::new()
    //     .get("https://api.example.com")
    //     .send()
    //     .await
    //     .context("Failed to fetch data")?;  // System error -> HTTP 500

    // if !response.status().is_success() {
    //     return Ok(PageProps::NotFound);  // Business logic -> HTTP 200
    // }

    Ok(PageProps::Ok {
        message: "Welcome to Forte WASM!".into(),
    })
}

// POST action types (for form submissions and mutations)
// Add this section if you want to handle POST requests

// Input data from the client
#[derive(Deserialize)]
pub struct ActionInput {
    // Add your form fields here
    // Example:
    // pub message: String,
}

// Success response variants
// Add your own variants as needed
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Response {
    // Example variants:
    // Success { message: String },
    // Redirect { url: String },
}

// Error response variants
// Add your own error types as needed
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum Error {
    // Example variants:
    // ValidationError { field: String, message: String },
    // NotFound,
}

// POST action handler
// Returns anyhow::Result<Result<Response, Error>>
// - Err(e): System error -> HTTP 500
// - Ok(Ok(response)): Success -> HTTP 200 with Response JSON
// - Ok(Err(error)): Business error -> HTTP 200 with Error JSON
pub async fn post_action(_input: ActionInput) -> Result<Result<Response, Error>> {
    // Example implementation:
    // if input.message.is_empty() {
    //     return Ok(Err(Error::ValidationError {
    //         field: "message".into(),
    //         message: "Message cannot be empty".into(),
    //     }));
    // }
    //
    // Ok(Ok(Response::Success {
    //     message: "Action completed successfully".into(),
    // }))

    // For now, return a dummy error since Response and Error have no variants
    anyhow::bail!("post_action not implemented - add variants to Response and Error enums")
}
"#;

// Frontend templates

pub fn frontend_package_json(project_name: &str) -> String {
    format!(
        r#"{{
  "name": "{}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "dev": "tsx ../.generated/frontend/server.ts",
    "build": "vite build && vite build --ssr",
    "preview": "vite preview"
  }},
  "dependencies": {{
    "react": "^18.2.0",
    "react-dom": "^18.2.0",
    "express": "^4.18.0",
    "@forte/runtime": "file:src/forte"
  }},
  "devDependencies": {{
    "@types/express": "^4.17.0",
    "@types/node": "^20.10.0",
    "@types/react": "^18.2.0",
    "@types/react-dom": "^18.2.0",
    "@vitejs/plugin-react": "^4.2.0",
    "typescript": "^5.3.0",
    "vite": "^5.0.0",
    "tsx": "^4.7.0",
    "esbuild": "^0.19.0"
  }}
}}
"#,
        project_name
    )
}

pub const FRONTEND_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "paths": {
      "@/*": ["./src/*"]
    }
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
"#;

pub const FRONTEND_VITE_CONFIG: &str = r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 3000,
  },
})
"#;

pub const FRONTEND_SERVER_WRAPPER: &str = r#"// [Generated] Server entry point - imports the generated SSR server
// This file exists in the frontend directory to ensure proper module resolution
import '../.generated/frontend/server.ts';
"#;

pub const FRONTEND_ROOT_LAYOUT: &str = r#"import * as React from 'react';

interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="UTF-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1.0" />
        <title>Forte App</title>
      </head>
      <body>
        <div id="root">
          <main>{children}</main>
        </div>
      </body>
    </html>
  );
}
"#;

pub const FRONTEND_INDEX_PAGE: &str = r#"import * as React from 'react';
import type { PageProps } from "./props.gen";

// PageProps is an enum - pattern match on the "type" field
export default function IndexPage(props: PageProps) {
  // Handle the Ok variant
  if (props.type === "Ok") {
    return (
      <div>
        <h1>{props.message}</h1>
        <p>Edit backend/src/routes/index/props.rs to change this message.</p>
      </div>
    );
  }

  // You can handle other variants here:
  // if (props.type === "NotFound") {
  //   return <div><h1>Not Found</h1></div>;
  // }
  //
  // if (props.type === "Unauthorized") {
  //   return <div><h1>Unauthorized</h1><p>{props.reason}</p></div>;
  // }

  // Fallback for unknown types
  return <div>Unknown page state</div>;
}
"#;

pub const FRONTEND_ERROR_PAGE: &str = r#"import * as React from 'react';

interface ErrorProps {
  status: number;
  message: string;
  error?: any;
}

export default function ErrorPage({ status, message, error }: ErrorProps) {
  return (
    <div style={{ padding: '2rem', fontFamily: 'system-ui' }}>
      <h1 style={{ color: '#d32f2f' }}>Error {status}</h1>
      <p style={{ fontSize: '1.2rem', color: '#666' }}>{message}</p>
      {error && process.env.NODE_ENV === 'development' && (
        <details style={{ marginTop: '2rem', padding: '1rem', background: '#f5f5f5', borderRadius: '4px' }}>
          <summary style={{ cursor: 'pointer', fontWeight: 'bold' }}>Error Details</summary>
          <pre style={{ marginTop: '1rem', fontSize: '0.9rem', overflow: 'auto' }}>
            {JSON.stringify(error, null, 2)}
          </pre>
        </details>
      )}
    </div>
  );
}
"#;

// Forte Runtime Library Templates

pub const FORTE_RUNTIME_PACKAGE_JSON: &str = r#"{
  "name": "@forte/runtime",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "index.ts",
  "types": "index.ts"
}
"#;

pub const FORTE_RUNTIME_INDEX: &str = r#"// Forte Runtime Library
// Auto-generated client-side utilities

export { RouterProvider, useRouter, ErrorOverlay } from './Router';
export { Form, useAction } from './Form';
export { Link } from './Link';
"#;

pub const FORTE_RUNTIME_FORM: &str = r#"import * as React from 'react';

/**
 * useAction hook for handling form actions with proper state management
 * Uses discriminated unions for type-safe state handling
 */
type ActionState<TResponse, TError> =
  | { status: 'idle' }
  | { status: 'pending' }
  | { status: 'success'; data: TResponse }
  | { status: 'error'; error: TError; validationErrors?: Record<string, string> };

interface UseActionOptions {
  onSuccess?: (data: any) => void;
  onError?: (error: any) => void;
}

export function useAction<TInput = any, TResponse = any, TError = any>(
  action: string,
  options?: UseActionOptions
) {
  const [state, setState] = React.useState<ActionState<TResponse, TError>>({ status: 'idle' });

  const execute = React.useCallback(async (input: TInput) => {
    setState({ status: 'pending' });

    try {
      const response = await fetch(action, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(input),
      });

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({ message: 'Unknown error' }));

        // Extract validation errors if present
        const validationErrors = errorData.validationErrors || errorData.errors;

        setState({
          status: 'error',
          error: errorData as TError,
          validationErrors
        });

        options?.onError?.(errorData);
        return;
      }

      const data = await response.json();
      setState({ status: 'success', data: data as TResponse });
      options?.onSuccess?.(data);
    } catch (error) {
      const errorObj = error instanceof Error ? error : new Error('Unknown error');
      setState({
        status: 'error',
        error: errorObj as TError
      });
      options?.onError?.(errorObj);
    }
  }, [action, options]);

  const reset = React.useCallback(() => {
    setState({ status: 'idle' });
  }, []);

  return {
    ...state,
    execute,
    reset,
    isPending: state.status === 'pending',
    isSuccess: state.status === 'success',
    isError: state.status === 'error',
    isIdle: state.status === 'idle',
  };
}

interface FormProps extends React.FormHTMLAttributes<HTMLFormElement> {
  children: React.ReactNode;
}

/**
 * Forte Form component
 * Handles POST actions with progressive enhancement
 */
export function Form({ children, action, method = 'POST', ...props }: FormProps) {
  const handleSubmit = async (e: React.FormEvent<HTMLFormElement>) => {
    // For now, use default form submission
    // Future: client-side fetch with optimistic updates
  };

  return (
    <form
      action={action}
      method={method}
      onSubmit={handleSubmit}
      {...props}
    >
      {children}
    </form>
  );
}
"#;

pub const FORTE_RUNTIME_ROUTER: &str = r#"import * as React from 'react';

interface RouterContextValue {
  navigate: (href: string) => Promise<void>;
  prefetch: (href: string) => Promise<void>;
  currentPath: string;
}

const RouterContext = React.createContext<RouterContextValue | null>(null);

export function useRouter() {
  const context = React.useContext(RouterContext);
  const isServer = typeof window === 'undefined';

  if (!context) {
    // On server, return no-op router
    if (isServer) {
      return {
        navigate: async () => {},
        prefetch: async () => {},
        currentPath: '/'
      };
    }
    throw new Error('useRouter must be used within a RouterProvider');
  }
  return context;
}

interface RouterProviderProps {
  children: React.ReactNode;
  initialPath?: string;
}

export function RouterProvider({ children, initialPath }: RouterProviderProps) {
  const isServer = typeof window === 'undefined';
  const [currentPath, setCurrentPath] = React.useState(
    initialPath || (!isServer ? window.location.pathname : '/')
  );
  const prefetchCache = React.useRef<Map<string, any>>(new Map());

  const prefetch = React.useCallback(async (href: string) => {
    // Skip on server
    if (isServer) return;

    // Skip if already in cache
    if (prefetchCache.current.has(href)) {
      return;
    }

    try {
      // Use /__wasm prefix to fetch directly from WASM backend
      const response = await fetch(`/__wasm${href}`, {
        headers: { 'Accept': 'application/json' }
      });

      if (response.ok) {
        const data = await response.json();
        prefetchCache.current.set(href, data);
      }
    } catch (err) {
      // Ignore prefetch errors
      console.warn('[Forte Router] Prefetch failed for:', href, err);
    }
  }, [isServer]);

  const navigate = React.useCallback(async (href: string) => {
    // Skip on server
    if (isServer) return;

    try {
      // Check cache first
      let pageProps = prefetchCache.current.get(href);

      if (!pageProps) {
        // Fetch from WASM backend using /__wasm prefix
        const response = await fetch(`/__wasm${href}`, {
          headers: { 'Accept': 'application/json' }
        });

        if (!response.ok) {
          // Fall back to full page navigation on error
          window.location.href = href;
          return;
        }

        pageProps = await response.json();
      }

      // Update URL without reload
      window.history.pushState({ path: href }, '', href);

      // Update current path
      setCurrentPath(href);

      // Trigger custom event for page updates
      window.dispatchEvent(new CustomEvent('forte:navigate', {
        detail: { href, pageProps }
      }));

      // Clear cache entry after use
      prefetchCache.current.delete(href);
    } catch (err) {
      console.error('[Forte Router] Navigation failed:', err);
      // Fall back to full page navigation
      window.location.href = href;
    }
  }, [isServer]);

  // Handle browser back/forward
  React.useEffect(() => {
    // Skip on server
    if (isServer) return;

    const handlePopState = () => {
      setCurrentPath(window.location.pathname);
      window.dispatchEvent(new CustomEvent('forte:navigate', {
        detail: { href: window.location.pathname }
      }));
    };

    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, [isServer]);

  const value = React.useMemo(() => ({
    navigate,
    prefetch,
    currentPath
  }), [navigate, prefetch, currentPath]);

  return (
    <RouterContext.Provider value={value}>
      {children}
      <ErrorOverlay />
    </RouterContext.Provider>
  );
}

/**
 * Development Error Overlay
 * Shows compilation errors in development mode
 */
interface CompileError {
  file: string;
  line?: number;
  column?: number;
  message: string;
  code?: string;
}

export function ErrorOverlay() {
  const [errors, setErrors] = React.useState<CompileError[]>([]);
  const [dismissed, setDismissed] = React.useState(false);

  React.useEffect(() => {
    // Skip in production
    if (typeof window === 'undefined' || process.env.NODE_ENV === 'production') {
      return;
    }

    // Poll for errors every 1 second
    const interval = setInterval(async () => {
      try {
        const res = await fetch('/__forte/errors');
        if (res.ok) {
          const data: CompileError[] = await res.json();
          setErrors(data);
          if (data.length > 0) {
            setDismissed(false);
          }
        }
      } catch (err) {
        // Ignore fetch errors in development
      }
    }, 1000);

    return () => clearInterval(interval);
  }, []);

  if (errors.length === 0 || dismissed) {
    return null;
  }

  return (
    <div style={{
      position: 'fixed',
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      backgroundColor: 'rgba(0, 0, 0, 0.9)',
      color: '#fff',
      zIndex: 999999,
      padding: '20px',
      overflow: 'auto',
      fontFamily: 'monospace',
    }}>
      <div style={{ maxWidth: '800px', margin: '0 auto' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '20px' }}>
          <h1 style={{ margin: 0, fontSize: '24px', color: '#ff6b6b' }}>
            ❌ Compilation Error{errors.length > 1 ? 's' : ''}
          </h1>
          <button
            onClick={() => setDismissed(true)}
            style={{
              background: 'transparent',
              border: '1px solid #666',
              color: '#fff',
              padding: '8px 16px',
              cursor: 'pointer',
              borderRadius: '4px',
            }}
          >
            Dismiss
          </button>
        </div>

        {errors.map((error, index) => (
          <div
            key={index}
            style={{
              backgroundColor: '#1e1e1e',
              padding: '16px',
              marginBottom: '16px',
              borderRadius: '4px',
              border: '1px solid #ff6b6b',
            }}
          >
            <div style={{ marginBottom: '8px', color: '#ffd43b' }}>
              <strong>{error.file}</strong>
              {error.line && `:${error.line}`}
              {error.column && `:${error.column}`}
            </div>
            {error.code && (
              <div style={{ marginBottom: '8px', color: '#ff6b6b' }}>
                error[{error.code}]
              </div>
            )}
            <pre style={{
              margin: 0,
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              color: '#ddd',
            }}>
              {error.message}
            </pre>
          </div>
        ))}

        <div style={{ marginTop: '20px', padding: '16px', backgroundColor: '#1e1e1e', borderRadius: '4px' }}>
          <p style={{ margin: 0, color: '#888' }}>
            💡 Fix the errors above and save the file. This overlay will disappear automatically.
          </p>
        </div>
      </div>
    </div>
  );
}
"#;

pub const FORTE_RUNTIME_LINK: &str = r#"import * as React from 'react';
import { useRouter } from './Router';

type PrefetchStrategy = false | 'intent' | 'viewport' | 'render';

interface LinkProps extends React.AnchorHTMLAttributes<HTMLAnchorElement> {
  href: string;
  children: React.ReactNode;
  prefetch?: PrefetchStrategy;
}

/**
 * Forte Link component
 * Client-side navigation with progressive enhancement
 *
 * Prefetch strategies:
 * - false: No prefetching (default)
 * - 'intent': Prefetch on hover
 * - 'viewport': Prefetch when link enters viewport
 * - 'render': Prefetch immediately on component render
 */
export function Link({ href, children, prefetch = false, onClick, ...props }: LinkProps) {
  const router = useRouter();
  const [isPrefetching, setIsPrefetching] = React.useState(false);
  const linkRef = React.useRef<HTMLAnchorElement>(null);

  const doPrefetch = React.useCallback(() => {
    if (isPrefetching) return;
    setIsPrefetching(true);
    router.prefetch(href).catch(() => {
      // Ignore prefetch errors
    });
  }, [isPrefetching, href, router]);

  // Prefetch on render if strategy is 'render'
  React.useEffect(() => {
    if (prefetch === 'render') {
      doPrefetch();
    }
  }, [prefetch, doPrefetch]);

  // Prefetch on viewport intersection if strategy is 'viewport'
  React.useEffect(() => {
    if (prefetch !== 'viewport' || isPrefetching || typeof IntersectionObserver === 'undefined') {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            doPrefetch();
          }
        });
      },
      {
        // Start prefetching when link is 50px away from viewport
        rootMargin: '50px',
      }
    );

    if (linkRef.current) {
      observer.observe(linkRef.current);
    }

    return () => {
      if (linkRef.current) {
        observer.unobserve(linkRef.current);
      }
    };
  }, [prefetch, isPrefetching, doPrefetch]);

  const handleClick = async (e: React.MouseEvent<HTMLAnchorElement>) => {
    // Allow default behavior if:
    // - Command/Ctrl click (open in new tab)
    // - Middle mouse button click
    // - Target is set (e.g., target="_blank")
    if (
      e.ctrlKey ||
      e.metaKey ||
      e.button !== 0 ||
      props.target
    ) {
      return;
    }

    // Prevent default navigation
    e.preventDefault();

    // Call custom onClick if provided
    if (onClick) {
      onClick(e);
    }

    // Navigate using client-side routing
    await router.navigate(href);
  };

  const handleMouseEnter = () => {
    // Prefetch on hover if strategy is 'intent'
    if (prefetch === 'intent') {
      doPrefetch();
    }
  };

  return (
    <a
      ref={linkRef}
      href={href}
      onClick={handleClick}
      onMouseEnter={handleMouseEnter}
      {...props}
    >
      {children}
    </a>
  );
}
"#;
