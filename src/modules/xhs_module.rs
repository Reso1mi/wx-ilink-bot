use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};

use crate::core::parser::ParsedMessage;
use crate::core::xhs_client::{XhsClient, XhsClientConfig, XhsWork, XhsWorkKind};
use crate::modules::base::{reply, MessageSender, ModuleHandler};

pub struct XhsModule {
    client: XhsClient,
}

impl XhsModule {
    pub fn new(config: XhsClientConfig) -> Result<Self> {
        Ok(Self {
            client: XhsClient::new(config)?,
        })
    }

    async fn send_images(
        &self,
        work: &XhsWork,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let total = work.media_urls.len();
        let mut success = 0usize;

        reply(
            sender,
            msg,
            &format!(
                "已解析到小红书{}《{}》，共 {} 张图片，开始回传无水印原图。",
                work.work_type, work.title, total
            ),
        )
        .await?;

        for (index, url) in work.media_urls.iter().enumerate() {
            match self.client.download_media(url).await {
                Ok(media) => {
                    let file_name = format!(
                        "xhs_{}_{}.{}",
                        sanitize_file_stem(&work.note_id),
                        index + 1,
                        media.extension
                    );

                    if let Err(error) = sender
                        .upload_and_send_image(
                            &msg.user_id,
                            &media.bytes,
                            &file_name,
                            &media.content_type,
                        )
                        .await
                    {
                        warn!(
                            "发送小红书图片失败: user={} note_id={} index={} error={}",
                            msg.user_id,
                            work.note_id,
                            index + 1,
                            error
                        );
                    } else {
                        success += 1;
                    }
                }
                Err(error) => {
                    warn!(
                        "下载小红书图片失败: user={} note_id={} index={} error={}",
                        msg.user_id,
                        work.note_id,
                        index + 1,
                        error
                    );
                }
            }
        }

        let summary = match success {
            0 => "图片下载或发送失败，请稍后重试。".to_string(),
            value if value == total => format!("已发送完成，共返回 {total} 张无水印图片。"),
            value => format!("已返回 {value}/{total} 张图片，部分图片处理失败。"),
        };

        reply(sender, msg, &summary).await
    }

    async fn send_video(
        &self,
        work: &XhsWork,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let video_url = work
            .media_urls
            .first()
            .ok_or_else(|| anyhow::anyhow!("未获取到视频下载地址"))?;

        reply(
            sender,
            msg,
            &format!("已解析到小红书视频《{}》，开始回传无水印视频。", work.title),
        )
        .await?;

        let media = self.client.download_media(video_url).await?;
        let stem = sanitize_file_stem(&work.note_id);
        let file_name = format!("xhs_{}.{}", stem, media.extension);

        match sender
            .upload_and_send_video(
                &msg.user_id,
                &media.bytes,
                &file_name,
                &media.content_type,
                1,
            )
            .await
        {
            Ok(_) => reply(sender, msg, "视频发送完成。").await,
            Err(video_error) => {
                warn!(
                    "发送小红书视频消息失败，回退为文件发送: user={} note_id={} error={}",
                    msg.user_id, work.note_id, video_error
                );
                sender
                    .upload_and_send_file(
                        &msg.user_id,
                        &media.bytes,
                        &file_name,
                        &media.content_type,
                    )
                    .await?;
                reply(sender, msg, "视频已作为文件发送完成。").await
            }
        }
    }
}

#[async_trait]
impl ModuleHandler for XhsModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let link = match XhsClient::extract_first_link(&msg.text) {
            Some(link) => link,
            None => {
                reply(sender, msg, "未识别到有效的小红书链接，请重新发送。").await?;
                return Ok(());
            }
        };

        info!("开始处理小红书链接: user={} link={}", msg.user_id, link);

        match self.client.fetch_work_by_url(&link).await {
            Ok(work) => match work.kind {
                XhsWorkKind::ImageSet => self.send_images(&work, msg, sender).await?,
                XhsWorkKind::Video => self.send_video(&work, msg, sender).await?,
                XhsWorkKind::Unknown => {
                    reply(sender, msg, "当前作品类型暂不支持处理。").await?;
                }
            },
            Err(error) => {
                warn!(
                    "处理小红书链接失败: user={} link={} error={}",
                    msg.user_id, link, error
                );
                reply(sender, msg, &format!("解析小红书链接失败: {error}")).await?;
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "XhsModule"
    }
}

fn sanitize_file_stem(value: &str) -> String {
    let filtered: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if filtered.is_empty() {
        "xhs".to_string()
    } else {
        filtered
    }
}
