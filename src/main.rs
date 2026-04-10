mod config;
mod core;
mod http_server;
mod modules;
mod utils;

use std::sync::Arc;

use config::AppConfig;
use core::bot::WeixinBot;
use core::router::{MatchType, RouteRule};
use modules::echo_module::EchoModule;
use modules::help_module::HelpModule;
use modules::notify_module::NotifyModule;
use modules::query_module::QueryModule;

#[tokio::main]
async fn main() {
    // 1. 加载配置
    let config = AppConfig::from_env();

    // 2. 初始化日志
    utils::logger::init_logger(&config.log_level);

    tracing::info!("微信 iLink Bot 启动中...");
    tracing::info!("状态目录: {}", config.state_dir);

    let http_port = config.http_port;

    // 3. 创建 Bot 实例并注册业务模块
    let mut bot = WeixinBot::new(config);

    bot.router.register(
        RouteRule::new("echo", "回声", MatchType::Prefix),
        Arc::new(EchoModule::new()),
    );
    bot.router.register(
        RouteRule::new("query", "查询", MatchType::Prefix),
        Arc::new(QueryModule::new()),
    );
    bot.router.register(
        RouteRule::new("notify", "通知", MatchType::Prefix),
        Arc::new(NotifyModule::new()),
    );

    // 4. 设置默认处理器（未匹配时触发）
    bot.router.set_default(Arc::new(HelpModule::new(vec![
        ("回声 <内容>", "原样返回你的消息"),
        ("查询 <关键词>", "查询信息"),
        ("通知 <用户ID> <内容>", "向指定用户发送通知"),
    ])));

    // 5. 包装为 Arc 共享
    let bot = Arc::new(bot);

    // 6. 恢复已保存的账号（自动启动长轮询）
    if let Err(e) = bot.startup().await {
        tracing::error!("启动异常: {}", e);
        return;
    }

    // 7. 启动 HTTP 管理接口
    println!("\n========================================");
    println!("  微信 iLink Bot 已启动!");
    println!("========================================");
    println!("HTTP 管理接口: http://localhost:{}", http_port);
    println!("添加新账号:    POST http://localhost:{}/account/add", http_port);
    println!("异步添加:      POST /account/qrcode → GET /account/status?qrcode=xxx");
    println!("查看状态:      GET  http://localhost:{}/status", http_port);
    println!("\n所有账号均通过 HTTP 接口按需添加");
    println!("按 Ctrl+C 停止\n");

    let bot_http = Arc::clone(&bot);
    let http_handle = tokio::spawn(async move {
        http_server::start_http_server(bot_http, http_port).await;
    });

    // 8. 等待停止信号
    tokio::signal::ctrl_c().await.ok();
    println!("\n收到停止信号，正在关闭...");
    bot.stop().await;

    // 等待一小段时间让长轮询退出
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    drop(http_handle);
    println!("Bot 已停止");
}
