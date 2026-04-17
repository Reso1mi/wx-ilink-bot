use anyhow::Result;
use async_trait::async_trait;

use crate::config::AppConfig;
use crate::core::todo_store::TodoStore;
use crate::core::parser::ParsedMessage;
use crate::core::router::{MatchType, RouteRule};
use crate::modules::base::{reply, MessageSender, ModuleHandler};

/// TODO 模块 — 处理待办事项相关命令
///
/// 命令:
/// - `待办 <内容>` / `todo <内容>` — 添加待办
/// - `待办列表` / `todo list` — 查看未完成
/// - `完成 <编号>` / `done <编号>` — 标记完成
/// - `删除待办 <编号>` — 删除待办
/// - `所有待办` — 查看全部（含已完成）
/// - `清空已完成` — 批量删除已完成项
pub struct TodoModule {
    todo_store: TodoStore,
}

impl TodoModule {
    pub async fn new(config: &AppConfig) -> Self {
        let todo_store = TodoStore::new(&config.state_dir).await;
        Self { todo_store }
    }
}

#[async_trait]
impl ModuleHandler for TodoModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let text = msg.text.trim();

        // 精确匹配命令
        if text == "待办列表" || text.eq_ignore_ascii_case("todo list") {
            return self.handle_list_pending(msg, sender).await;
        }
        if text == "所有待办" {
            return self.handle_list_all(msg, sender).await;
        }
        if text == "清空已完成" {
            return self.handle_clear_done(msg, sender).await;
        }

        // 前缀匹配命令
        if text.starts_with("完成") || text.starts_with("done ") {
            let num_str = text
                .replacen("完成", "", 1)
                .replacen("done ", "", 1)
                .trim()
                .to_string();
            return self.handle_complete(msg, sender, &num_str).await;
        }
        if text.starts_with("删除待办") {
            let num_str = text.replacen("删除待办", "", 1).trim().to_string();
            return self.handle_delete(msg, sender, &num_str).await;
        }

        // 添加待办（「待办 xxx」或「todo xxx」）
        if text.starts_with("待办") {
            let content = text.replacen("待办", "", 1).trim().to_string();
            return self.handle_add(msg, sender, &content).await;
        }
        if text.to_ascii_lowercase().starts_with("todo ") {
            let content = text[5..].trim().to_string();
            return self.handle_add(msg, sender, &content).await;
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "TodoModule"
    }

    fn routes(&self) -> Vec<RouteRule> {
        vec![
            RouteRule::new("todo_add", "待办", MatchType::Prefix),
            RouteRule::new("todo_add_en", "todo ", MatchType::Prefix),
            RouteRule::new("todo_complete", "完成", MatchType::Prefix),
            RouteRule::new("todo_complete_en", "done ", MatchType::Prefix),
            RouteRule::new("todo_delete", "删除待办", MatchType::Prefix),
            RouteRule::new("todo_all", "所有待办", MatchType::Exact),
            RouteRule::new("todo_clear", "清空已完成", MatchType::Exact),
        ]
    }
}

impl TodoModule {
    /// 添加待办
    async fn handle_add(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
        content: &str,
    ) -> Result<()> {
        if content.is_empty() {
            reply(sender, msg, "请输入待办内容，例如: 待办 明天下午3点开会").await?;
            return Ok(());
        }

        if content.len() > 500 {
            reply(sender, msg, "待办内容太长了，请控制在 500 字以内").await?;
            return Ok(());
        }

        let item = self.todo_store.add(&msg.user_id, content).await;
        reply(
            sender,
            msg,
            &format!("✅ 待办已添加:\n#{} {}", item.id, item.content),
        )
        .await
    }

    /// 查看未完成待办
    async fn handle_list_pending(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let pending = self.todo_store.list_pending(&msg.user_id).await;
        let done_count = self.todo_store.count_done(&msg.user_id).await;

        if pending.is_empty() && done_count == 0 {
            reply(sender, msg, "📋 你还没有任何待办\n\n发送「待办 <内容>」添加").await?;
            return Ok(());
        }

        if pending.is_empty() {
            reply(
                sender,
                msg,
                &format!(
                    "📋 没有未完成的待办\n\n✅ 已完成 {} 项（发送「所有待办」查看）",
                    done_count
                ),
            )
            .await?;
            return Ok(());
        }

        let mut lines = Vec::new();
        for item in &pending {
            lines.push(format!("{}. ⬜ {}", item.id, item.content));
        }

        let mut response = format!("📋 你的待办 ({}项):\n{}", pending.len(), lines.join("\n"));

        if done_count > 0 {
            response.push_str(&format!(
                "\n\n✅ 已完成 {} 项（发送「所有待办」查看）",
                done_count
            ));
        }

        reply(sender, msg, &response).await
    }

    /// 查看所有待办（含已完成）
    async fn handle_list_all(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let all = self.todo_store.list_all(&msg.user_id).await;

        if all.is_empty() {
            reply(sender, msg, "📋 你还没有任何待办\n\n发送「待办 <内容>」添加").await?;
            return Ok(());
        }

        let mut lines = Vec::new();
        for item in &all {
            let status = if item.done { "✅" } else { "⬜" };
            lines.push(format!("{}. {} {}", item.id, status, item.content));
        }

        let pending_count = all.iter().filter(|i| !i.done).count();
        let done_count = all.len() - pending_count;

        let response = format!(
            "📋 所有待办 ({}项未完成, {}项已完成):\n{}",
            pending_count,
            done_count,
            lines.join("\n")
        );

        reply(sender, msg, &response).await
    }

    /// 完成待办
    async fn handle_complete(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
        num_str: &str,
    ) -> Result<()> {
        let item_id: u32 = match num_str.trim_start_matches('#').parse() {
            Ok(id) => id,
            Err(_) => {
                reply(sender, msg, "请输入待办编号，例如: 完成 1").await?;
                return Ok(());
            }
        };

        match self.todo_store.complete(&msg.user_id, item_id).await {
            Some(item) => {
                reply(
                    sender,
                    msg,
                    &format!("✅ 已完成: #{} {}", item.id, item.content),
                )
                .await
            }
            None => {
                reply(sender, msg, &format!("未找到编号 #{} 的未完成待办", item_id)).await
            }
        }
    }

    /// 删除待办
    async fn handle_delete(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
        num_str: &str,
    ) -> Result<()> {
        let item_id: u32 = match num_str.trim_start_matches('#').parse() {
            Ok(id) => id,
            Err(_) => {
                reply(sender, msg, "请输入待办编号，例如: 删除待办 1").await?;
                return Ok(());
            }
        };

        match self.todo_store.delete(&msg.user_id, item_id).await {
            Some(item) => {
                reply(
                    sender,
                    msg,
                    &format!("🗑️ 已删除: #{} {}", item.id, item.content),
                )
                .await
            }
            None => reply(sender, msg, &format!("未找到编号 #{} 的待办", item_id)).await,
        }
    }

    /// 清空已完成
    async fn handle_clear_done(
        &self,
        msg: &ParsedMessage,
        sender: &dyn MessageSender,
    ) -> Result<()> {
        let removed = self.todo_store.clear_done(&msg.user_id).await;
        if removed == 0 {
            reply(sender, msg, "没有已完成的待办需要清理").await
        } else {
            reply(
                sender,
                msg,
                &format!("🗑️ 已清空 {} 条已完成的待办", removed),
            )
            .await
        }
    }
}
