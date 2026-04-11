use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const XHS_LINK_PATTERN: &str = r#"(?:https?://)?(?:www\.)?(?:xiaohongshu\.com/(?:explore|discovery/item|user/profile/[A-Za-z0-9]+/)\S+|xhslink\.com/[^\s"<>\\^`{|}，。；！？、【】《》]+)"#;

static XHS_LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(XHS_LINK_PATTERN).expect("小红书链接正则无效"));

#[derive(Debug, Clone)]
pub struct XhsClientConfig {
    pub api_url: String,
    pub cookie: String,
    pub proxy: String,
    pub timeout_ms: u64,
}

#[derive(Clone)]
pub struct XhsClient {
    api_base_url: String,
    cookie: Option<String>,
    proxy: Option<String>,
    api_client: Client,
    media_client: Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhsWorkKind {
    ImageSet,
    Video,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct XhsWork {
    pub title: String,
    pub note_id: String,
    pub work_type: String,
    pub media_urls: Vec<String>,
    pub kind: XhsWorkKind,
}

#[derive(Debug, Clone)]
pub struct DownloadedMedia {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub extension: String,
}

#[derive(Debug, Serialize)]
struct XhsDetailRequest<'a> {
    url: &'a str,
    download: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cookie: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct XhsDetailResponse {
    message: String,
    #[allow(dead_code)]
    params: Option<Value>,
    data: Option<Value>,
}

impl XhsClientConfig {
    pub fn new(api_url: String, cookie: String, proxy: String, timeout_ms: u64) -> Self {
        Self {
            api_url,
            cookie,
            proxy,
            timeout_ms,
        }
    }
}

impl XhsClient {
    pub fn new(config: XhsClientConfig) -> Result<Self> {
        let api_base_url = config.api_url.trim_end_matches('/').to_string();
        let api_timeout = Duration::from_millis(config.timeout_ms.max(1_000));
        let media_timeout = Duration::from_secs(120);

        Ok(Self {
            api_base_url,
            cookie: non_empty(config.cookie),
            proxy: non_empty(config.proxy),
            api_client: Client::builder()
                .timeout(api_timeout)
                .build()
                .context("创建 XHS API 客户端失败")?,
            media_client: Client::builder()
                .timeout(media_timeout)
                .build()
                .context("创建媒体下载客户端失败")?,
        })
    }

    pub fn extract_first_link(text: &str) -> Option<String> {
        XHS_LINK_REGEX
            .find(text)
            .map(|matched| normalize_url(matched.as_str()))
    }

    pub async fn fetch_work_by_url(&self, url: &str) -> Result<XhsWork> {
        let endpoint = format!("{}/xhs/detail", self.api_base_url);
        let request = XhsDetailRequest {
            url,
            download: false,
            cookie: self.cookie.as_deref(),
            proxy: self.proxy.as_deref(),
        };

        let response = self
            .api_client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .context("请求 XHS-Downloader 失败")?;

        let status = response.status();
        let body = response.text().await.context("读取 XHS 响应失败")?;

        if !status.is_success() {
            anyhow::bail!("XHS-Downloader 返回 HTTP {status}: {body}");
        }

        let payload: XhsDetailResponse =
            serde_json::from_str(&body).context("解析 XHS 响应 JSON 失败")?;
        let data = payload.data.context(payload.message)?;
        parse_work(data)
    }

    pub async fn download_media(&self, url: &str) -> Result<DownloadedMedia> {
        let response = self
            .media_client
            .get(url)
            .send()
            .await
            .with_context(|| format!("下载媒体失败: {url}"))?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("下载媒体失败: HTTP {status} ({url})");
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(strip_content_type)
            .unwrap_or_else(|| guess_content_type(url));

        let bytes = response.bytes().await.context("读取媒体内容失败")?;
        let extension = guess_extension(url, &content_type);

        Ok(DownloadedMedia {
            bytes: bytes.to_vec(),
            content_type,
            extension,
        })
    }
}

fn parse_work(data: Value) -> Result<XhsWork> {
    let title = string_field(&data, "作品标题")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "小红书作品".to_string());
    let note_id = string_field(&data, "作品ID")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "xhs".to_string());
    let work_type = string_field(&data, "作品类型").unwrap_or_else(|| "未知".to_string());
    let media_urls = collect_urls(data.get("下载地址"));

    if media_urls.is_empty() {
        anyhow::bail!("未获取到可下载的无水印媒体地址");
    }

    let kind = match work_type.as_str() {
        "图文" | "图集" => XhsWorkKind::ImageSet,
        "视频" => XhsWorkKind::Video,
        _ if media_urls.len() == 1 && looks_like_video_url(&media_urls[0]) => XhsWorkKind::Video,
        _ if !media_urls.is_empty() => XhsWorkKind::ImageSet,
        _ => XhsWorkKind::Unknown,
    };

    Ok(XhsWork {
        title,
        note_id,
        work_type,
        media_urls,
        kind,
    })
}

fn collect_urls(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "NaN")
            .map(normalize_url)
            .collect(),
        Some(Value::String(item)) => item
            .split_whitespace()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "NaN")
            .map(normalize_url)
            .collect(),
        _ => Vec::new(),
    }
}

fn string_field(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_url(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

fn strip_content_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string()
}

fn guess_content_type(url: &str) -> String {
    match guess_extension(url, "application/octet-stream").as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "webp" => "image/webp".to_string(),
        "gif" => "image/gif".to_string(),
        "heic" => "image/heic".to_string(),
        "avif" => "image/avif".to_string(),
        "mp4" => "video/mp4".to_string(),
        "mov" => "video/quicktime".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn guess_extension(url: &str, content_type: &str) -> String {
    if let Some(extension) = extension_from_content_type(content_type) {
        return extension.to_string();
    }

    let path = url.split('?').next().unwrap_or(url);
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .unwrap_or_else(|| "bin".to_string())
}

fn extension_from_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/heic" => Some("heic"),
        "image/avif" => Some("avif"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        _ => None,
    }
}

fn looks_like_video_url(url: &str) -> bool {
    matches!(
        guess_extension(url, "application/octet-stream").as_str(),
        "mp4" | "mov" | "m4v" | "webm"
    )
}
