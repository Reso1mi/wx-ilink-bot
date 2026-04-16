mod config;
mod core;
mod http_server;
mod modules;
mod utils;

use std::sync::Arc;

use config::AppConfig;
use core::bot::WeixinBot;
use core::nickname_store::NicknameStore;
use core::todo_store::TodoStore;
use core::xhs_client::XhsClientConfig;
use modules::echo_module::EchoModule;
use modules::help_module::HelpModule;
use modules::nickname_module::NicknameModule;
use modules::notify_module::NotifyModule;
use modules::query_module::QueryModule;
use modules::todo_module::TodoModule;
use modules::xhs_module::XhsModule;

#[tokio::main]
async fn main() {
    // 1. 加载配置
    let config = AppConfig::from_env();

    // 2. 初始化日志
    utils::logger::init_logger(&config.log_level);

    tracing::info!("微信 iLink Bot 启动中...");
    tracing::info!("状态目录: {}", config.state_dir);

    let http_port = config.http_port;
    let xhs_config = XhsClientConfig::new(
        config.xhs_api_url.clone(),
        config.xhs_cookie.clone(),
        config.xhs_proxy.clone(),
        config.xhs_timeout_ms,
    );

    // 创建共享存储（自动恢复持久化数据）
    let nickname_store = NicknameStore::new(&config.state_dir).await;
    let todo_store = TodoStore::new(&config.state_dir).await;

    // 3. 创建 Bot 实例并注册业务模块
    let mut bot = WeixinBot::new(config);

    bot.router.register_module(Arc::new(NicknameModule::new(
        nickname_store.clone(),
        bot.ctx_store().clone(),
    )));
    bot.router
        .register_module(Arc::new(TodoModule::new(todo_store.clone())));
    bot.router.register_module(Arc::new(EchoModule::new()));
    bot.router.register_module(Arc::new(QueryModule::new()));
    bot.router.register_module(Arc::new(NotifyModule::new()));
    bot.router.register_module(Arc::new(
        XhsModule::new(xhs_config).expect("初始化 XhsModule 失败"),
    ));

    // 4. 设置默认处理器（未匹配时触发）
    bot.router.set_default(Arc::new(HelpModule::new(vec![
        ("叫我 <昵称>", "设置你的昵称"),
        ("我是谁", "查看你的昵称"),
        ("用户列表", "查看当前所有用户"),
        ("待办 <内容>", "添加待办事项"),
        ("待办列表", "查看未完成的待办"),
        ("完成 <编号>", "标记待办为已完成"),
        ("删除待办 <编号>", "删除指定待办"),
        ("所有待办", "查看全部待办（含已完成）"),
        ("清空已完成", "批量删除已完成项"),
        ("回声 <内容>", "原样返回你的消息"),
        ("查询 <关键词>", "查询信息"),
        ("通知 <用户ID> <内容>", "向指定用户发送通知"),
        ("直接发送小红书链接", "自动提取并返回无水印图片/视频"),
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
    println!("管理后台:      http://localhost:{http_port}/admin");
    println!("HTTP API:      http://localhost:{http_port}");
    println!("添加新账号:    POST http://localhost:{http_port}/account/add");
    println!("异步添加:      POST /account/qrcode → GET /account/status?qrcode=xxx");
    println!("查看状态:      GET  http://localhost:{http_port}/status");
    println!("\n所有账号均通过 HTTP 接口按需添加");
    println!("按 Ctrl+C 停止\n");

    let bot_http = Arc::clone(&bot);
    let http_handle = tokio::spawn(async move {
        let state = http_server::AppState {
            bot: bot_http,
            nickname_store: nickname_store.clone(),
        };
        http_server::start_http_server(state, http_port).await;
    });

    // 8. 等待停止信号
    tokio::signal::ctrl_c().await.ok();
    println!("\n收到停止信号, 正在关闭...");
    bot.stop().await;

    // 等待一小段时间让长轮询退出
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    drop(http_handle);
    println!("Bot 已停止");
}
