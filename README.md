# liberado-anythingllm-mcp

Rust MCP server for ingesting content into [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm) workspaces.
Exposes tools that let AI agents (LibreChat, OpenClaw) send text, URLs, and files to AnythingLLM for automatic
vectorization — no manual document uploads required.

## Tools

| Tool | Description |
|---|---|
| `list_workspaces` | List all workspaces with their names and slugs |
| `create_workspace` | Create a new workspace by name |
| `ingest_text` | Ingest raw text into a workspace (with title + optional description) |
| `ingest_url` | Scrape and ingest a URL into a workspace |
| `ingest_file` | Upload a file (base64 data URL) to a workspace |
| `list_workspace_documents` | List documents in a workspace with docpaths, IDs, titles |
| `get_workspace_details` | Get full workspace metadata (document count, threads, timestamps) |
| `search_workspace` | Vector similarity search within a workspace — returns matching chunks with scores |
| `delete_document` | Remove a document from a workspace by docpath |


## Environment

| Variable | Default | Description |
|---|---|---|
| `ANYTHINGLLM_BASE_URL` | `http://anythingllm:3001` | AnythingLLM API base URL |
| `ANYTHINGLLM_API_KEY` | `(empty)` | API key for AnythingLLM auth |
| `MCP_ANYTHINGLLM_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `MCP_ANYTHINGLLM_HTTP_HOST` | `0.0.0.0` | HTTP bind address |
| `MCP_ANYTHINGLLM_HTTP_PORT` | `8080` | HTTP port |

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for details on the code structure, data flow,
and design decisions.

