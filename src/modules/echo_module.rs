use anyhow::Result;
use async_trait::async_trait;

use crate::core::parser::ParsedMessage;
use crate::core::router::{MatchType, RouteRule};
use crate::modules::base::{reply, MessageSender, ModuleHandler};

/// 回声模块 — 原样返回用户消息（用于测试）
pub struct EchoModule;

impl EchoModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ModuleHandler for EchoModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let response = format!("你说的是: {}", msg.text);
        reply(sender, msg, &response).await
    }

    fn name(&self) -> &str {
        "EchoModule"
    }

    fn routes(&self) -> Vec<RouteRule> {
        vec![RouteRule::new("echo", "回声", MatchType::Prefix)]
    }
}
