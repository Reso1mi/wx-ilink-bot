use anyhow::Result;
use async_trait::async_trait;

use crate::core::nickname_store::NicknameStore;
use crate::core::parser::ParsedMessage;
use crate::core::router::{MatchType, RouteRule};
use crate::core::session::ContextTokenStore;
use crate::modules::base::{reply, MessageSender, ModuleHandler};

/// 昵称模块 — 处理「叫我」「我是谁」「用户列表」命令
pub struct NicknameModule {
    nickname_store: NicknameStore,
    ctx_store: ContextTokenStore,
}

impl NicknameModule {
    pub fn new(nickname_store: NicknameStore, ctx_store: ContextTokenStore) -> Self {
        Self {
            nickname_store,
            ctx_store,
        }
    }
}

#[async_trait]
impl ModuleHandler for NicknameModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let text = msg.text.trim();

        if text == "我是谁" {
            return self.handle_whoami(msg, sender).await;
        }

        if text == "用户列表" {
            return self.handle_user_list(msg, sender).await;
        }

        // 「叫我 xxx」
        if text.starts_with("叫我") {
            let nickname = text.replacen("叫我", "", 1).trim().to_string();
            return self.handle_set_nickname(msg, sender, &nickname).await;
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "NicknameModule"
    }

    fn routes(&self) -> Vec<RouteRule> {
        vec![
            RouteRule::new("nickname_call", "叫我", MatchType::Prefix),
            RouteRule::new("nickname_whoami", "我是谁", MatchType::Exact),
            RouteRule::new("nickname_users", "用户列表", MatchType::Exact),
        ]
    }
}

impl NicknameModule {
    /// 处理「叫我 xxx」
    async fn handle_set_nickname(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
        nickname: &str,
    ) -> Result<()> {
        if nickname.is_empty() {
            reply(sender, msg, "请告诉我你的昵称，例如: 叫我大王").await?;
            return Ok(());
        }

        if nickname.len() > 60 {
            reply(sender, msg, "昵称太长了，请控制在 20 个字以内").await?;
            return Ok(());
        }

        // 检查昵称是否已被其他用户使用
        if let Some(existing_uid) = self.nickname_store.find_by_nickname(nickname).await {
            if existing_uid != msg.user_id {
                reply(sender, msg, &format!("昵称「{}」已被其他用户使用，请换一个", nickname)).await?;
                return Ok(());
            }
        }

        self.nickname_store.set(&msg.user_id, nickname).await;
        reply(sender, msg, &format!("✅ 好的，以后叫你「{}」", nickname)).await
    }

    /// 处理「我是谁」
    async fn handle_whoami(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let display = self.nickname_store.display_name(&msg.user_id).await;
        match self.nickname_store.get(&msg.user_id).await {
            Some(nickname) => {
                reply(sender, msg, &format!("你的昵称是「{}」\n\n发送「叫我 <新昵称>」可以修改", nickname)).await
            }
            None => {
                reply(
                    sender,
                    msg,
                    &format!("你还没有设置昵称，当前显示为「{}」\n\n发送「叫我 <昵称>」来设置", display),
                )
                .await
            }
        }
    }

    /// 处理「用户列表」
    async fn handle_user_list(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        // 收集所有账号下的用户
        let all_users = self.ctx_store.get_all_user_ids().await;

        if all_users.is_empty() {
            reply(sender, msg, "📭 当前没有已连接的用户").await?;
            return Ok(());
        }

        let mut lines = Vec::new();
        for (index, user_id) in all_users.iter().enumerate() {
            let display = self.nickname_store.display_name(user_id).await;
            let has_nickname = self.nickname_store.get(user_id).await.is_some();
            if has_nickname {
                lines.push(format!("{}. {}", index + 1, display));
            } else {
                lines.push(format!("{}. {} (未设昵称)", index + 1, display));
            }
        }

        let response = format!(
            "👥 当前用户 ({}人):\n{}",
            all_users.len(),
            lines.join("\n")
        );
        reply(sender, msg, &response).await
    }
}
