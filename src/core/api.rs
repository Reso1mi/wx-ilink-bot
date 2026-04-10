use anyhow::{Context, Result};
use base64::Engine;
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, warn};

/// iLink Bot API 封装
#[derive(Clone)]
pub struct ILinkAPI {
    /// API 基础地址
    pub base_url: String,
    /// Bot Token
    pub token: Option<String>,
    /// App ID
    pub app_id: String,
    /// 客户端版本号（uint32 编码）
    pub client_version: u32,
    /// HTTP 客户端
    client: Client,
}

/// 长轮询默认超时（毫秒）
const DEFAULT_LONG_POLL_TIMEOUT_MS: u64 = 35_000;
/// 普通 API 默认超时（毫秒）
const DEFAULT_API_TIMEOUT_MS: u64 = 15_000;

/// 扫码状态响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct QRCodeStatusResponse {
    pub status: Option<String>,
    pub bot_token: Option<String>,
    pub ilink_bot_id: Option<String>,
    pub baseurl: Option<String>,
    pub ilink_user_id: Option<String>,
    pub redirect_host: Option<String>,
    #[serde(default)]
    pub errcode: i64,
    pub errmsg: Option<String>,
}

/// 二维码响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct QRCodeResponse {
    pub qrcode: Option<String>,
    pub qrcode_img_content: Option<String>,
    #[serde(default)]
    pub errcode: i64,
    pub errmsg: Option<String>,
}

/// getupdates 响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GetUpdatesResponse {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    pub errmsg: Option<String>,
    #[serde(default)]
    pub msgs: Vec<Value>,
    #[serde(default)]
    pub get_updates_buf: String,
    pub longpolling_timeout_ms: Option<u64>,
}

/// sendmessage 响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SendMessageResponse {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    pub errmsg: Option<String>,
}

/// getconfig 响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GetConfigResponse {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    pub typing_ticket: Option<String>,
}

/// getuploadurl 响应
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GetUploadUrlResponse {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    pub upload_url: Option<String>,
    pub download_url: Option<String>,
    pub file_id: Option<String>,
}

impl ILinkAPI {
    /// 创建新的 API 实例
    pub fn new(base_url: Option<&str>, token: Option<&str>, app_id: &str, version: &str) -> Self {
        let base_url = base_url
            .unwrap_or("https://ilinkai.weixin.qq.com")
            .trim_end_matches('/')
            .to_string();

        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("创建 HTTP 客户端失败");

        Self {
            base_url,
            token: token.map(|t| t.to_string()),
            app_id: app_id.to_string(),
            client_version: Self::build_client_version(version),
            client,
        }
    }

    /// 版本号编码: major<<16 | minor<<8 | patch
    fn build_client_version(version: &str) -> u32 {
        let parts: Vec<u32> = version
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();
        let major = parts.first().copied().unwrap_or(0) & 0xFF;
        let minor = parts.get(1).copied().unwrap_or(0) & 0xFF;
        let patch = parts.get(2).copied().unwrap_or(0) & 0xFF;
        (major << 16) | (minor << 8) | patch
    }

    /// 生成随机 X-WECHAT-UIN
    fn random_wechat_uin() -> String {
        let uint32: u32 = rand::thread_rng().gen();
        let num_str = uint32.to_string();
        base64::engine::general_purpose::STANDARD.encode(num_str.as_bytes())
    }

    /// 构建请求头
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert("AuthorizationType", "ilink_bot_token".parse().unwrap());
        headers.insert("iLink-App-Id", self.app_id.parse().unwrap());
        headers.insert(
            "iLink-App-ClientVersion",
            self.client_version.to_string().parse().unwrap(),
        );
        headers.insert("X-WECHAT-UIN", Self::random_wechat_uin().parse().unwrap());

        if let Some(ref token) = self.token {
            headers.insert(
                "Authorization",
                format!("Bearer {}", token).parse().unwrap(),
            );
        }

        headers
    }

    /// 构建简单请求头（用于扫码相关 GET 请求）
    fn build_simple_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("iLink-App-Id", self.app_id.parse().unwrap());
        headers.insert(
            "iLink-App-ClientVersion",
            self.client_version.to_string().parse().unwrap(),
        );
        headers
    }

    /// 通用 POST 请求
    async fn post(&self, endpoint: &str, mut body: Value, timeout_ms: u64) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, endpoint);

        // 添加 base_info
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "base_info".to_string(),
                json!({"channel_version": "1.0.0"}),
            );
        }

        let timeout = Duration::from_millis(timeout_ms);
        debug!("POST {} timeout={}ms", url, timeout_ms);

        let resp = self
            .client
            .post(&url)
            .headers(self.build_headers())
            .json(&body)
            .timeout(timeout)
            .send()
            .await
            .context(format!("请求 {} 失败", endpoint))?;

        let status = resp.status();
        let text = resp.text().await.context("读取响应体失败")?;

        if !status.is_success() {
            anyhow::bail!("HTTP {} - {}: {}", status, endpoint, text);
        }

        serde_json::from_str(&text).context(format!("解析 {} 响应 JSON 失败: {}", endpoint, text))
    }

    // ==================== 扫码登录 ====================

    /// 获取登录二维码
    ///
    /// - 首次登录（无 token）：使用简单请求头，生成新 Bot 会话
    /// - 绑定新用户（有 token）：携带 Authorization，为当前 Bot 绑定新用户
    pub async fn get_bot_qrcode(&self, bot_type: &str) -> Result<QRCodeResponse> {
        let url = format!(
            "{}/ilink/bot/get_bot_qrcode?bot_type={}",
            self.base_url, bot_type
        );
        debug!("GET {}", url);

        // 有 token 时使用完整请求头（包含 Authorization），确保新用户绑定到当前 Bot
        let headers = if self.token.is_some() {
            self.build_headers()
        } else {
            self.build_simple_headers()
        };

        let resp = self
            .client
            .get(&url)
            .headers(headers)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("获取二维码请求失败")?;

        let text = resp.text().await?;
        serde_json::from_str(&text).context("解析二维码响应失败")
    }

    /// 轮询扫码状态（长轮询，35s 超时）
    pub async fn get_qrcode_status(
        &self,
        base_url: &str,
        qrcode: &str,
    ) -> Result<QRCodeStatusResponse> {
        let url = format!(
            "{}/ilink/bot/get_qrcode_status?qrcode={}",
            base_url, qrcode
        );
        debug!("GET {}", url);

        let headers = if self.token.is_some() {
            self.build_headers()
        } else {
            self.build_simple_headers()
        };

        let resp = self
            .client
            .get(&url)
            .headers(headers)
            .timeout(Duration::from_secs(40))
            .send()
            .await;

        match resp {
            Ok(resp) => {
                let text = resp.text().await?;
                serde_json::from_str(&text).context("解析扫码状态响应失败")
            }
            Err(e) if e.is_timeout() => {
                // 长轮询超时是正常的
                Ok(QRCodeStatusResponse {
                    status: Some("wait".to_string()),
                    bot_token: None,
                    ilink_bot_id: None,
                    baseurl: None,
                    ilink_user_id: None,
                    redirect_host: None,
                    errcode: 0,
                    errmsg: None,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    // ==================== 消息收发 ====================

    /// 长轮询获取消息
    pub async fn get_updates(
        &self,
        get_updates_buf: &str,
        timeout_ms: Option<u64>,
    ) -> Result<GetUpdatesResponse> {
        let timeout = timeout_ms.unwrap_or(DEFAULT_LONG_POLL_TIMEOUT_MS);
        // 客户端超时比服务端多 5s 余量
        let client_timeout = timeout + 5_000;

        let body = json!({
            "get_updates_buf": get_updates_buf,
        });

        match self.post("ilink/bot/getupdates", body, client_timeout).await {
            Ok(value) => serde_json::from_value(value).context("解析 getupdates 响应失败"),
            Err(e) => {
                // 超时是正常的，返回空结果
                if e.to_string().contains("timeout") || e.to_string().contains("Timeout") {
                    warn!("长轮询超时，将继续下一轮");
                    Ok(GetUpdatesResponse {
                        ret: 0,
                        errcode: 0,
                        errmsg: None,
                        msgs: vec![],
                        get_updates_buf: get_updates_buf.to_string(),
                        longpolling_timeout_ms: None,
                    })
                } else {
                    Err(e)
                }
            }
        }
    }

    /// 发送文本消息给指定用户
    pub async fn send_message(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: &str,
        client_id: Option<&str>,
    ) -> Result<SendMessageResponse> {
        let cid = client_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("bot-{:016x}", rand::thread_rng().gen::<u64>()));

        let body = json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": cid,
                "message_type": 2,
                "message_state": 2,
                "item_list": [
                    {
                        "type": 1,
                        "text_item": {"text": text}
                    }
                ],
                "context_token": if context_token.is_empty() { Value::Null } else { Value::String(context_token.to_string()) },
            }
        });

        let value = self
            .post("ilink/bot/sendmessage", body, DEFAULT_API_TIMEOUT_MS)
            .await?;
        serde_json::from_value(value).context("解析 sendmessage 响应失败")
    }

    /// 获取 Bot 配置（typing_ticket 等）
    #[allow(dead_code)]
    pub async fn get_config(
        &self,
        ilink_user_id: &str,
        context_token: &str,
    ) -> Result<GetConfigResponse> {
        let body = json!({
            "ilink_user_id": ilink_user_id,
            "context_token": context_token,
        });

        let value = self
            .post("ilink/bot/getconfig", body, DEFAULT_API_TIMEOUT_MS)
            .await?;
        serde_json::from_value(value).context("解析 getconfig 响应失败")
    }

    /// 发送输入状态指示器 (1=typing, 2=cancel)
    #[allow(dead_code)]
    pub async fn send_typing(
        &self,
        ilink_user_id: &str,
        typing_ticket: &str,
        status: i32,
    ) -> Result<Value> {
        let body = json!({
            "ilink_user_id": ilink_user_id,
            "typing_ticket": typing_ticket,
            "status": status,
        });

        self.post("ilink/bot/sendtyping", body, DEFAULT_API_TIMEOUT_MS)
            .await
    }

    /// 获取媒体上传预签名 URL
    #[allow(dead_code)]
    pub async fn get_upload_url(
        &self,
        filekey: &str,
        media_type: i32,
        to_user_id: &str,
    ) -> Result<GetUploadUrlResponse> {
        let body = json!({
            "filekey": filekey,
            "media_type": media_type,
            "to_user_id": to_user_id,
        });

        let value = self
            .post("ilink/bot/getuploadurl", body, DEFAULT_API_TIMEOUT_MS)
            .await?;
        serde_json::from_value(value).context("解析 getuploadurl 响应失败")
    }

    /// 更新 token
    #[allow(dead_code)]
    pub fn set_token(&mut self, token: &str) {
        self.token = Some(token.to_string());
    }

    /// 更新 base_url
    #[allow(dead_code)]
    pub fn set_base_url(&mut self, url: &str) {
        self.base_url = url.trim_end_matches('/').to_string();
    }
}
