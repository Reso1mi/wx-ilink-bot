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
        }
    }
}
