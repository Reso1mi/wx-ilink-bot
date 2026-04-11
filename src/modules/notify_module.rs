use anyhow::Result;
use async_trait::async_trait;

use crate::core::parser::ParsedMessage;
use crate::modules::base::{reply, send_to, MessageSender, ModuleHandler};

/// 通知模块 — 处理 "通知" 开头的消息
///
/// 演示跨用户发送能力: 用户 A 发送 "通知 xxx@im.wechat 消息内容"
/// Bot 会将消息转发给指定用户
pub struct NotifyModule;

impl NotifyModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ModuleHandler for NotifyModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let text = msg.text.replace("通知", "").trim().to_string();
        let parts: Vec<&str> = text.splitn(2, ' ').collect();

        if parts.len() < 2 {
            reply(sender, msg, "格式: 通知 <用户ID> <消息内容>").await?;
            return Ok(());
        }

        let target_user_id = parts[0];
        let content = parts[1];

        match send_to(
            sender,
            target_user_id,
            &format!("📢 来自 {} 的通知:\n{}", msg.user_id, content),
        )
        .await
        {
            Ok(_) => {
                reply(sender, msg, &format!("✅ 已通知 {target_user_id}")).await?;
            }
            Err(e) => {
                reply(sender, msg, &format!("❌ 通知失败: {e}")).await?;
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "NotifyModule"
    }
}
