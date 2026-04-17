use anyhow::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;

use crate::core::parser::ParsedMessage;
use crate::modules::base::{reply, MessageSender, ModuleHandler};

/// 帮助模块 — 默认处理器，显示可用命令
pub struct HelpModule {
    commands: BTreeMap<String, String>,
}

impl HelpModule {
    pub fn new(commands: Vec<(&str, &str)>) -> Self {
        let mut map = BTreeMap::new();
        for (cmd, desc) in commands {
            map.insert(cmd.to_string(), desc.to_string());
        }
        Self { commands: map }
    }
}

#[async_trait]
impl ModuleHandler for HelpModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        let mut lines: Vec<String> = Vec::new();
        lines.push("🤖 可用命令:".to_string());
        lines.push(String::new());

        for (cmd, desc) in &self.commands {
            lines.push(format!("  {cmd}\r\n  　{desc}"));
        }

        lines.push(String::new());
        lines.push("发送对应命令即可使用".to_string());

        let help_text = lines.join("\r\n");
        reply(sender, msg, &help_text).await
    }

    fn name(&self) -> &str {
        "HelpModule"
    }
}
