# liberado-anythingllm-mcp — Architecture

## Overview

A lightweight Rust binary that implements the [Model Context Protocol (MCP)](https://modelcontextprotocol.io)
to bridge AI agents (LibreChat, OpenClaw) with AnythingLLM's document ingestion API. It is a **passthrough
gateway** — all heavy processing (PDF parsing, OCR, audio transcription, chunking, vector embedding) is
delegated to AnythingLLM's collector service.

## Code Layout

```
src/
├── main.rs        # Entry point: tracing init, config load, server dispatch
├── lib.rs         # Re-exports for crate API surface
├── config.rs      # ServerConfig, TransportConfig, env-based config
└── server.rs      # MCP server definition (tools) + input parsing helpers
```

### Key files

**`main.rs`** — Bootstraps the server. Reads `ServerConfig::from_env()`, chooses transport
(stdio or HTTP), and calls `server.builder().serve().await`.

**`config.rs`** — Defines `TransportConfig` (Stdio vs Http) and `ServerConfig` (AnythingLLM
base URL + API key + transport). `from_env()` reads from environment variables with
sensible defaults. All defaults point at the Docker Compose service name `anythingllm:3001`.

**`server.rs`** — The bulk of the crate. Contains:
- `AnythingLlmServer` struct wrapping a `reqwest::Client`
- 9 `#[tool]` annotated methods (the MCP tool surface)
- `parse_file_input` / `parse_data_url` / `percent_decode` helpers for base64 data URL parsing
- `format_search_results` / `format_workspace_document_list` / `format_workspace_details` response formatters

## Data Flow

```
AI Agent (LibreChat / OpenClaw)
    │
    │  MCP (streamable-http / stdio)
    ▼
liberado-anythingllm-mcp
    │
    │  HTTP (JSON / multipart) with Bearer auth
    ▼
AnythingLLM REST API (port 3001)
    │
    ├── POST /api/v1/document/raw-text       (ingest_text)
    ├── POST /api/v1/document/upload-link     (ingest_url)
    ├── POST /api/v1/document/upload          (ingest_file)
    ├── GET  /api/v1/workspaces               (list_workspaces)
    ├── POST /api/v1/workspace/new            (create_workspace)
    ├── GET  /api/v1/workspace/:slug          (list_workspace_documents, get_workspace_details)
    ├── POST /api/v1/workspace/:slug/vector-search      (search_workspace)
    └── POST /api/v1/workspace/:slug/update-embeddings  (delete_document)
    │
    ▼
AnythingLLM Collector → Qdrant (vector DB) → DeepInfra embeddings
```

All document upload endpoints use AnythingLLM's `addToWorkspaces` parameter, which
triggers automatic chunking, embedding, and indexing into the target workspace
on the Qdrant vector database.

## MCP Transport

The server supports two transports selectable via `MCP_ANYTHINGLLM_TRANSPORT`:

| Transport | Use case |
|---|---|
| `stdio` (default) | Local agent use (e.g. Claude Code, OpenClaw binary MCP) |
| `http` | Deployed container for LibreChat / OpenClaw remote MCP |

In Docker, the default is `http` on port 8080. The service is registered in
LibreChat's `librechat.yaml` as `streamable-http` and in OpenClaw's `openclaw.json`
as `streamable-http`.

## Dependencies

- **turbomcp** — MCP SDK (HTTP + stdio transports, macro-based tool definitions)
- **reqwest** — HTTP client for AnythingLLM API calls
- **base64** — Data URL parsing for file ingestion
- **tokio** — Async runtime
- **serde_json** — JSON parsing for AnythingLLM responses
- **tracing** / **tracing-subscriber** — Structured logging

The binary is compiled statically via `rust:1.94-slim-bookworm` builder and deployed
on `gcr.io/distroless/cc-debian12:nonroot` for a minimal ~8MB RSS footprint.

## Testing

```bash
cd services/liberado-anythingllm-mcp
cargo test    # 45 unit tests covering config, URL construction, data URL parsing, response formatting
```

Tests use `serial_test` for env-dependent config tests to avoid cross-contamination
between parallel test cases.

## Deployment

Requires `ANYTHINGLLM_API_KEY` 
