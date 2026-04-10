use anyhow::Result;
use async_trait::async_trait;

use crate::core::parser::ParsedMessage;

/// 消息发送接口 — Bot 主类实现此 trait 供模块使用
///
/// 支持发送文本、图片、文件、视频消息。
/// 图片/文件/视频需要先上传获取 file_id 和 download_url，再调用对应的发送方法。
/// 也可以使用 `upload_and_send_*` 快捷方法，一步完成上传+发送。
#[async_trait]
#[allow(dead_code)]
pub trait MessageSender: Send + Sync {
    /// 发送文本消息给指定用户
    async fn send_text(&self, to_user_id: &str, text: &str) -> Result<()>;

    /// 发送图片消息（已上传的媒体）
    async fn send_image(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        aes_key: &str,
    ) -> Result<()>;

    /// 发送文件消息（已上传的媒体）
    async fn send_file(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        file_name: &str,
        file_size: i64,
    ) -> Result<()>;

    /// 发送视频消息（已上传的媒体）
    async fn send_video(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        video_size: i64,
        play_length: i64,
    ) -> Result<()>;

    /// 上传并发送图片（一步完成）
    async fn upload_and_send_image(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()>;

    /// 上传并发送文件（一步完成）
    async fn upload_and_send_file(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()>;

    /// 上传并发送视频（一步完成）
    async fn upload_and_send_video(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
        play_length: i64,
    ) -> Result<()>;
}

/// 业务模块处理 trait
///
/// 所有业务模块必须实现此 trait。
/// 模块通过 `MessageSender` 发送消息，实现与用户的交互。
#[async_trait]
pub trait ModuleHandler: Send + Sync {
    /// 处理消息
    ///
    /// 参数:
    ///   - msg: 解析后的消息对象
    ///   - sender: 消息发送器，用于回复/转发消息
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()>;

    /// 模块名称（用于日志和路由注册）
    fn name(&self) -> &str;
}

/// 回复辅助函数 — 直接回复消息发送者
pub async fn reply(sender: &dyn MessageSender, msg: &ParsedMessage, text: &str) -> Result<()> {
    sender.send_text(&msg.user_id, text).await
}

/// 发送辅助函数 — 发送消息给指定用户
pub async fn send_to(sender: &dyn MessageSender, user_id: &str, text: &str) -> Result<()> {
    sender.send_text(user_id, text).await
}
