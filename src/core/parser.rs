use serde_json::Value;

/// 消息内容类型常量
pub mod message_item_type {
    #[allow(dead_code)]
    pub const NONE: i64 = 0;
    pub const TEXT: i64 = 1;
    pub const IMAGE: i64 = 2;
    pub const VOICE: i64 = 3;
    pub const FILE: i64 = 4;
    pub const VIDEO: i64 = 5;
}

/// 消息类型枚举
#[derive(Debug, Clone, PartialEq)]
pub enum MessageType {
    Text,
    Image,
    Voice,
    File,
    Video,
    Unknown,
}

impl MessageType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Voice => "voice",
            Self::File => "file",
            Self::Video => "video",
            Self::Unknown => "unknown",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "text" => Self::Text,
            "image" => Self::Image,
            "voice" => Self::Voice,
            "file" => Self::File,
            "video" => Self::Video,
            _ => Self::Unknown,
        }
    }
}

/// CDN 媒体引用信息
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct MediaInfo {
    /// CDN 下载加密参数
    pub encrypt_query_param: String,
    /// AES-128 密钥（base64 编码）
    pub aes_key: String,
    /// 加密类型
    pub encrypt_type: i64,
    /// 完整下载 URL
    pub full_url: String,
    /// 文件名（仅 File 类型）
    pub file_name: String,
    /// 文件 MD5（仅 File 类型）
    pub md5: String,
    /// 文件大小
    pub size: String,
    /// 编码类型（仅 Voice 类型）
    pub encode_type: Option<i64>,
    /// 播放时长（仅 Voice 类型）
    pub playtime: Option<i64>,
    /// 视频大小（仅 Video 类型）
    pub video_size: Option<i64>,
    /// 播放时长（仅 Video 类型）
    pub play_length: Option<i64>,
    /// 缩略图媒体信息
    pub thumb_media: Option<Value>,
}

/// 解析后的消息结构
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ParsedMessage {
    /// 发送者 ID
    pub user_id: String,
    /// 文本内容
    pub text: String,
    /// 会话令牌
    pub context_token: String,
    /// 消息类型
    pub message_type: MessageType,
    /// 媒体信息
    pub media_info: Option<MediaInfo>,
    /// 引用消息文本
    pub ref_text: Option<String>,
    /// 原始消息
    pub raw_message: Value,
    /// 消息创建时间（毫秒时间戳）
    pub create_time_ms: i64,
    /// 会话 ID
    pub session_id: String,
    /// 群组 ID
    pub group_id: String,
}

impl Default for ParsedMessage {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            text: String::new(),
            context_token: String::new(),
            message_type: MessageType::Unknown,
            media_info: None,
            ref_text: None,
            raw_message: Value::Null,
            create_time_ms: 0,
            session_id: String::new(),
            group_id: String::new(),
        }
    }
}

/// 消息解析器 — 从 WeixinMessage 中提取结构化信息
pub struct MessageParser;

impl MessageParser {
    pub fn new() -> Self {
        Self
    }

    /// 解析一条 WeixinMessage
    pub fn parse(&self, msg: &Value) -> ParsedMessage {
        let mut parsed = ParsedMessage {
            user_id: msg["from_user_id"].as_str().unwrap_or("").to_string(),
            context_token: msg["context_token"].as_str().unwrap_or("").to_string(),
            raw_message: msg.clone(),
            create_time_ms: msg["create_time_ms"].as_i64().unwrap_or(0),
            session_id: msg["session_id"].as_str().unwrap_or("").to_string(),
            group_id: msg["group_id"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        };

        let item_list = match msg["item_list"].as_array() {
            Some(list) => list,
            None => return parsed,
        };

        for item in item_list {
            let item_type = item["type"].as_i64().unwrap_or(0);

            match item_type {
                message_item_type::TEXT => {
                    parsed.message_type = MessageType::Text;
                    parsed.text = Self::extract_text(item);
                    if let Some(ref_text) = Self::extract_ref_text(item) {
                        parsed.ref_text = Some(ref_text);
                    }
                }
                message_item_type::IMAGE => {
                    parsed.message_type = MessageType::Image;
                    parsed.media_info = Some(Self::extract_image_info(item));
                }
                message_item_type::VOICE => {
                    parsed.message_type = MessageType::Voice;
                    // 语音转文字：优先使用文字内容
                    let voice_text = item["voice_item"]["text"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    if !voice_text.is_empty() {
                        parsed.text = voice_text;
                    }
                    parsed.media_info = Some(Self::extract_voice_info(item));
                }
                message_item_type::FILE => {
                    parsed.message_type = MessageType::File;
                    parsed.media_info = Some(Self::extract_file_info(item));
                }
                message_item_type::VIDEO => {
                    parsed.message_type = MessageType::Video;
                    parsed.media_info = Some(Self::extract_video_info(item));
                }
                _ => {}
            }
        }

        parsed
    }

    /// 提取文本内容
    fn extract_text(item: &Value) -> String {
        item["text_item"]["text"].as_str().unwrap_or("").to_string()
    }

    /// 提取引用消息文本
    fn extract_ref_text(item: &Value) -> Option<String> {
        let ref_msg = &item["ref_msg"];
        if ref_msg.is_null() {
            return None;
        }

        let mut parts = Vec::new();

        if let Some(title) = ref_msg["title"].as_str() {
            if !title.is_empty() {
                parts.push(title.to_string());
            }
        }

        let ref_item = &ref_msg["message_item"];
        if !ref_item.is_null() && ref_item["type"].as_i64() == Some(message_item_type::TEXT) {
            if let Some(text) = ref_item["text_item"]["text"].as_str() {
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }

    /// 提取图片信息
    fn extract_image_info(item: &Value) -> MediaInfo {
        let image = &item["image_item"];
        Self::extract_cdn_media(&image["media"], |info| {
            info.thumb_media = Some(image["thumb_media"].clone());
            if let Some(aeskey) = image["aeskey"].as_str() {
                info.aes_key = aeskey.to_string();
            }
        })
    }

    /// 提取语音信息
    fn extract_voice_info(item: &Value) -> MediaInfo {
        let voice = &item["voice_item"];
        Self::extract_cdn_media(&voice["media"], |info| {
            info.encode_type = voice["encode_type"].as_i64();
            info.playtime = voice["playtime"].as_i64();
        })
    }

    /// 提取文件信息
    fn extract_file_info(item: &Value) -> MediaInfo {
        let file_item = &item["file_item"];
        Self::extract_cdn_media(&file_item["media"], |info| {
            info.file_name = file_item["file_name"].as_str().unwrap_or("").to_string();
            info.md5 = file_item["md5"].as_str().unwrap_or("").to_string();
            info.size = file_item["len"]
                .as_str()
                .or_else(|| file_item["len"].as_i64().map(|_| ""))
                .unwrap_or("0")
                .to_string();
            // 如果 len 是数字而不是字符串
            if info.size.is_empty() {
                info.size = file_item["len"].as_i64().unwrap_or(0).to_string();
            }
        })
    }

    /// 提取视频信息
    fn extract_video_info(item: &Value) -> MediaInfo {
        let video = &item["video_item"];
        Self::extract_cdn_media(&video["media"], |info| {
            info.video_size = video["video_size"].as_i64();
            info.play_length = video["play_length"].as_i64();
        })
    }

    /// 提取 CDN 媒体引用信息
    fn extract_cdn_media<F>(media: &Value, mut customize: F) -> MediaInfo
    where
        F: FnMut(&mut MediaInfo),
    {
        let mut info = MediaInfo {
            encrypt_query_param: media["encrypt_query_param"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            aes_key: media["aes_key"].as_str().unwrap_or("").to_string(),
            encrypt_type: media["encrypt_type"].as_i64().unwrap_or(0),
            full_url: media["full_url"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        };

        customize(&mut info);
        info
    }
}
