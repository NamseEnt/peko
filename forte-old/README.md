# Forte

Are you torn between the performance of **Rust/Go** and the developer experience of **TypeScript**?
Forte solves this dilemma. It acts as a bridge, analyzing your backend code to automatically generate type-safe clients for your frontend. No manual schema files, no duplicate type definitions.

### Key Features

- 🧬 **Polyglot Backend:** Write your server logic in **Rust**, **Go**, or **TypeScript**.
- 🔗 **End-to-End Type Safety:** Changes in backend structs/types are instantly reflected in your frontend client.
- ⚡ **RPC-over-HTTP:** Call backend functions as if they were local JavaScript functions. No REST API boilerplate.
- 🛠 **Zero Schema Definition:** No `.graphql`, `.proto`, or OpenAPI YAML files required. **Your code is the schema.**

### How it Works (Architecture)

Forte uses a **"Manifest-First"** approach:

1. **Analyze:** The Forte CLI scans your backend code (Rust macros, Go comments) and extracts API signatures.
2. **Manifest:** It generates an intermediate `forte.toml`.
3. **Generate:** The frontend consumes this manifest to build a fully typed TypeScript client SDK.

### Quick Start

#### 1. Define Backend Functions

Use language-specific decorators or macros to expose your functions.

**🦀 Rust (with `serde` & `ts-rs`)**

```rust
// be/rust-service/src/main.rs
use Forte::rpc;

#[derive(Serialize, TS)]
struct User {
    id: u32,
    username: String,
}

#[Forte::rpc] // Expose this function to Frontend
fn get_user(user_id: u32) -> User {
    User { id: user_id, username: "rustacean".to_string() }
}
```

**🐹 Go (with Comment Directives)**

```go
// be/go-service/main.go

type Product struct {
    ID    int    `json:"id"`
    Title string `json:"title"`
}

// @rpc:export
func GetProduct(id int) Product {
    return Product{ID: id, Title: "Gopher Doll"}
}
```

#### 2. Generate Client

Run the CLI to sync your backend changes to the frontend.

```bash
npx Forte codegen
# Output:
# ✅ [Rust] Parsed 1 endpoint
# ✅ [Go]   Parsed 1 endpoint
# ✨ Generated: fe/src/generated/client.ts
```

#### 3. Use in Frontend (React Example)

Enjoy full autocomplete and type checking.

```tsx
import { api } from "@/generated/client";

const Dashboard = async () => {
  // 1. Call Rust Backend
  // TypeScript knows 'user' has { id: number, username: string }
  const user = await api.rust.getUser({ user_id: 1 });

  // 2. Call Go Backend
  const product = await api.go.getProduct({ id: 99 });

  return (
    <div>
      <h1>Hello, {user.username}</h1>
      <p>Recommended: {product.Title}</p>
    </div>
  );
};
```

### Roadmap

- [ ] **Core:** Protocol Definition & Manifest Schema
- [ ] **Adapter:** Rust (Axum/Actix) Support
- [ ] **Adapter:** Go (Gin/Fiber) Support
- [ ] **Feature:** Server-Side Streaming (SSE)
- [ ] **Feature:** Form Actions & Progressive Enhancement
