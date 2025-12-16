FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && \
    echo "fn main() {println!(\"if you see this, the build broke\");}" > src/main.rs
RUN cargo build --release
COPY . .
RUN touch src/main.rs
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/hq .
EXPOSE 8080
CMD ["./hq"]
