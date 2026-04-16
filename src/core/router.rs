use regex::Regex;
use tracing::{debug, warn};

use crate::core::parser::ParsedMessage;
use crate::modules::base::ModuleHandler;
use std::sync::Arc;

/// 路由匹配方式
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MatchType {
    /// 前缀匹配
    Prefix,
    /// 精确匹配
    Exact,
    /// 正则匹配
    RegexMatch,
    /// 包含匹配
    Contains,
    /// 按消息类型匹配
    TypeMatch,
}

/// 路由规则
#[derive(Clone)]
pub struct RouteRule {
    /// 规则名称
    pub name: String,
    /// 匹配模式
    pub pattern: String,
    /// 匹配方式
    pub match_type: MatchType,
    /// 编译后的正则（仅 RegexMatch 时使用）
    compiled_regex: Option<Regex>,
}

impl RouteRule {
    pub fn new(name: &str, pattern: &str, match_type: MatchType) -> Self {
        let compiled_regex = match &match_type {
            MatchType::RegexMatch => Some(
                Regex::new(pattern).unwrap_or_else(|e| panic!("无效的正则表达式 '{pattern}': {e}")),
            ),
            _ => None,
        };

        Self {
            name: name.to_string(),
            pattern: pattern.to_string(),
            match_type,
            compiled_regex,
        }
    }

    /// 检查消息是否匹配此规则
    pub fn matches(&self, msg: &ParsedMessage) -> bool {
        let text = msg.text.trim();

        match &self.match_type {
            MatchType::Prefix => text.starts_with(&self.pattern),
            MatchType::Exact => text == self.pattern,
            MatchType::RegexMatch => self
                .compiled_regex
                .as_ref()
                .map(|re| re.is_match(text))
                .unwrap_or(false),
            MatchType::Contains => text.contains(&self.pattern),
            MatchType::TypeMatch => msg.message_type.as_str() == self.pattern,
        }
    }
}

/// 消息路由器
///
/// 根据注册的规则将消息分发到对应的处理模块。
/// 规则按注册顺序匹配，第一个匹配的规则生效。
#[derive(Clone)]
pub struct MessageRouter {
    routes: Vec<(RouteRule, Arc<dyn ModuleHandler>)>,
    default_handler: Option<Arc<dyn ModuleHandler>>,
}

impl MessageRouter {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            default_handler: None,
        }
    }

    /// 注册路由规则和对应的处理模块
    #[allow(dead_code)]
    pub fn register(&mut self, rule: RouteRule, handler: Arc<dyn ModuleHandler>) {
        tracing::info!(
            "路由注册: [{:?}] '{}' → {}",
            rule.match_type,
            rule.pattern,
            handler.name()
        );
        self.routes.push((rule, handler));
    }

    /// 注册一个模块 — 自动从 `routes()` 获取所有路由规则
    ///
    /// 模块只需实现 `routes()` 声明自己关心的匹配规则，
    /// 路由器会将每条规则关联到同一个模块实例（`Arc` 共享）。
    pub fn register_module(&mut self, handler: Arc<dyn ModuleHandler>) {
        let rules = handler.routes();
        if rules.is_empty() {
            tracing::warn!("模块 {} 未声明任何路由规则", handler.name());
            return;
        }
        for rule in rules {
            tracing::info!(
                "路由注册: [{:?}] '{}' → {}",
                rule.match_type,
                rule.pattern,
                handler.name()
            );
            self.routes.push((rule, Arc::clone(&handler)));
        }
    }

    /// 设置默认处理模块（无规则匹配时触发）
    pub fn set_default(&mut self, handler: Arc<dyn ModuleHandler>) {
        tracing::info!("默认路由: → {}", handler.name());
        self.default_handler = Some(handler);
    }

    /// 根据消息内容匹配路由规则，返回对应的处理模块
    pub fn route(&self, msg: &ParsedMessage) -> Option<Arc<dyn ModuleHandler>> {
        for (rule, handler) in &self.routes {
            if rule.matches(msg) {
                debug!("路由匹配: [{}] → {}", rule.name, handler.name());
                return Some(Arc::clone(handler));
            }
        }

        if let Some(ref default) = self.default_handler {
            debug!("路由默认: → {}", default.name());
            return Some(Arc::clone(default));
        }

        let text_preview: String = msg.text.chars().take(50).collect();
        warn!("无匹配路由: user={} text='{}'", msg.user_id, text_preview);
        None
    }
}
