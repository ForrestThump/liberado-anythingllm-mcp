use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use tracing::warn;
use turbomcp::prelude::*;

use crate::config::ServerConfig;

#[derive(Clone)]
pub struct AnythingLlmServer {
    pub config: ServerConfig,
    client: reqwest::Client,
}

impl AnythingLlmServer {
    pub fn new(config: ServerConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");
        Self { config, client }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api{}", self.config.anythingllm_base_url, path)
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.config.anythingllm_api_key)
    }

    async fn send_get(&self, tool: &str, path: &str) -> McpResult<String> {
        let url = self.api_url(path);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| {
                McpError::tool_execution_failed(
                    tool.to_string(),
                    format!("HTTP request failed: {}", e),
                )
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.check_api_response(tool, &url, status, body));
        }
        Ok(body)
    }

    async fn send_post_json(
        &self,
        tool: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> McpResult<String> {
        let url = self.api_url(path);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                McpError::tool_execution_failed(
                    tool.to_string(),
                    format!("HTTP request failed: {}", e),
                )
            })?;
        let status = resp.status();
        let response_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.check_api_response(tool, &url, status, response_text));
        }
        Ok(response_text)
    }

    fn check_api_response(
        &self,
        tool: &str,
        url: &str,
        status: reqwest::StatusCode,
        body: String,
    ) -> McpError {
        let is_auth_error = status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN;
        let hint = if is_auth_error {
            format!(
                "\n\n  REASON: The API key sent by this MCP was rejected by AnythingLLM.\n  \
                 URL: {url}\n  \
                 TROUBLESHOOTING:\n  \
                 1. Verify the key in envs/_shared/auth.env (ANYTHINGLLM_API_KEY) matches a key\n  \
                    in the AnythingLLM api_keys table:\n  \
                    sqlite3 volumes/anythingllm/storage/anythingllm.db\n  \
                      \"SELECT secret, name FROM api_keys;\"\n  \
                 2. If missing, add the key from auth.env to the table:\n  \
                    sqlite3 volumes/anythingllm/storage/anythingllm.db\n  \
                      \"INSERT INTO api_keys (secret, createdAt, lastUpdatedAt, name)\n  \
                       VALUES ('<key>', strftime('%%s','now')||'000',\n  \
                               strftime('%%s','now')||'000', 'liberado-anythingllm-mcp');\"\n  \
                 3. Restart the anythingllm container if the key still doesn't work."
            )
        } else {
            format!("\n\n  URL: {url}", url = url)
        };

        warn!(
            tool = %tool,
            url = %url,
            status = %status.as_u16(),
            "AnythingLLM API returned error"
        );

        McpError::tool_execution_failed(
            tool.to_string(),
            format!("AnythingLLM API error ({}): {}{}", status, body, hint),
        )
    }
}

#[server(name = "liberado-anythingllm-mcp", version = "0.1.0")]
impl AnythingLlmServer {
    /// Ingest raw text into an AnythingLLM workspace.
    /// The text will be vectorized and added to the specified workspace's knowledge base.
    #[tool]
    async fn ingest_text(
        &self,
        #[description("Text content to ingest into the workspace")]
        text: String,
        #[description("Title for the document (required)")]
        title: String,
        #[description("Slug or name of the target workspace. If slug, must match the workspace slug exactly. If name, a workspace with this name must exist.")]
        workspace: String,
        #[description("Optional description for the document")]
        description: Option<String>,
    ) -> McpResult<String> {
        let mut metadata = serde_json::json!({ "title": title });
        if let Some(desc) = description {
            metadata["description"] = serde_json::Value::String(desc);
        }

        let body = serde_json::json!({
            "textContent": text,
            "metadata": metadata,
            "addToWorkspaces": workspace,
        });

        let response_text = self.send_post_json("ingest_text", "/v1/document/raw-text", &body).await?;

        Ok(format!(
            "Successfully ingested text into workspace '{}'.\n\nResponse: {}",
            workspace, response_text
        ))
    }

    /// Ingest a URL into an AnythingLLM workspace.
    /// The URL will be scraped, processed, and vectorized into the specified workspace.
    #[tool]
    async fn ingest_url(
        &self,
        #[description("URL to scrape and ingest")]
        url: String,
        #[description("Slug or name of the target workspace")]
        workspace: String,
        #[description("Optional title for the document")]
        title: Option<String>,
    ) -> McpResult<String> {
        let mut body = serde_json::json!({
            "link": url,
            "addToWorkspaces": workspace,
        });
        if let Some(t) = title {
            body["metadata"] = serde_json::json!({ "title": t });
        }

        let response_text = self.send_post_json("ingest_url", "/v1/document/upload-link", &body).await?;

        Ok(format!(
            "Successfully ingested URL '{}' into workspace '{}'.\n\nResponse: {}",
            url, workspace, response_text
        ))
    }

    /// List all workspaces in AnythingLLM.
    /// Returns workspace names and slugs so you can target content to the correct workspace.
    #[tool]
    async fn list_workspaces(&self) -> McpResult<String> {
        let response_text = self.send_get("list_workspaces", "/v1/workspaces").await?;
        format_list_workspaces(&response_text)
    }

    /// Create a new workspace in AnythingLLM.
    /// Returns the workspace slug which can be used with ingest_* tools.
    #[tool]
    async fn create_workspace(
        &self,
        #[description("Name for the new workspace")]
        name: String,
    ) -> McpResult<String> {
        let body = serde_json::json!({ "name": name });
        let response_text = self.send_post_json("create_workspace", "/v1/workspace/new", &body).await?;
        format_create_workspace(&response_text, &name)
    }

    /// Search for relevant content within a workspace using vector similarity search.
    /// Returns matching document chunks with their text content, relevance score, and metadata.
    #[tool]
    async fn search_workspace(
        &self,
        #[description("Slug of the workspace to search within")]
        slug: String,
        #[description("Search query to find relevant content")]
        query: String,
        #[description("Number of results to return (default: 5)")]
        top_n: Option<u32>,
        #[description("Minimum similarity score threshold (0.0 to 1.0, default: 0.0)")]
        score_threshold: Option<f64>,
    ) -> McpResult<String> {
        let mut body = serde_json::json!({ "query": query });
        if let Some(n) = top_n {
            body["topN"] = serde_json::json!(n);
        }
        if let Some(t) = score_threshold {
            body["scoreThreshold"] = serde_json::json!(t);
        }

        let path = format!("/v1/workspace/{}/vector-search", slug);
        let response_text = self.send_post_json("search_workspace", &path, &body).await?;
        format_search_results(&response_text, &slug)
    }

    /// List documents in a workspace.
    /// Returns document ID, docpath, title, and creation date for each document.
    /// Use the docpath with delete_document to remove documents.
    #[tool]
    async fn list_workspace_documents(
        &self,
        #[description("Slug of the workspace to list documents from")]
        slug: String,
    ) -> McpResult<String> {
        let path = format!("/v1/workspace/{}", slug);
        let response_text = self.send_get("list_workspace_documents", &path).await?;
        format_workspace_document_list(&response_text, &slug)
    }

    /// Get full details about a workspace including its metadata, document count, and settings.
    #[tool]
    async fn get_workspace_details(
        &self,
        #[description("Slug of the workspace to get details for")]
        slug: String,
    ) -> McpResult<String> {
        let path = format!("/v1/workspace/{}", slug);
        let response_text = self.send_get("get_workspace_details", &path).await?;
        format_workspace_details(&response_text, &slug)
    }

    /// Delete a document from a workspace.
    /// The docpath is obtained from list_workspace_documents.
    /// This removes the document from the workspace and un-embeds it.
    #[tool]
    async fn delete_document(
        &self,
        #[description("Slug of the workspace containing the document")]
        slug: String,
        #[description("Document docpath to delete (from list_workspace_documents). Format: custom-documents/<filename>-<hash>.json")]
        docpath: String,
    ) -> McpResult<String> {
        let path = format!("/v1/workspace/{}/update-embeddings", slug);
        let body = serde_json::json!({
            "adds": [],
            "deletes": [docpath],
        });

        let response_text = self.send_post_json("delete_document", &path, &body).await?;

        Ok(format!(
            "Successfully deleted document from workspace '{}'.\n\nResponse: {}",
            slug, response_text
        ))
    }

    /// Ingest a file (base64-encoded) into an AnythingLLM workspace.
    /// Supports: PDF, DOCX, PPTX, XLSX, images (PNG, JPG, WEBP), audio (MP3, WAV, M4A),
    /// video (MP4), markdown, plain text, and more.
    /// Pass the file as a data URL (data:<mime>;base64,<data>) or raw base64.
    #[tool]
    async fn ingest_file(
        &self,
        #[description("File as a data URL or raw base64 string. Format: data:<mime>;base64,<data> or just the base64 data.")]
        file_data: String,
        #[description("Filename with extension (e.g., 'report.pdf', 'image.png', 'notes.md'). Required.")]
        filename: String,
        #[description("Slug or name of the target workspace")]
        workspace: String,
        #[description("Optional title for the document (defaults to filename)")]
        title: Option<String>,
    ) -> McpResult<String> {
        let (raw_bytes, mime_type) = parse_file_input(&file_data)?;
        let doc_title = title.unwrap_or_else(|| filename.clone());

        let url = self.api_url("/v1/document/upload");

        let file_part = reqwest::multipart::Part::bytes(raw_bytes)
            .file_name(filename.clone())
            .mime_str(&mime_type)
            .map_err(|e| {
                McpError::invalid_params(format!("Invalid MIME type '{}': {}", mime_type, e))
            })?;

        let metadata = serde_json::json!({ "title": doc_title });

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("addToWorkspaces", workspace.clone())
            .text("metadata", metadata.to_string());

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                McpError::tool_execution_failed(
                    "ingest_file",
                    format!("HTTP request failed: {}", e),
                )
            })?;

        let status = resp.status();
        let response_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(self.check_api_response("ingest_file", &url, status, response_text));
        }

        Ok(format!(
            "Successfully ingested file '{}' into workspace '{}'.\n\nResponse: {}",
            filename, workspace, response_text
        ))
    }
}

pub(crate) fn parse_file_input(s: &str) -> Result<(Vec<u8>, String), McpError> {
    if s.starts_with("data:") {
        parse_data_url(s)
    } else {
        let decoded = BASE64.decode(s).map_err(|e| {
            McpError::invalid_params(format!(
                "Invalid base64 input: {}. Provide a data URL (data:<mime>;base64,<data>) or raw base64.",
                e
            ))
        })?;
        Ok((decoded, "application/octet-stream".to_string()))
    }
}

pub(crate) fn parse_data_url(data_url: &str) -> Result<(Vec<u8>, String), McpError> {
    let after_prefix = data_url.strip_prefix("data:").ok_or_else(|| {
        McpError::invalid_params("Data URL must start with 'data:'".to_string())
    })?;

    let (mime_part, encoded) = after_prefix.split_once(',').ok_or_else(|| {
        McpError::invalid_params("Invalid data URL: missing comma separator".to_string())
    })?;

    let is_base64 = mime_part.ends_with(";base64");
    let mime_type = if is_base64 {
        mime_part.trim_end_matches(";base64")
    } else {
        mime_part
    };

    let mime_type = if mime_type.is_empty() {
        "application/octet-stream"
    } else {
        mime_type
    };

    let decoded = if is_base64 {
        BASE64.decode(encoded).map_err(|e| {
            McpError::invalid_params(format!("Base64 decode error: {}", e))
        })?
    } else {
        percent_decode(encoded)?
    };

    Ok((decoded, mime_type.to_string()))
}

pub(crate) fn percent_decode(s: &str) -> Result<Vec<u8>, McpError> {
    let mut result = Vec::with_capacity(s.len());
    let mut chars = s.as_bytes().iter().copied();
    while let Some(c) = chars.next() {
        if c == b'%' {
            let hi = chars.next().ok_or_else(|| {
                McpError::invalid_params("Truncated percent encoding".to_string())
            })?;
            let lo = chars.next().ok_or_else(|| {
                McpError::invalid_params("Truncated percent encoding".to_string())
            })?;
            let byte = hex_to_byte(hi, lo)?;
            result.push(byte);
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

pub(crate) fn hex_to_byte(hi: u8, lo: u8) -> Result<u8, McpError> {
    let hi = hex_digit(hi)?;
    let lo = hex_digit(lo)?;
    Ok(hi * 16 + lo)
}

pub(crate) fn hex_digit(b: u8) -> Result<u8, McpError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(McpError::invalid_params(format!(
            "Invalid hex digit: {}",
            b as char
        ))),
    }
}

fn format_search_results(response_text: &str, slug: &str) -> McpResult<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(response_text) {
        if let Some(results) = parsed.get("results").and_then(|r| r.as_array()) {
            if results.is_empty() {
                return Ok(format!(
                    "No results found in workspace '{}'.",
                    slug
                ));
            }

            let mut output = format!(
                "Found {} result(s) in workspace '{}':\n\n",
                results.len(),
                slug
            );
            for (i, result) in results.iter().enumerate() {
                let text = result
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let score = result
                    .get("score")
                    .and_then(|s| s.as_f64())
                    .unwrap_or(0.0);
                let distance = result
                    .get("distance")
                    .and_then(|d| d.as_f64())
                    .unwrap_or(0.0);
                let title = result
                    .get("metadata")
                    .and_then(|m| m.get("title"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");

                output.push_str(&format!(
                    "{}. [Score: {:.3}, Distance: {:.3}] {}\n   {}\n\n",
                    i + 1,
                    score,
                    distance,
                    title,
                    text
                ));
            }
            return Ok(output);
        }
    }

    Ok(format!("Search results for workspace '{}':\n{}", slug, response_text))
}

fn format_list_workspaces(response_text: &str) -> McpResult<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(response_text) {
        let workspaces = parsed.get("workspaces").and_then(|w| w.as_array()).map(|arr| {
            let mut result = String::new();
            if arr.is_empty() {
                result.push_str("No workspaces found.");
            } else {
                result.push_str(&format!("Found {} workspace(s):\n\n", arr.len()));
                for (i, ws) in arr.iter().enumerate() {
                    let name = ws.get("name").and_then(|n| n.as_str()).unwrap_or("unnamed");
                    let slug = ws.get("slug").and_then(|s| s.as_str()).unwrap_or("n/a");
                    result.push_str(&format!("{}. {} (slug: {})\n", i + 1, name, slug));
                }
            }
            result
        });
        if let Some(output) = workspaces {
            return Ok(output);
        }
    }

    Ok(format!("Workspaces:\n{}", response_text))
}

fn format_create_workspace(response_text: &str, name: &str) -> McpResult<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(response_text) {
        if let Some(slug) = parsed
            .get("workspace")
            .and_then(|w| w.get("slug"))
            .and_then(|s| s.as_str())
        {
            return Ok(format!(
                "Successfully created workspace '{}' with slug '{}'.",
                name, slug
            ));
        }
    }

    Ok(format!(
        "Successfully created workspace '{}'.\n\nResponse: {}",
        name, response_text
    ))
}

fn format_workspace_document_list(response_text: &str, slug: &str) -> McpResult<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(response_text) {
        let ws = parsed.get("workspace").or_else(|| {
            parsed.get("workspaces").and_then(|a| a.as_array()).and_then(|a| a.first())
        });

        if let Some(workspace) = ws {
            let name = workspace
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(slug);

            if let Some(docs) = workspace.get("documents").and_then(|d| d.as_array()) {
                if docs.is_empty() {
                    return Ok(format!(
                        "Workspace '{}' has no documents.",
                        name
                    ));
                }

                let mut output =
                    format!("Workspace '{}' has {} document(s):\n\n", name, docs.len());
                for (i, doc) in docs.iter().enumerate() {
                    let id = doc.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                    let docpath = doc
                        .get("docpath")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let title = doc
                        .get("metadata")
                        .and_then(|m| m.as_str())
                        .and_then(|m| {
                            serde_json::from_str::<serde_json::Value>(m)
                                .ok()
                                .and_then(|v| {
                                    v.get("title")
                                        .and_then(|t| t.as_str().map(String::from))
                                })
                        })
                        .unwrap_or_default();
                    let created = doc
                        .get("createdAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    output.push_str(&format!(
                        "{}. [{}] {}\n   Docpath: {}\n   Created: {}\n\n",
                        i + 1,
                        id,
                        title,
                        docpath,
                        created,
                    ));
                }
                return Ok(output);
            }
        }
    }

    Ok(format!("Workspace '{}':\n{}", slug, response_text))
}

fn format_workspace_details(response_text: &str, slug: &str) -> McpResult<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(response_text) {
        let ws = parsed.get("workspace").or_else(|| {
            parsed.get("workspaces").and_then(|a| a.as_array()).and_then(|a| a.first())
        });

        if let Some(workspace) = ws {
            let name = workspace
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(slug);
            let ws_slug = workspace
                .get("slug")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let created = workspace
                .get("createdAt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let updated = workspace
                .get("lastUpdatedAt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let doc_count = workspace
                .get("documents")
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let thread_count = workspace
                .get("threads")
                .and_then(|t| t.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            return Ok(format!(
                "Workspace: {}\nSlug: {}\nDocuments: {}\nThreads: {}\nCreated: {}\nLast Updated: {}",
                name, ws_slug, doc_count, thread_count, created, updated,
            ));
        }
    }

    Ok(format!("Workspace '{}':\n{}", slug, response_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ServerConfig, TransportConfig};

    // ── api_url ──

    #[test]
    fn test_api_url_constructs_correctly() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        assert_eq!(
            server.api_url("/v1/document/raw-text"),
            "http://anythingllm:3001/api/v1/document/raw-text"
        );
        assert_eq!(
            server.api_url("/v1/workspace/my-space/update-embeddings"),
            "http://anythingllm:3001/api/v1/workspace/my-space/update-embeddings"
        );
    }

    #[test]
    fn test_api_url_workspace_paths() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        assert_eq!(
            server.api_url("/v1/workspace/my-slug"),
            "http://anythingllm:3001/api/v1/workspace/my-slug"
        );
        assert_eq!(
            server.api_url("/v1/workspace/my-slug/update-embeddings"),
            "http://anythingllm:3001/api/v1/workspace/my-slug/update-embeddings"
        );
    }

    #[test]
    fn test_api_url_custom_base() {
        let config = ServerConfig {
            anythingllm_base_url: "http://myhost:4071".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        assert_eq!(
            server.api_url("/v1/document/upload"),
            "http://myhost:4071/api/v1/document/upload"
        );
    }

    // ── auth_header ──

    #[test]
    fn test_auth_header_format() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "sk-test-token".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        assert_eq!(server.auth_header(), "Bearer sk-test-token");
    }

    #[test]
    fn test_auth_header_empty_key() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: String::new(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        assert_eq!(server.auth_header(), "Bearer ");
    }

    // ── check_api_response ──

    #[test]
    fn test_check_api_response_auth_error_401() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        let err = server.check_api_response(
            "test_tool",
            "http://localhost/api/test",
            reqwest::StatusCode::UNAUTHORIZED,
            "Unauthorized".to_string(),
        );
        let msg = format!("{}", err);
        assert!(msg.contains("API key"));
        assert!(msg.contains("TROUBLESHOOTING"));
        assert!(msg.contains("401"));
    }

    #[test]
    fn test_check_api_response_auth_error_403() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        let err = server.check_api_response(
            "test_tool",
            "http://localhost/api/test",
            reqwest::StatusCode::FORBIDDEN,
            "Forbidden".to_string(),
        );
        let msg = format!("{}", err);
        assert!(msg.contains("API key"));
        assert!(msg.contains("TROUBLESHOOTING"));
        assert!(msg.contains("403"));
    }

    #[test]
    fn test_check_api_response_non_auth_error() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        let err = server.check_api_response(
            "my_tool",
            "http://localhost/api/resource",
            reqwest::StatusCode::NOT_FOUND,
            "Not Found".to_string(),
        );
        let msg = format!("{}", err);
        assert!(msg.contains("my_tool"));
        assert!(msg.contains("404"));
        assert!(msg.contains("Not Found"));
        assert!(!msg.contains("TROUBLESHOOTING"));
        assert!(msg.contains("URL: http://localhost/api/resource"));
    }

    #[test]
    fn test_check_api_response_server_error() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        let server = AnythingLlmServer::new(config);
        let err = server.check_api_response(
            "server_tool",
            "http://localhost/api/error",
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error".to_string(),
        );
        let msg = format!("{}", err);
        assert!(msg.contains("500"));
        assert!(msg.contains("Internal error"));
        assert!(!msg.contains("TROUBLESHOOTING"));
    }

    // ── hex_digit ──

    #[test]
    fn test_hex_digit_lowercase() {
        assert_eq!(hex_digit(b'0').unwrap(), 0);
        assert_eq!(hex_digit(b'9').unwrap(), 9);
        assert_eq!(hex_digit(b'a').unwrap(), 10);
        assert_eq!(hex_digit(b'f').unwrap(), 15);
    }

    #[test]
    fn test_hex_digit_uppercase() {
        assert_eq!(hex_digit(b'A').unwrap(), 10);
        assert_eq!(hex_digit(b'F').unwrap(), 15);
    }

    #[test]
    fn test_hex_digit_invalid() {
        assert!(hex_digit(b'g').is_err());
        assert!(hex_digit(b'z').is_err());
        assert!(hex_digit(b'@').is_err());
        assert!(hex_digit(b' ').is_err());
    }

    // ── hex_to_byte ──

    #[test]
    fn test_hex_to_byte_valid() {
        assert_eq!(hex_to_byte(b'0', b'0').unwrap(), 0x00);
        assert_eq!(hex_to_byte(b'F', b'F').unwrap(), 0xFF);
        assert_eq!(hex_to_byte(b'A', b'B').unwrap(), 0xAB);
        assert_eq!(hex_to_byte(b'1', b'a').unwrap(), 0x1A);
    }

    #[test]
    fn test_hex_to_byte_invalid() {
        assert!(hex_to_byte(b'G', b'0').is_err());
        assert!(hex_to_byte(b'0', b' ').is_err());
    }

    // ── percent_decode ──

    #[test]
    fn test_percent_decode_no_encoding() {
        let result = percent_decode("Hello World").unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_percent_decode_simple() {
        let result = percent_decode("Hello%20World").unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_percent_decode_hex() {
        let result = percent_decode("%48%65%6C%6C%6F").unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_percent_decode_mixed() {
        let result = percent_decode("a%20b%20c").unwrap();
        assert_eq!(result, b"a b c");
    }

    #[test]
    fn test_percent_decode_truncated() {
        assert!(percent_decode("%4").is_err());
        assert!(percent_decode("%").is_err());
    }

    #[test]
    fn test_percent_decode_empty() {
        let result = percent_decode("").unwrap();
        assert!(result.is_empty());
    }

    // ── parse_data_url ──

    #[test]
    fn test_parse_data_url_base64_pdf() {
        let (data, mime) = parse_data_url("data:application/pdf;base64,SGVsbG8=").unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(mime, "application/pdf");
    }

    #[test]
    fn test_parse_data_url_base64_png() {
        let (data, mime) = parse_data_url("data:image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(data, b"\x89PNG\r\n\x1a\n");
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn test_parse_data_url_percent_encoded() {
        let (data, mime) = parse_data_url("data:text/plain,Hello%20World").unwrap();
        assert_eq!(data, b"Hello World");
        assert_eq!(mime, "text/plain");
    }

    #[test]
    fn test_parse_data_url_no_mime_defaults_to_octet_stream() {
        let (data, mime) = parse_data_url("data:;base64,SGVsbG8=").unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(mime, "application/octet-stream");
    }

    #[test]
    fn test_parse_data_url_missing_data_prefix() {
        assert!(parse_data_url("not-a-data-url").is_err());
    }

    #[test]
    fn test_parse_data_url_missing_comma() {
        assert!(parse_data_url("data:text/plain").is_err());
    }

    #[test]
    fn test_parse_data_url_invalid_base64() {
        assert!(parse_data_url("data:text/plain;base64,!!!invalid!!!").is_err());
    }

    // ── parse_file_input ──

    #[test]
    fn test_parse_file_input_data_url() {
        let (data, mime) =
            parse_file_input("data:application/pdf;base64,SGVsbG8=").unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(mime, "application/pdf");
    }

    #[test]
    fn test_parse_file_input_raw_base64() {
        let (data, mime) = parse_file_input("SGVsbG8=").unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(mime, "application/octet-stream");
    }

    #[test]
    fn test_parse_file_input_invalid_raw_base64() {
        assert!(parse_file_input("!!!invalid!!!").is_err());
    }

    // ── format_search_results ──

    #[test]
    fn test_format_search_results_empty() {
        let json = r#"{"results": []}"#;
        let result = format_search_results(json, "test-ws").unwrap();
        assert!(result.contains("No results found"));
        assert!(result.contains("test-ws"));
    }

    #[test]
    fn test_format_search_results_with_results() {
        let json = r#"{
            "results": [
                {
                    "id": "chunk-1",
                    "text": "This is the first matching chunk of text from a document.",
                    "metadata": {"title": "Annual Report"},
                    "distance": 0.15,
                    "score": 0.92
                },
                {
                    "id": "chunk-2",
                    "text": "This is another chunk with different content.",
                    "metadata": {"title": "Meeting Notes"},
                    "distance": 0.35,
                    "score": 0.78
                }
            ]
        }"#;
        let result = format_search_results(json, "research").unwrap();
        assert!(result.contains("Found 2 result(s)"));
        assert!(result.contains("research"));
        assert!(result.contains("Annual Report"));
        assert!(result.contains("Meeting Notes"));
        assert!(result.contains("Score: 0.920"));
        assert!(result.contains("Distance: 0.150"));
        assert!(result.contains("first matching chunk"));
        assert!(result.contains("another chunk"));
    }

    #[test]
    fn test_format_search_results_long_text_not_truncated() {
        let long_text = "A".repeat(1000);
        let json = format!(
            r#"{{"results": [{{"id": "c1", "text": "{}", "metadata": {{"title": "Long Doc"}}, "distance": 0.1, "score": 0.99}}]}}"#,
            long_text
        );
        let result = format_search_results(&json, "ws").unwrap();
        assert!(result.len() > 1050); // full text preserved
        assert!(result.contains(&"A".repeat(1000))); // all 1000 chars present
        assert!(!result.contains("...")); // no ellipsis
    }

    #[test]
    fn test_format_search_results_fallback() {
        let json = r#"{"error": "something broke"}"#;
        let result = format_search_results(json, "broken-ws").unwrap();
        assert!(result.contains("broken-ws"));
    }

    #[test]
    fn test_format_search_results_single_result() {
        let json = r#"{"results": [{"id": "c1", "text": "chunk text", "metadata": {"title": "Doc"}, "distance": 0.2, "score": 0.85}]}"#;
        let result = format_search_results(json, "ws").unwrap();
        assert!(result.contains("Found 1 result(s)"));
        assert!(result.contains("Score: 0.850"));
        assert!(result.contains("Distance: 0.200"));
        assert!(result.contains("chunk text"));
    }

    #[test]
    fn test_format_search_results_missing_fields() {
        let json = r#"{"results": [{"id": "c1", "text": "only text"}]}"#;
        let result = format_search_results(json, "ws").unwrap();
        assert!(result.contains("Score: 0.000"));
        assert!(result.contains("Distance: 0.000"));
        assert!(result.contains("only text"));
    }

    #[test]
    fn test_format_search_results_missing_text_field() {
        let json = r#"{"results": [{"id": "c1", "score": 0.9}]}"#;
        let result = format_search_results(json, "ws").unwrap();
        // text defaults to empty, title defaults to empty
        assert!(result.contains("Score: 0.900"));
    }

    #[test]
    fn test_format_search_results_null_score() {
        let json = r#"{"results": [{"id": "c1", "text": "hello", "score": null, "distance": null}]}"#;
        let result = format_search_results(json, "ws").unwrap();
        assert!(result.contains("Score: 0.000"));
        assert!(result.contains("Distance: 0.000"));
    }

    // ── format_workspace_document_list ──

    #[test]
    fn test_format_workspace_document_list_empty() {
        let json = r#"{"workspace":{"name":"Test WS","slug":"test-ws","documents":[]}}"#;
        let result = format_workspace_document_list(json, "test-ws").unwrap();
        assert!(result.contains("has no documents"));
        assert!(result.contains("Test WS"));
    }

    #[test]
    fn test_format_workspace_document_list_with_docs() {
        let json = r#"{
            "workspace": {
                "name": "Research",
                "slug": "research",
                "documents": [
                    {
                        "id": 1,
                        "docpath": "custom-documents/report.pdf-uuid123.json",
                        "metadata": "{\"title\": \"Annual Report\"}",
                        "pinned": false,
                        "watched": false,
                        "createdAt": "2025-01-15 10:00:00",
                        "lastUpdatedAt": "2025-01-15 10:00:00"
                    },
                    {
                        "id": 2,
                        "docpath": "custom-documents/notes.txt-uuid456.json",
                        "metadata": "{\"title\": \"Meeting Notes\"}",
                        "pinned": false,
                        "watched": false,
                        "createdAt": "2025-01-16 14:30:00",
                        "lastUpdatedAt": "2025-01-16 14:30:00"
                    }
                ]
            }
        }"#;
        let result = format_workspace_document_list(json, "research").unwrap();
        assert!(result.contains("Research"));
        assert!(result.contains("2 document(s)"));
        assert!(result.contains("Annual Report"));
        assert!(result.contains("Meeting Notes"));
        assert!(result.contains("custom-documents/report.pdf-uuid123.json"));
        assert!(result.contains("custom-documents/notes.txt-uuid456.json"));
        assert!(result.contains("[1]"));
        assert!(result.contains("[2]"));
    }

    #[test]
    fn test_format_workspace_document_list_fallback() {
        let json = r#"{"workspace": {"name": "Test"}}"#;
        let result = format_workspace_document_list(json, "test-ws").unwrap();
        assert!(result.contains("test-ws")); // fallback to raw JSON
    }

    #[test]
    fn test_format_workspace_document_list_no_documents_key() {
        let json = r#"{"workspace": {"name": "Empty WS", "slug": "empty"}}"#;
        let result = format_workspace_document_list(json, "empty").unwrap();
        assert!(result.contains("empty"));
    }

    #[test]
    fn test_format_workspace_document_list_non_json_metadata() {
        let json = r#"{
            "workspace": {
                "name": "Test",
                "slug": "test",
                "documents": [
                    {"id": 1, "docpath": "path.json", "metadata": "not-json-{{", "createdAt": "2025-01-01"}
                ]
            }
        }"#;
        let result = format_workspace_document_list(json, "test").unwrap();
        assert!(result.contains("1 document(s)"));
        assert!(result.contains("path.json"));
    }

    #[test]
    fn test_format_workspace_document_list_missing_fields() {
        let json = r#"{
            "workspace": {
                "name": "Minimal",
                "slug": "min",
                "documents": [
                    {"id": null},
                    {"docpath": null}
                ]
            }
        }"#;
        let result = format_workspace_document_list(json, "min").unwrap();
        assert!(result.contains("2 document(s)"));
        assert!(result.contains("[0]"));
        assert!(result.contains("unknown"));
    }

    #[test]
    fn test_format_workspace_document_list_workspaces_array_fallback() {
        let json = r#"{"workspaces": [{"name": "From Array", "slug": "arr", "documents": [{"id": 1, "docpath": "doc.json", "metadata": "{}", "createdAt": "2025-01-01"}]}]}"#;
        let result = format_workspace_document_list(json, "arr").unwrap();
        assert!(result.contains("From Array"));
        assert!(result.contains("1 document(s)"));
    }

    // ── format_workspace_details ──

    #[test]
    fn test_format_workspace_details_full() {
        let json = r#"{
            "workspace": {
                "id": 79,
                "name": "My workspace",
                "slug": "my-workspace-123",
                "createdAt": "2023-08-17 00:45:03",
                "openAiTemp": null,
                "lastUpdatedAt": "2023-08-17 01:00:00",
                "openAiHistory": 20,
                "openAiPrompt": null,
                "documents": [{"id": 1, "docpath": "doc.json"}],
                "threads": [{"id": 1}]
            }
        }"#;
        let result = format_workspace_details(json, "my-workspace-123").unwrap();
        assert!(result.contains("My workspace"));
        assert!(result.contains("my-workspace-123"));
        assert!(result.contains("Documents: 1"));
        assert!(result.contains("Threads: 1"));
        assert!(result.contains("2023-08-17 00:45:03"));
        assert!(result.contains("2023-08-17 01:00:00"));
    }

    #[test]
    fn test_format_workspace_details_no_docs_no_threads() {
        let json = r#"{
            "workspace": {
                "name": "Empty",
                "slug": "empty",
                "createdAt": "2024-01-01",
                "lastUpdatedAt": "2024-01-01",
                "documents": [],
                "threads": []
            }
        }"#;
        let result = format_workspace_details(json, "empty").unwrap();
        assert!(result.contains("Documents: 0"));
        assert!(result.contains("Threads: 0"));
    }

    #[test]
    fn test_format_workspace_details_fallback() {
        let json = r#"{"error": "not found"}"#;
        let result = format_workspace_details(json, "missing-ws").unwrap();
        assert!(result.contains("missing-ws"));
    }

    #[test]
    fn test_format_workspace_details_null_fields() {
        let json = r#"{
            "workspace": {
                "name": null,
                "slug": null,
                "createdAt": null,
                "lastUpdatedAt": null,
                "documents": null,
                "threads": null
            }
        }"#;
        let result = format_workspace_details(json, "slug-only").unwrap();
        assert!(result.contains("slug-only"));
        assert!(result.contains("Documents: 0"));
        assert!(result.contains("Threads: 0"));
    }

    #[test]
    fn test_format_workspace_details_workspaces_array_fallback() {
        let json = r#"{"workspaces": [{"name": "Arr WS", "slug": "arr-ws", "documents": [], "threads": []}]}"#;
        let result = format_workspace_details(json, "arr-ws").unwrap();
        assert!(result.contains("Arr WS"));
        assert!(result.contains("arr-ws"));
    }

    #[test]
    fn test_parse_file_input_empty() {
        let (data, mime) = parse_file_input("").unwrap();
        assert!(data.is_empty());
        assert_eq!(mime, "application/octet-stream");
    }

    #[test]
    fn test_parse_file_input_data_url_no_mime() {
        let (data, mime) = parse_file_input("data:;base64,SGVsbG8=").unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(mime, "application/octet-stream");
    }

    #[test]
    fn test_parse_file_input_data_url_with_spaces_is_error() {
        // base64 does not allow spaces; this should fail
        assert!(parse_file_input("data:text/plain;base64, SGVsbG8=").is_err());
    }

    #[test]
    fn test_parse_data_url_empty_encoded() {
        let (data, mime) = parse_data_url("data:text/plain,").unwrap();
        assert!(data.is_empty());
        assert_eq!(mime, "text/plain");
    }

    #[test]
    fn test_parse_data_url_only_mime() {
        let (data, mime) = parse_data_url("data:image/png;base64,").unwrap();
        assert!(data.is_empty());
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn test_hex_digit_boundaries() {
        assert_eq!(hex_digit(b'0').unwrap(), 0);
        assert_eq!(hex_digit(b'9').unwrap(), 9);
        assert_eq!(hex_digit(b'a').unwrap(), 10);
        assert_eq!(hex_digit(b'f').unwrap(), 15);
        assert_eq!(hex_digit(b'A').unwrap(), 10);
        assert_eq!(hex_digit(b'F').unwrap(), 15);
    }

    #[test]
    fn test_hex_to_byte_boundaries() {
        assert_eq!(hex_to_byte(b'0', b'0').unwrap(), 0x00);
        assert_eq!(hex_to_byte(b'F', b'F').unwrap(), 0xFF);
        assert_eq!(hex_to_byte(b'7', b'f').unwrap(), 0x7F);
        assert_eq!(hex_to_byte(b'8', b'0').unwrap(), 0x80);
    }

    #[test]
    fn test_percent_decode_all_encoded() {
        let result = percent_decode("%48%65%6C%6C%6F%20%57%6F%72%6C%64").unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_percent_decode_special_chars() {
        let result = percent_decode("%21%40%23%24%25%5E%26%2A%28%29").unwrap();
        assert_eq!(result, b"!@#$%^&*()");
    }

    #[test]
    fn test_percent_decode_invalid_hex() {
        assert!(percent_decode("%GG").is_err());
        assert!(percent_decode("%0G").is_err());
    }
}
