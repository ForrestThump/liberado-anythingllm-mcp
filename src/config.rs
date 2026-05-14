#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TransportConfig {
    #[default]
    Stdio,
    Http {
        host: String,
        port: u16,
    },
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub anythingllm_base_url: String,
    pub anythingllm_api_key: String,
    pub transport: TransportConfig,
}

fn default_http_port() -> u16 {
    8080
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            anythingllm_base_url: "http://anythingllm:3001".to_string(),
            anythingllm_api_key: String::new(),
            transport: TransportConfig::Stdio,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("ANYTHINGLLM_BASE_URL") {
            config.anythingllm_base_url = val;
        }
        if let Ok(val) = std::env::var("ANYTHINGLLM_API_KEY") {
            config.anythingllm_api_key = val;
        }
        if let Ok(val) = std::env::var("MCP_ANYTHINGLLM_TRANSPORT") {
            match val.to_lowercase().as_str() {
                "http" => {
                    let host = std::env::var("MCP_ANYTHINGLLM_HTTP_HOST")
                        .unwrap_or_else(|_| "0.0.0.0".to_string());
                    let port = std::env::var("MCP_ANYTHINGLLM_HTTP_PORT")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(default_http_port);
                    config.transport = TransportConfig::Http { host, port };
                }
                _ => {
                    config.transport = TransportConfig::Stdio;
                }
            }
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_default_transport_is_stdio() {
        let config = ServerConfig::default();
        assert_eq!(config.transport, TransportConfig::Stdio);
    }

    #[test]
    #[serial]
    fn test_from_env_defaults() {
        unsafe { std::env::remove_var("ANYTHINGLLM_BASE_URL") };
        unsafe { std::env::remove_var("ANYTHINGLLM_API_KEY") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_TRANSPORT") };
        let config = ServerConfig::from_env();
        assert_eq!(config.anythingllm_base_url, "http://anythingllm:3001");
        assert_eq!(config.anythingllm_api_key, "");
        assert_eq!(config.transport, TransportConfig::Stdio);
    }

    #[test]
    #[serial]
    fn test_from_env_custom_url_and_key() {
        unsafe { std::env::set_var("ANYTHINGLLM_BASE_URL", "http://custom:3001") };
        unsafe { std::env::set_var("ANYTHINGLLM_API_KEY", "sk-test123") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_TRANSPORT") };
        let config = ServerConfig::from_env();
        assert_eq!(config.anythingllm_base_url, "http://custom:3001");
        assert_eq!(config.anythingllm_api_key, "sk-test123");
        assert_eq!(config.transport, TransportConfig::Stdio);
    }

    #[test]
    #[serial]
    fn test_from_env_http_transport_defaults() {
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_TRANSPORT", "http") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_HTTP_HOST") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_HTTP_PORT") };
        let config = ServerConfig::from_env();
        assert_eq!(
            config.transport,
            TransportConfig::Http {
                host: "0.0.0.0".to_string(),
                port: 8080,
            }
        );
    }

    #[test]
    #[serial]
    fn test_from_env_http_transport_custom() {
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_TRANSPORT", "http") };
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_HTTP_HOST", "127.0.0.1") };
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_HTTP_PORT", "9000") };
        let config = ServerConfig::from_env();
        assert_eq!(
            config.transport,
            TransportConfig::Http {
                host: "127.0.0.1".to_string(),
                port: 9000,
            }
        );
    }

    #[test]
    #[serial]
    fn test_from_env_invalid_transport_falls_back_to_stdio() {
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_TRANSPORT", "tcp") };
        let config = ServerConfig::from_env();
        assert_eq!(config.transport, TransportConfig::Stdio);
    }

    #[test]
    #[serial]
    fn test_from_env_http_transport_case_insensitive() {
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_TRANSPORT", "HTTP") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_HTTP_HOST") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_HTTP_PORT") };
        let config = ServerConfig::from_env();
        assert_eq!(
            config.transport,
            TransportConfig::Http {
                host: "0.0.0.0".to_string(),
                port: 8080,
            }
        );
    }

    #[test]
    #[serial]
    fn test_from_env_invalid_port_falls_back_to_default() {
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_TRANSPORT", "http") };
        unsafe { std::env::set_var("MCP_ANYTHINGLLM_HTTP_PORT", "not-a-port") };
        unsafe { std::env::remove_var("MCP_ANYTHINGLLM_HTTP_HOST") };
        let config = ServerConfig::from_env();
        assert_eq!(
            config.transport,
            TransportConfig::Http {
                host: "0.0.0.0".to_string(),
                port: 8080,
            }
        );
    }
}
