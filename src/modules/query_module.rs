use anyhow::Result;
use async_trait::async_trait;

use crate::core::parser::ParsedMessage;
use crate::modules::base::{reply, MessageSender, ModuleHandler};

/// 查询模块 — 处理 "查询" 开头的消息
pub struct QueryModule;

impl QueryModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ModuleHandler for QueryModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let keyword = msg.text.replace("查询", "").trim().to_string();

        if keyword.is_empty() {
            reply(sender, msg, "请输入查询关键词，例如: 查询 天气").await?;
            return Ok(());
        }

        // 执行查询逻辑（此处为示例实现）
        let result = self.do_query(&keyword).await;
        let response = format!("📋 查询结果:\n{result}");
        reply(sender, msg, &response).await
    }

    fn name(&self) -> &str {
        "QueryModule"
    }
}

impl QueryModule {
    /// 实际查询逻辑（替换为你的业务代码）
    async fn do_query(&self, keyword: &str) -> String {
        // 示例: 调用外部 API、查数据库等
        format!("关于 '{keyword}' 的查询结果...")
    }
}
