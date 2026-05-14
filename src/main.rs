use tracing::info;
use turbomcp::prelude::*;

use liberado_anythingllm_mcp::{AnythingLlmServer, ServerConfig, TransportConfig};

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

fn api_key_status(config: &ServerConfig) -> &'static str {
    if config.anythingllm_api_key.is_empty() {
        tracing::warn!(
            "ANYTHINGLLM_API_KEY is NOT SET — all requests to AnythingLLM will fail with auth errors"
        );
        "UNSET"
    } else {
        info!(
            "ANYTHINGLLM_API_KEY is set ({} chars)",
            config.anythingllm_api_key.len()
        );
        "SET"
    }
}

fn log_startup(config: &ServerConfig, key_status: &'static str) {
    info!(
        base_url = %config.anythingllm_base_url,
        api_key = %key_status,
        "liberado-anythingllm-mcp starting"
    );
}

async fn run(config: ServerConfig) {
    let key_status = api_key_status(&config);
    log_startup(&config, key_status);
    let transport_cfg = config.transport.clone();
    let server = AnythingLlmServer::new(config);

    let builder = server.builder().with_protocol(ProtocolConfig {
        allow_fallback: true,
        ..Default::default()
    });

    let server = match transport_cfg {
        TransportConfig::Stdio => builder.transport(turbomcp::Transport::stdio()),
        TransportConfig::Http { host, port } => {
            let addr = format!("{host}:{port}");
            info!(addr = %addr, "HTTP transport enabled");
            builder
                .transport(turbomcp::Transport::http(addr))
                .allow_any_origin(true)
        }
    };

    server.serve().await.unwrap();
}

#[tokio::main]
async fn main() {
    init_tracing();
    let config = ServerConfig::from_env();
    run(config).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_status_set() {
        let config = ServerConfig {
            anythingllm_base_url: "http://test:3001".to_string(),
            anythingllm_api_key: "sk-abc123".to_string(),
            transport: TransportConfig::Stdio,
        };
        assert_eq!(api_key_status(&config), "SET");
    }

    #[test]
    fn test_api_key_status_unset() {
        let config = ServerConfig {
            anythingllm_base_url: "http://test:3001".to_string(),
            anythingllm_api_key: String::new(),
            transport: TransportConfig::Stdio,
        };
        assert_eq!(api_key_status(&config), "UNSET");
    }

    #[test]
    fn test_log_startup_does_not_panic() {
        let config = ServerConfig {
            anythingllm_base_url: "http://test:3001".to_string(),
            anythingllm_api_key: "key".to_string(),
            transport: TransportConfig::Stdio,
        };
        log_startup(&config, "SET");
    }

    #[tokio::test]
    async fn test_run_http_binds_and_cancels() {
        let config = ServerConfig {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: "test-key".to_string(),
            transport: TransportConfig::Http {
                host: "127.0.0.1".to_string(),
                port: 0,
            },
        };

        let handle = tokio::spawn(async {
            run(config).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        handle.abort();
        let result = handle.await;
        assert!(result.is_err());
    }
}
