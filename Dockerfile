FROM rust:1.94-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    cmake \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release --bin liberado-anythingllm-mcp

RUN strip target/release/liberado-anythingllm-mcp

FROM scratch AS export
COPY --from=builder /app/target/release/liberado-anythingllm-mcp /liberado-anythingllm-mcp

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /app/target/release/liberado-anythingllm-mcp /usr/local/bin/liberado-anythingllm-mcp

ENV MCP_ANYTHINGLLM_TRANSPORT=http
ENV MCP_ANYTHINGLLM_HTTP_HOST=0.0.0.0
ENV MCP_ANYTHINGLLM_HTTP_PORT=8080

ENTRYPOINT ["/usr/local/bin/liberado-anythingllm-mcp"]
