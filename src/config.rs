use std::env;

/// 应用配置
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// 状态文件存储目录
    pub state_dir: String,
    /// 日志级别
    pub log_level: String,
    /// iLink API 基础地址
    pub base_url: String,
    /// App ID
    pub app_id: String,
    /// 客户端版本号
    pub version: String,
    /// HTTP 管理接口端口
    pub http_port: u16,
    /// XHS-Downloader API 地址
    pub xhs_api_url: String,
    /// 小红书请求 Cookie（可选）
    pub xhs_cookie: String,
    /// 小红书请求代理（可选）
    pub xhs_proxy: String,
    /// XHS-Downloader API 超时时间（毫秒）
    pub xhs_timeout_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            state_dir: "./state".to_string(),
            log_level: "info".to_string(),
            base_url: "https://ilinkai.weixin.qq.com".to_string(),
            app_id: "bot".to_string(),
            version: "1.0.0".to_string(),
            http_port: 3000,
            xhs_api_url: "http://127.0.0.1:5556".to_string(),
            xhs_cookie: String::new(),
            xhs_proxy: String::new(),
            xhs_timeout_ms: 60_000,
        }
    }
}

impl AppConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        // 尝试加载 .env 文件（忽略错误）
        let _ = dotenvy::dotenv();

        Self {
            state_dir: env::var("BOT_STATE_DIR").unwrap_or_else(|_| "./state".to_string()),
            log_level: env::var("BOT_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            base_url: env::var("BOT_BASE_URL")
                .unwrap_or_else(|_| "https://ilinkai.weixin.qq.com".to_string()),
            app_id: env::var("BOT_APP_ID").unwrap_or_else(|_| "bot".to_string()),
            version: env::var("BOT_VERSION").unwrap_or_else(|_| "1.0.0".to_string()),
            http_port: env::var("BOT_HTTP_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            xhs_api_url: env::var("BOT_XHS_API_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:5556".to_string()),
            xhs_cookie: env::var("BOT_XHS_COOKIE").unwrap_or_default(),
            xhs_proxy: env::var("BOT_XHS_PROXY").unwrap_or_default(),
            xhs_timeout_ms: env::var("BOT_XHS_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60_000),
        }
    }
}
