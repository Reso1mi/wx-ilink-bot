use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::AppConfig;
use crate::core::api::ILinkAPI;
use crate::core::auth::{QRCodeManager, ScanResult};
use crate::core::parser::MessageParser;
use crate::core::router::MessageRouter;
use crate::core::session::ContextTokenStore;
use crate::modules::base::MessageSender;

/// 会话过期错误码
const SESSION_EXPIRED_ERRCODE: i64 = -14;
/// 连续失败最大次数
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// 退避延迟（秒）
const BACKOFF_DELAY_S: u64 = 30;
/// 重试延迟（秒）
const RETRY_DELAY_S: u64 = 2;
/// 会话过期暂停时间（秒）
const SESSION_PAUSE_DURATION_S: u64 = 3600;

/// 单个账号（每次扫码产生一个独立的账号）
///
/// iLink 协议中，每次扫码会创建一个新的 Bot 会话，
/// 拥有独立的 `bot_token`、`account_id`，以及独立的 `getupdates` 消息流。
struct Account {
    /// 账号 ID（xxx@im.bot）
    account_id: String,
    /// 扫码用户 ID（xxx@im.wechat）
    user_id: String,
    /// 该账号的 API 客户端
    api: ILinkAPI,
    /// 同步游标
    sync_buf: Arc<RwLock<String>>,
}

/// 微信 iLink Bot 主类
///
/// 管理多个账号。iLink 协议中每次扫码创建一个新的 Bot 会话，
/// 每个会话有独立的 `bot_token`，需要各自运行 `getupdates` 长轮询。
///
/// 工作流程：
/// 1. 首次扫码 → 创建第一个 Account → 启动长轮询
/// 2. 通过 HTTP `POST /bind` 扫码 → 创建新 Account → 自动启动独立长轮询
/// 3. 所有 Account 的消息统一路由到同一个 MessageRouter
pub struct WeixinBot {
    /// 消息路由器（所有账号共享）
    pub router: MessageRouter,
    /// Context Token 存储（所有账号共享）
    ctx_store: ContextTokenStore,
    /// 所有已注册的账号 (account_id → Account)
    accounts: Arc<RwLock<HashMap<String, Arc<Account>>>>,
    /// 运行状态
    running: Arc<RwLock<bool>>,
    /// 应用配置
    config: AppConfig,
}

/// 全局消息发送器 — 支持跨账号调度
///
/// 当给指定用户发消息时，自动查找该用户对应的账号（通过 context_token 关联），
/// 使用该账号的 API 发送消息。
///
/// 查找策略：
/// 1. 优先使用 `primary_account_id`（消息来源账号）
/// 2. 若该账号下没有目标用户的 context_token，遍历所有账号查找
/// 3. 都找不到则报错
pub struct BotSender {
    /// 所有账号表
    accounts: Arc<RwLock<HashMap<String, Arc<Account>>>>,
    /// 共享的 context_token 存储
    ctx_store: ContextTokenStore,
    /// 优先使用的账号（消息来源账号）
    primary_account_id: String,
}

/// 查找结果：匹配的账号 + context_token
struct ResolvedAccount {
    account: Arc<Account>,
    account_id: String,
    context_token: String,
}

impl BotSender {
    /// 查找目标用户对应的账号（跨账号调度核心）
    async fn resolve_account(&self, to_user_id: &str) -> Result<ResolvedAccount> {
        // 1. 优先尝试 primary account
        if !self.primary_account_id.is_empty() {
            if let Some(token) = self
                .ctx_store
                .get(&self.primary_account_id, to_user_id)
                .await
            {
                let accounts = self.accounts.read().await;
                if let Some(account) = accounts.get(&self.primary_account_id) {
                    return Ok(ResolvedAccount {
                        account: Arc::clone(account),
                        account_id: self.primary_account_id.clone(),
                        context_token: token,
                    });
                }
            }
        }

        // 2. 遍历所有账号查找
        let accounts = self.accounts.read().await;
        for (account_id, account) in accounts.iter() {
            if account_id == &self.primary_account_id {
                continue;
            }
            if let Some(token) = self.ctx_store.get(account_id, to_user_id).await {
                return Ok(ResolvedAccount {
                    account: Arc::clone(account),
                    account_id: account_id.clone(),
                    context_token: token,
                });
            }
        }

        anyhow::bail!(
            "无法发送消息给 {}: 所有账号中均未找到该用户的 context_token，该用户可能从未给 Bot 发过消息",
            to_user_id
        )
    }
}

#[async_trait]
impl MessageSender for BotSender {
    async fn send_text(&self, to_user_id: &str, text: &str) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        resolved
            .account
            .api
            .send_message(to_user_id, text, &resolved.context_token, None)
            .await?;
        info!(
            "文本消息已发送: account={} to={} len={}",
            resolved.account_id,
            to_user_id,
            text.len()
        );
        Ok(())
    }

    async fn send_image(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        aes_key: &str,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        resolved
            .account
            .api
            .send_image(
                to_user_id,
                &resolved.context_token,
                file_id,
                download_url,
                aes_key,
            )
            .await?;
        info!(
            "图片消息已发送: account={} to={} file_id={}",
            resolved.account_id, to_user_id, file_id
        );
        Ok(())
    }

    async fn send_file(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        file_name: &str,
        file_size: i64,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        resolved
            .account
            .api
            .send_file(
                to_user_id,
                &resolved.context_token,
                file_id,
                download_url,
                file_name,
                file_size,
            )
            .await?;
        info!(
            "文件消息已发送: account={} to={} file={}",
            resolved.account_id, to_user_id, file_name
        );
        Ok(())
    }

    async fn send_video(
        &self,
        to_user_id: &str,
        file_id: &str,
        download_url: &str,
        video_size: i64,
        play_length: i64,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        resolved
            .account
            .api
            .send_video(
                to_user_id,
                &resolved.context_token,
                file_id,
                download_url,
                video_size,
                play_length,
            )
            .await?;
        info!(
            "视频消息已发送: account={} to={} size={} length={}s",
            resolved.account_id, to_user_id, video_size, play_length
        );
        Ok(())
    }

    async fn upload_and_send_image(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        let (file_id, download_url) = resolved
            .account
            .api
            .upload_media(data, file_name, 1, to_user_id, content_type)
            .await?;
        resolved
            .account
            .api
            .send_image(
                to_user_id,
                &resolved.context_token,
                &file_id,
                &download_url,
                "",
            )
            .await?;
        info!(
            "图片上传并发送: account={} to={} file={}",
            resolved.account_id, to_user_id, file_name
        );
        Ok(())
    }

    async fn upload_and_send_file(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        let file_size = data.len() as i64;
        let (file_id, download_url) = resolved
            .account
            .api
            .upload_media(data, file_name, 4, to_user_id, content_type)
            .await?;
        resolved
            .account
            .api
            .send_file(
                to_user_id,
                &resolved.context_token,
                &file_id,
                &download_url,
                file_name,
                file_size,
            )
            .await?;
        info!(
            "文件上传并发送: account={} to={} file={} size={}",
            resolved.account_id, to_user_id, file_name, file_size
        );
        Ok(())
    }

    async fn upload_and_send_video(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
        play_length: i64,
    ) -> Result<()> {
        let resolved = self.resolve_account(to_user_id).await?;
        let video_size = data.len() as i64;
        let (file_id, download_url) = resolved
            .account
            .api
            .upload_media(data, file_name, 3, to_user_id, content_type)
            .await?;
        resolved
            .account
            .api
            .send_video(
                to_user_id,
                &resolved.context_token,
                &file_id,
                &download_url,
                video_size,
                play_length,
            )
            .await?;
        info!(
            "视频上传并发送: account={} to={} file={} size={} length={}s",
            resolved.account_id, to_user_id, file_name, video_size, play_length
        );
        Ok(())
    }
}

impl WeixinBot {
    /// 创建新的 Bot 实例
    pub fn new(config: AppConfig) -> Self {
        let ctx_store = ContextTokenStore::new(&config.state_dir);

        Self {
            router: MessageRouter::new(),
            ctx_store,
            accounts: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(true)),
            config,
        }
    }

    // ==================== 启动与恢复 ====================

    /// 启动 Bot — 自动恢复之前保存的账号
    ///
    /// 扫描 state 目录中的凭证文件，恢复所有已保存的账号并启动长轮询。
    /// 如果没有已保存的账号，Bot 进入待命状态，等待通过 HTTP 接口添加。
    pub async fn startup(&self) -> Result<()> {
        let restored = self.restore_accounts().await;
        if restored > 0 {
            info!("已恢复 {} 个账号，启动长轮询", restored);
        } else {
            info!("无已保存的账号，等待通过 HTTP 接口添加新账号");
        }
        Ok(())
    }

    /// 从磁盘恢复已保存的账号并启动长轮询
    async fn restore_accounts(&self) -> usize {
        let state_dir = &self.config.state_dir;
        let mut restored = 0;

        // 扫描 state 目录中的 {account_id}.json 凭证文件
        let entries = match std::fs::read_dir(state_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("无法读取状态目录 {}: {}", state_dir, e);
                return 0;
            }
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            // 只匹配 xxx@im.bot.json 格式的凭证文件（跳过 .sync.json 和 .context-tokens.json）
            if !name.ends_with(".json")
                || name.ends_with(".sync.json")
                || name.ends_with(".context-tokens.json")
            {
                continue;
            }

            let account_id = name.trim_end_matches(".json");

            if let Some((bot_token, base_url, user_id)) =
                self.ctx_store.restore_credentials(account_id).await
            {
                info!("恢复账号: account_id={} user_id={}", account_id, user_id);

                let scan_result = ScanResult {
                    success: true,
                    bot_token,
                    account_id: account_id.to_string(),
                    base_url,
                    user_id,
                    error: None,
                };

                self.register_account(&scan_result).await;
                self.spawn_account_poller(account_id).await;
                restored += 1;
            }
        }

        restored
    }

    /// 注册一个新账号（扫码成功后调用）
    async fn register_account(&self, result: &ScanResult) {
        let api = ILinkAPI::new(
            Some(&result.base_url),
            Some(&result.bot_token),
            &self.config.app_id,
            &self.config.version,
        );

        // 恢复同步游标
        let sync_buf = self.ctx_store.restore_sync_buf(&result.account_id).await;

        let account = Arc::new(Account {
            account_id: result.account_id.clone(),
            user_id: result.user_id.clone(),
            api,
            sync_buf: Arc::new(RwLock::new(sync_buf)),
        });

        // 恢复 context_token
        self.ctx_store.restore(&result.account_id).await;

        // 保存凭证
        self.ctx_store
            .save_credentials(
                &result.account_id,
                &result.bot_token,
                &result.base_url,
                &result.user_id,
            )
            .await;

        // 注册到账号表
        {
            let mut accounts = self.accounts.write().await;
            accounts.insert(result.account_id.clone(), Arc::clone(&account));
        }

        info!(
            "账号已注册: account_id={} user_id={}",
            result.account_id, result.user_id
        );
    }

    // ==================== 消息发送 ====================

    /// 创建一个全局发送器（跨账号自动调度）
    fn create_sender(&self) -> BotSender {
        BotSender {
            accounts: Arc::clone(&self.accounts),
            ctx_store: self.ctx_store.clone(),
            primary_account_id: String::new(),
        }
    }

    /// 向指定用户发送文本消息（跨账号自动调度）
    pub async fn send_message(&self, to_user_id: &str, text: &str) -> Result<()> {
        self.create_sender().send_text(to_user_id, text).await
    }

    /// 向指定用户发送图片（上传+发送，跨账号自动调度）
    pub async fn send_image(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()> {
        self.create_sender()
            .upload_and_send_image(to_user_id, data, file_name, content_type)
            .await
    }

    /// 向指定用户发送文件（上传+发送，跨账号自动调度）
    pub async fn send_file(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<()> {
        self.create_sender()
            .upload_and_send_file(to_user_id, data, file_name, content_type)
            .await
    }

    /// 向指定用户发送视频（上传+发送，跨账号自动调度）
    pub async fn send_video(
        &self,
        to_user_id: &str,
        data: &[u8],
        file_name: &str,
        content_type: &str,
        play_length: i64,
    ) -> Result<()> {
        self.create_sender()
            .upload_and_send_video(to_user_id, data, file_name, content_type, play_length)
            .await
    }

    // ==================== 状态查询 ====================

    /// 获取 Bot 是否在线（至少有一个账号）
    pub async fn is_online(&self) -> bool {
        let accounts = self.accounts.read().await;
        !accounts.is_empty()
    }

    /// 获取所有账号信息
    pub async fn get_accounts_info(&self) -> Vec<AccountInfo> {
        let accounts = self.accounts.read().await;
        accounts
            .values()
            .map(|acc| AccountInfo {
                account_id: acc.account_id.clone(),
                user_id: acc.user_id.clone(),
            })
            .collect()
    }

    /// 获取所有已连接用户（跨所有账号）
    pub async fn get_connected_users(&self) -> HashMap<String, Vec<UserInfo>> {
        let accounts = self.accounts.read().await;
        let mut result = HashMap::new();

        for (account_id, _) in accounts.iter() {
            let users = self.ctx_store.get_all_users(account_id).await;
            let user_list: Vec<UserInfo> = users
                .keys()
                .map(|uid| UserInfo {
                    user_id: uid.clone(),
                    account_id: account_id.clone(),
                })
                .collect();
            if !user_list.is_empty() {
                result.insert(account_id.clone(), user_list);
            }
        }

        result
    }

    // ==================== 添加新账号（扫码绑定） ====================

    /// 按需添加新账号（同步：阻塞等待扫码完成）
    ///
    /// 每次调用会生成一个新的二维码，用户扫码后创建一个新的 Bot 账号，
    /// 并自动启动该账号的长轮询。
    pub async fn add_account(&self) -> Result<AddAccountResult> {
        let api = ILinkAPI::new(
            Some(&self.config.base_url),
            None,
            &self.config.app_id,
            &self.config.version,
        );

        let qr_mgr = QRCodeManager::new(api);

        info!("收到添加账号请求，生成二维码等待扫码...");

        let scan_result = qr_mgr.wait_for_scan().await?;

        if scan_result.success {
            // 注册新账号
            self.register_account(&scan_result).await;

            // 启动该账号的长轮询
            self.spawn_account_poller(&scan_result.account_id).await;

            info!(
                "新账号添加成功: account_id={} user_id={}",
                scan_result.account_id, scan_result.user_id
            );

            Ok(AddAccountResult {
                success: true,
                account_id: Some(scan_result.account_id),
                user_id: Some(scan_result.user_id),
                error: None,
            })
        } else {
            warn!("扫码未成功: {:?}", scan_result.error);
            Ok(AddAccountResult {
                success: false,
                account_id: None,
                user_id: None,
                error: scan_result.error,
            })
        }
    }

    /// 按需添加新账号 Step 1：仅生成二维码
    pub async fn create_account_qrcode(&self) -> Result<BindQRCode> {
        let api = ILinkAPI::new(
            Some(&self.config.base_url),
            None,
            &self.config.app_id,
            &self.config.version,
        );

        let qr_data = api.get_bot_qrcode("3").await?;
        let qrcode = qr_data.qrcode.unwrap_or_default();
        let qrcode_img_url = qr_data.qrcode_img_content.unwrap_or_default();

        info!("账号绑定二维码已生成: {}", qrcode_img_url);

        Ok(BindQRCode {
            qrcode,
            qrcode_img_url,
        })
    }

    /// 按需添加新账号 Step 2：轮询扫码状态
    pub async fn poll_account_status(&self, qrcode: &str) -> Result<BindPollResult> {
        let api = ILinkAPI::new(
            Some(&self.config.base_url),
            None,
            &self.config.app_id,
            &self.config.version,
        );

        let status_resp = api.get_qrcode_status(&api.base_url, qrcode).await?;
        let status = status_resp
            .status
            .clone()
            .unwrap_or_else(|| "wait".to_string());

        let mut result = BindPollResult {
            status: status.clone(),
            account_id: None,
            user_id: None,
        };

        if status == "confirmed" {
            let user_id = status_resp.ilink_user_id.unwrap_or_default();
            let account_id = status_resp.ilink_bot_id.unwrap_or_default();
            let bot_token = status_resp.bot_token.unwrap_or_default();
            let base_url = status_resp
                .baseurl
                .unwrap_or_else(|| self.config.base_url.clone());

            info!("扫码确认: account_id={} user_id={}", account_id, user_id);

            // 注册新账号并启动长轮询
            let scan_result = ScanResult {
                success: true,
                bot_token,
                account_id: account_id.clone(),
                base_url,
                user_id: user_id.clone(),
                error: None,
            };
            self.register_account(&scan_result).await;
            self.spawn_account_poller(&account_id).await;

            result.account_id = Some(account_id);
            result.user_id = Some(user_id);
        }

        Ok(result)
    }

    // ==================== 长轮询 ====================

    /// 为指定账号启动长轮询（在独立 tokio 任务中运行）
    async fn spawn_account_poller(&self, account_id: &str) {
        let account = {
            let accounts = self.accounts.read().await;
            match accounts.get(account_id) {
                Some(acc) => Arc::clone(acc),
                None => {
                    error!("无法启动长轮询: 账号 {} 不存在", account_id);
                    return;
                }
            }
        };

        let running = Arc::clone(&self.running);
        let ctx_store = self.ctx_store.clone();
        let parser = MessageParser::new();
        let router = self.router.clone();
        let accounts_ref = Arc::clone(&self.accounts);

        let account_id_owned = account_id.to_string();

        tokio::spawn(async move {
            info!("长轮询启动: account_id={}", account_id_owned);

            let mut consecutive_failures: u32 = 0;

            loop {
                // 检查运行状态
                {
                    let r = running.read().await;
                    if !*r {
                        break;
                    }
                }

                let current_buf = {
                    let buf = account.sync_buf.read().await;
                    buf.clone()
                };

                match account.api.get_updates(&current_buf, None).await {
                    Ok(resp) => {
                        let ret = resp.ret;
                        let errcode = resp.errcode;

                        if ret != 0 || errcode != 0 {
                            if errcode == SESSION_EXPIRED_ERRCODE || ret == SESSION_EXPIRED_ERRCODE
                            {
                                error!(
                                    "[{}] 会话过期，暂停 {}s",
                                    account_id_owned, SESSION_PAUSE_DURATION_S
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(
                                    SESSION_PAUSE_DURATION_S,
                                ))
                                .await;
                                continue;
                            }

                            consecutive_failures += 1;
                            error!(
                                "[{}] getupdates 失败: ret={} errcode={} ({}/{})",
                                account_id_owned,
                                ret,
                                errcode,
                                consecutive_failures,
                                MAX_CONSECUTIVE_FAILURES
                            );

                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                consecutive_failures = 0;
                                tokio::time::sleep(std::time::Duration::from_secs(BACKOFF_DELAY_S))
                                    .await;
                            } else {
                                tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_S))
                                    .await;
                            }
                            continue;
                        }

                        consecutive_failures = 0;

                        // 更新同步游标
                        let new_buf = &resp.get_updates_buf;
                        if !new_buf.is_empty() {
                            let mut buf = account.sync_buf.write().await;
                            *buf = new_buf.clone();
                            ctx_store.save_sync_buf(&account_id_owned, new_buf).await;
                        }

                        // 处理消息
                        for raw_msg in &resp.msgs {
                            let parsed = parser.parse(raw_msg);

                            let text_preview: String = parsed.text.chars().take(50).collect();
                            info!(
                                "[{}] 收到消息: user={} type={} text='{}'",
                                account_id_owned,
                                parsed.user_id,
                                parsed.message_type.as_str(),
                                text_preview
                            );

                            // 忽略 Bot 自己发送的消息
                            if let Some(msg_type) = raw_msg["message_type"].as_i64() {
                                if msg_type == 2 {
                                    continue;
                                }
                            }

                            // 存储 context_token
                            if !parsed.context_token.is_empty() {
                                ctx_store
                                    .set(&account_id_owned, &parsed.user_id, &parsed.context_token)
                                    .await;
                            }

                            // 路由到业务模块
                            let handler = router.route(&parsed);
                            if let Some(handler) = handler {
                                let sender = BotSender {
                                    accounts: Arc::clone(&accounts_ref),
                                    ctx_store: ctx_store.clone(),
                                    primary_account_id: account_id_owned.clone(),
                                };
                                if let Err(e) = handler.handle(&parsed, &sender).await {
                                    error!(
                                        "[{}] 模块处理异常 [{}]: {}",
                                        account_id_owned,
                                        handler.name(),
                                        e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        error!(
                            "[{}] 轮询异常 ({}/{}): {}",
                            account_id_owned, consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                        );

                        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                            consecutive_failures = 0;
                            tokio::time::sleep(std::time::Duration::from_secs(BACKOFF_DELAY_S))
                                .await;
                        } else {
                            tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_S)).await;
                        }
                    }
                }
            }

            info!("[{}] 长轮询已停止", account_id_owned);
        });
    }

    /// 停止所有长轮询
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Bot 停止信号已发送");
    }
}

/// 账号信息（对外展示）
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub account_id: String,
    pub user_id: String,
}

/// 用户信息
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UserInfo {
    pub user_id: String,
    pub account_id: String,
}

/// 添加账号结果
#[derive(Debug, Clone)]
pub struct AddAccountResult {
    pub success: bool,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub error: Option<String>,
}

/// 二维码信息
#[derive(Debug, Clone)]
pub struct BindQRCode {
    pub qrcode: String,
    pub qrcode_img_url: String,
}

/// 扫码状态轮询结果
#[derive(Debug, Clone)]
pub struct BindPollResult {
    pub status: String,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
}
