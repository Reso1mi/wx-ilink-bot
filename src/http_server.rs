use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::{
    extract::{Multipart, Query, State},
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

use crate::core::bot::WeixinBot;
use crate::core::nickname_store::NicknameStore;

/// 共享状态
#[derive(Clone)]
pub struct AppState {
    pub bot: Arc<WeixinBot>,
    pub nickname_store: NicknameStore,
}

/// 创建 HTTP 路由
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/admin", get(admin_handler))
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
        .route("/message/send-file", post(send_file_handler))
        // 图片代理（解决 HTTPS 二维码在 localhost 上无法加载的问题）
        .route("/proxy/image", get(proxy_image_handler))
        // 二维码图片生成（后端渲染，不依赖前端 JS 库或外部 CDN）
        .route("/qrcode/image", get(qrcode_image_handler))
        .with_state(state)
}

/// 根路径 — 返回可用接口列表
async fn index_handler() -> Json<Value> {
    Json(json!({
        "service": "微信 iLink Bot",
        "endpoints": {
            "GET /admin": "管理后台页面",
            "GET /health": "健康检查",
            "GET /status": "Bot 状态",
            "GET /accounts": "所有账号列表",
            "GET /users": "所有已连接用户",
            "POST /account/add": "同步添加账号（阻塞等待扫码）",
            "POST /account/qrcode": "异步添加账号 Step 1（获取二维码）",
            "GET /account/status?qrcode=xxx": "异步添加账号 Step 2（轮询状态）",
            "POST /message/send": "发送文本消息给指定用户（跨账号自动调度）",
            "POST /message/send-file": "上传并发送文件/图片/视频（multipart/form-data）",
        }
    }))
}

/// 管理后台页面
async fn admin_handler() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

/// 健康检查
async fn health_handler() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Bot 状态
async fn status_handler(State(state): State<AppState>) -> Json<Value> {
    let bot = &state.bot;
    let ns = &state.nickname_store;
    let online = bot.is_online().await;
    let accounts = bot.get_accounts_info().await;

    let mut account_list = Vec::new();
    for a in &accounts {
        let display = ns.display_name(&a.user_id).await;
        account_list.push(json!({
            "nickname": display,
            "user_id": a.user_id,
        }));
    }

    Json(json!({
        "online": online,
        "account_count": accounts.len(),
        "accounts": account_list,
    }))
}

/// 所有账号列表
async fn accounts_handler(State(state): State<AppState>) -> Json<Value> {
    let bot = &state.bot;
    let ns = &state.nickname_store;
    let accounts = bot.get_accounts_info().await;

    let mut list = Vec::new();
    for a in &accounts {
        let display = ns.display_name(&a.user_id).await;
        list.push(json!({
            "nickname": display,
            "user_id": a.user_id,
        }));
    }

    Json(json!({
        "count": list.len(),
        "accounts": list,
    }))
}

/// 所有已连接用户（按账号分组）
async fn users_handler(State(state): State<AppState>) -> Json<Value> {
    let bot = &state.bot;
    let ns = &state.nickname_store;
    let users_by_account = bot.get_connected_users().await;
    let mut total = 0;

    let mut accounts = Vec::new();
    for (account_id, users) in &users_by_account {
        total += users.len();
        let mut user_list = Vec::new();
        for u in users {
            let display = ns.display_name(&u.user_id).await;
            user_list.push(json!({
                "nickname": display,
                "user_id": u.user_id,
            }));
        }
        accounts.push(json!({
            "account_id": account_id,
            "users": user_list,
        }));
    }

    Json(json!({
        "total_users": total,
        "accounts": accounts,
    }))
}

/// 同步添加账号：生成二维码并阻塞等待扫码完成
///
/// `POST /account/add`
async fn add_account_handler(State(state): State<AppState>) -> Json<Value> {
    match state.bot.add_account().await {
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
            "error": format!("添加账号异常: {e}"),
        })),
    }
}

/// 异步添加账号 Step 1：生成二维码
///
/// `POST /account/qrcode`
async fn account_qrcode_handler(State(state): State<AppState>) -> Json<Value> {
    match state.bot.create_account_qrcode().await {
        Ok(qr) => Json(json!({
            "success": true,
            "qrcode": qr.qrcode,
            "qrcode_img_url": qr.qrcode_img_url,
            "qrcode_content": qr.qrcode_content,
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": format!("生成二维码失败: {e}"),
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
    State(state): State<AppState>,
    Query(query): Query<AccountStatusQuery>,
) -> Json<Value> {
    match state.bot.poll_account_status(&query.qrcode).await {
        Ok(result) => Json(json!({
            "status": result.status,
            "account_id": result.account_id,
            "user_id": result.user_id,
        })),
        Err(e) => Json(json!({
            "status": "error",
            "error": format!("查询状态失败: {e}"),
        })),
    }
}

/// 启动 HTTP 服务
pub async fn start_http_server(state: AppState, port: u16) {
    let app = create_router(state);
    let addr = format!("0.0.0.0:{port}");

    info!("HTTP 管理接口启动: http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("无法绑定端口 {port}"));

    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}

/// 发送消息请求体
#[derive(Deserialize)]
struct SendMessageBody {
    /// 目标用户 ID（xxx@im.wechat）
    to_user_id: String,
    /// 消息内容（文本消息必填）
    text: String,
}

/// 发送文本消息给指定用户（跨账号自动调度）
///
/// `POST /message/send`
///
/// 请求体:
/// ```json
/// { "to_user_id": "xxx@im.wechat", "text": "你好" }
/// ```
async fn send_message_handler(
    State(state): State<AppState>,
    Json(body): Json<SendMessageBody>,
) -> Json<Value> {
    if body.to_user_id.is_empty() || body.text.is_empty() {
        return Json(json!({
            "success": false,
            "error": "to_user_id 和 text 不能为空",
        }));
    }

    // 支持通过昵称定位用户
    let to_user_id = match state.nickname_store.find_by_nickname(&body.to_user_id).await {
        Some(uid) => uid,
        None => body.to_user_id.clone(),
    };

    match state.bot.send_message(&to_user_id, &body.text).await {
        Ok(_) => Json(json!({
            "success": true,
            "type": "text",
            "to_user_id": body.to_user_id,
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": format!("{e}"),
        })),
    }
}

/// 上传并发送文件/图片/视频（multipart/form-data）
///
/// `POST /message/send-file`
///
/// Form 字段:
/// - `to_user_id`: 目标用户 ID（必填）
/// - `type`: 消息类型，可选 "image" / "file" / "video"（默认根据 content_type 推断）
/// - `file`: 文件内容（必填）
/// - `play_length`: 视频时长秒数（仅 video 类型）
async fn send_file_handler(State(state): State<AppState>, mut multipart: Multipart) -> Json<Value> {
    let mut to_user_id = String::new();
    let mut msg_type = String::new();
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name = String::from("upload");
    let mut content_type = String::from("application/octet-stream");
    let mut play_length: i64 = 0;

    // 解析 multipart 字段
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "to_user_id" => {
                to_user_id = field.text().await.unwrap_or_default();
            }
            "type" => {
                msg_type = field.text().await.unwrap_or_default();
            }
            "play_length" => {
                play_length = field.text().await.unwrap_or_default().parse().unwrap_or(0);
            }
            "file" => {
                if let Some(fname) = field.file_name() {
                    file_name = fname.to_string();
                }
                if let Some(ct) = field.content_type() {
                    content_type = ct.to_string();
                }
                match field.bytes().await {
                    Ok(bytes) => file_data = Some(bytes.to_vec()),
                    Err(e) => {
                        return Json(json!({
                            "success": false,
                            "error": format!("读取文件失败: {e}"),
                        }));
                    }
                }
            }
            _ => {
                warn!("未知的 multipart 字段: {}", name);
            }
        }
    }

    // 校验必填字段
    if to_user_id.is_empty() {
        return Json(json!({
            "success": false,
            "error": "to_user_id 不能为空",
        }));
    }

    let data = match file_data {
        Some(d) => d,
        None => {
            return Json(json!({
                "success": false,
                "error": "缺少 file 字段",
            }));
        }
    };

    // 自动推断消息类型
    if msg_type.is_empty() {
        msg_type = infer_message_type(&content_type);
    }

    // 支持通过昵称定位用户
    let resolved_uid = match state.nickname_store.find_by_nickname(&to_user_id).await {
        Some(uid) => uid,
        None => to_user_id.clone(),
    };

    info!(
        "收到文件发送请求: to={} type={} file={} size={} content_type={}",
        resolved_uid,
        msg_type,
        file_name,
        data.len(),
        content_type
    );

    let result = match msg_type.as_str() {
        "image" => {
            state.bot.send_image(&resolved_uid, &data, &file_name, &content_type)
                .await
        }
        "video" => {
            state.bot.send_video(&resolved_uid, &data, &file_name, &content_type, play_length)
                .await
        }
        _ => {
            // 默认作为文件发送
            state.bot.send_file(&resolved_uid, &data, &file_name, &content_type)
                .await
        }
    };

    match result {
        Ok(_) => Json(json!({
            "success": true,
            "type": msg_type,
            "to_user_id": to_user_id,
            "file_name": file_name,
            "file_size": data.len(),
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": format!("{e}"),
        })),
    }
}

/// 根据 Content-Type 推断消息类型
fn infer_message_type(content_type: &str) -> String {
    if content_type.starts_with("image/") {
        "image".to_string()
    } else if content_type.starts_with("video/") {
        "video".to_string()
    } else {
        "file".to_string()
    }
}

/// 图片代理查询参数
#[derive(Deserialize)]
struct ProxyImageQuery {
    url: String,
}

/// 图片代理 — 解决 HTTPS 二维码图片在 localhost 上无法加载的问题
///
/// `GET /proxy/image?url=https://...`
///
/// 后端抓取远程 HTTPS 图片并以原始 Content-Type 返回给浏览器，
/// 避免了混合内容安全策略（Mixed Content）和跨域加载限制。
async fn proxy_image_handler(
    Query(query): Query<ProxyImageQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let url = &query.url;

    // 安全校验：只允许代理 HTTPS URL
    if !url.starts_with("https://") {
        return Err((StatusCode::BAD_REQUEST, "仅支持代理 HTTPS URL".to_string()));
    }

    // 限制域名为 iLink 相关（防止 SSRF）
    let is_allowed =
        url.contains("weixin.qq.com") || url.contains("wechat.com") || url.contains("qq.com");
    if !is_allowed {
        return Err((StatusCode::FORBIDDEN, "不允许代理该域名".to_string()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建 HTTP 客户端失败: {e}"),
            )
        })?;

    let resp = client.get(url).send().await.map_err(|e| {
        warn!("代理图片请求失败: {} - {}", url, e);
        (StatusCode::BAD_GATEWAY, format!("请求远程图片失败: {e}"))
    })?;

    if !resp.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("远程服务器返回 {}", resp.status()),
        ));
    }

    // 获取 Content-Type
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png")
        .to_string();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("读取图片内容失败: {e}")))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(bytes.to_vec()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("构建响应失败: {e}"),
            )
        })
}

/// 二维码图片查询参数
#[derive(Deserialize)]
struct QRCodeImageQuery {
    /// 二维码内容（qrcode 标识符）
    data: String,
}

/// 二维码图片生成 — 后端直接将字符串渲染成 QR Code PNG 图片
///
/// `GET /qrcode/image?data=xxx`
///
/// 不依赖前端 JS 库或外部 CDN，由后端 Rust 直接生成。
async fn qrcode_image_handler(
    Query(query): Query<QRCodeImageQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let data = &query.data;

    if data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "data 参数不能为空".to_string()));
    }

    // 生成 QR 码
    let qr = qrcode::QrCode::new(data.as_bytes()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("生成二维码失败: {e}"),
        )
    })?;

    // 渲染为图片 (每个模块 10px, 安静区 2 模块)
    let image = qr.render::<image::Luma<u8>>().quiet_zone(true).build();

    // 编码为 PNG
    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    image::ImageEncoder::write_image(
        encoder,
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::L8,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("编码 PNG 失败: {e}"),
        )
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(png_bytes))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("构建响应失败: {e}"),
            )
        })
}
