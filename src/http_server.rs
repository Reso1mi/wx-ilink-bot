use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use crate::core::bot::WeixinBot;

/// 共享状态
pub type SharedBot = Arc<WeixinBot>;

/// 创建 HTTP 路由
pub fn create_router(bot: SharedBot) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/users", get(users_handler))
        .route("/accounts", get(accounts_handler))
        // 添加新账号（每次扫码创建独立的 Bot 会话）
        .route("/account/add", post(add_account_handler))
        .route("/account/qrcode", post(account_qrcode_handler))
        .route("/account/status", get(account_status_handler))
        // 消息发送（跨账号自动调度）
        .route("/message/send", post(send_message_handler))
        .with_state(bot)
}

/// 根路径 — 返回可用接口列表
async fn index_handler() -> Json<Value> {
    Json(json!({
        "service": "微信 iLink Bot",
        "endpoints": {
            "GET /health": "健康检查",
            "GET /status": "Bot 状态",
            "GET /accounts": "所有账号列表",
            "GET /users": "所有已连接用户",
            "POST /account/add": "同步添加账号（阻塞等待扫码）",
            "POST /account/qrcode": "异步添加账号 Step 1（获取二维码）",
            "GET /account/status?qrcode=xxx": "异步添加账号 Step 2（轮询状态）",
            "POST /message/send": "发送消息给指定用户（跨账号自动调度）",
        }
    }))
}

/// 健康检查
async fn health_handler() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Bot 状态
async fn status_handler(State(bot): State<SharedBot>) -> Json<Value> {
    let online = bot.is_online().await;
    let accounts = bot.get_accounts_info().await;

    Json(json!({
        "online": online,
        "account_count": accounts.len(),
        "accounts": accounts.iter().map(|a| json!({
            "account_id": a.account_id,
            "user_id": a.user_id,
        })).collect::<Vec<_>>(),
    }))
}

/// 所有账号列表
async fn accounts_handler(State(bot): State<SharedBot>) -> Json<Value> {
    let accounts = bot.get_accounts_info().await;
    let list: Vec<Value> = accounts
        .iter()
        .map(|a| {
            json!({
                "account_id": a.account_id,
                "user_id": a.user_id,
            })
        })
        .collect();

    Json(json!({
        "count": list.len(),
        "accounts": list,
    }))
}

/// 所有已连接用户（按账号分组）
async fn users_handler(State(bot): State<SharedBot>) -> Json<Value> {
    let users_by_account = bot.get_connected_users().await;
    let mut total = 0;

    let accounts: Vec<Value> = users_by_account
        .iter()
        .map(|(account_id, users)| {
            total += users.len();
            json!({
                "account_id": account_id,
                "users": users.iter().map(|u| json!({
                    "user_id": u.user_id,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Json(json!({
        "total_users": total,
        "accounts": accounts,
    }))
}

/// 同步添加账号：生成二维码并阻塞等待扫码完成
///
/// `POST /account/add`
async fn add_account_handler(State(bot): State<SharedBot>) -> Json<Value> {
    match bot.add_account().await {
        Ok(result) => {
            if result.success {
                Json(json!({
                    "success": true,
                    "account_id": result.account_id,
                    "user_id": result.user_id,
                }))
            } else {
                Json(json!({
                    "success": false,
                    "error": result.error,
                }))
            }
        }
        Err(e) => Json(json!({
            "success": false,
            "error": format!("添加账号异常: {}", e),
        })),
    }
}

/// 异步添加账号 Step 1：生成二维码
///
/// `POST /account/qrcode`
async fn account_qrcode_handler(State(bot): State<SharedBot>) -> Json<Value> {
    match bot.create_account_qrcode().await {
        Ok(qr) => Json(json!({
            "success": true,
            "qrcode": qr.qrcode,
            "qrcode_img_url": qr.qrcode_img_url,
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": format!("生成二维码失败: {}", e),
        })),
    }
}

/// 轮询状态的查询参数
#[derive(Deserialize)]
struct AccountStatusQuery {
    qrcode: String,
}

/// 异步添加账号 Step 2：轮询扫码状态
///
/// `GET /account/status?qrcode=xxx`
async fn account_status_handler(
    State(bot): State<SharedBot>,
    Query(query): Query<AccountStatusQuery>,
) -> Json<Value> {
    match bot.poll_account_status(&query.qrcode).await {
        Ok(result) => Json(json!({
            "status": result.status,
            "account_id": result.account_id,
            "user_id": result.user_id,
        })),
        Err(e) => Json(json!({
            "status": "error",
            "error": format!("查询状态失败: {}", e),
        })),
    }
}

/// 启动 HTTP 服务
pub async fn start_http_server(bot: SharedBot, port: u16) {
    let app = create_router(bot);
    let addr = format!("0.0.0.0:{}", port);

    info!("HTTP 管理接口启动: http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect(&format!("无法绑定端口 {}", port));

    axum::serve(listener, app)
        .await
        .expect("HTTP 服务异常退出");
}

/// 发送消息请求体
#[derive(Deserialize)]
struct SendMessageBody {
    /// 目标用户 ID（xxx@im.wechat）
    to_user_id: String,
    /// 消息内容
    text: String,
}

/// 发送消息给指定用户（跨账号自动调度）
///
/// `POST /message/send`
///
/// 请求体:
/// ```json
/// { "to_user_id": "xxx@im.wechat", "text": "你好" }
/// ```
///
/// 自动查找该用户对应的 Bot 账号并发送消息。
async fn send_message_handler(
    State(bot): State<SharedBot>,
    Json(body): Json<SendMessageBody>,
) -> Json<Value> {
    if body.to_user_id.is_empty() || body.text.is_empty() {
        return Json(json!({
            "success": false,
            "error": "to_user_id 和 text 不能为空",
        }));
    }

    match bot.send_message(&body.to_user_id, &body.text).await {
        Ok(_) => Json(json!({
            "success": true,
            "to_user_id": body.to_user_id,
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": format!("{}", e),
        })),
    }
}
