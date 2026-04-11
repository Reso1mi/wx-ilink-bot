use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::core::api::ILinkAPI;

/// 扫码确认结果
///
/// 每次用户扫码确认后返回此结构。
/// - 首次扫码：用于初始化 Bot（获取 bot_token）
/// - 后续扫码：用于新用户与 Bot 建立连接
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// 是否成功
    pub success: bool,
    /// Bot Token（每次扫码都会返回，后续可能与首次相同）
    pub bot_token: String,
    /// 账号 ID（ilink_bot_id, xxx@im.bot）
    pub account_id: String,
    /// 基础 URL（可能重定向后的地址）
    pub base_url: String,
    /// 扫码用户 ID（xxx@im.wechat）
    pub user_id: String,
    /// 错误信息
    pub error: Option<String>,
}

/// 当前二维码状态（内部追踪用）
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct QRCodeInfo {
    /// 二维码标识
    pub qrcode: String,
    /// 二维码图片链接
    pub qrcode_img_url: String,
    /// 当前状态: waiting / scaned / expired / confirmed
    pub status: String,
    /// 扫码用户（confirmed 后有值）
    pub confirmed_user_id: Option<String>,
}

/// 二维码管理器
///
/// 管理 Bot 的二维码生命周期：
/// - 生成二维码，等待用户扫码
/// - 扫码确认后创建独立的 Bot 账号
///
/// 每个二维码只能被一个用户扫码确认一次，确认后需要生成新的二维码。
pub struct QRCodeManager {
    api: ILinkAPI,
    /// 当前二维码信息
    current_qr: Arc<RwLock<Option<QRCodeInfo>>>,
}

/// 二维码最大刷新次数（单轮）
const MAX_QR_REFRESH: usize = 3;
/// 单个二维码等待超时（秒）
const QR_TIMEOUT_S: u64 = 480;

impl QRCodeManager {
    pub fn new(api: ILinkAPI) -> Self {
        Self {
            api,
            current_qr: Arc::new(RwLock::new(None)),
        }
    }

    /// 获取当前二维码信息（供 HTTP 接口查询）
    #[allow(dead_code)]
    pub async fn get_current_qr(&self) -> Option<QRCodeInfo> {
        let qr = self.current_qr.read().await;
        qr.clone()
    }

    /// 执行一次完整的扫码流程
    ///
    /// 生成二维码 → 等待用户扫码 → 返回结果。
    /// 此方法会阻塞直到：用户扫码确认 / 超时 / 多次过期。
    pub async fn wait_for_scan(&self) -> Result<ScanResult> {
        let mut qr_refresh_count = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(QR_TIMEOUT_S);

        // 获取二维码
        let qr_data = self.api.get_bot_qrcode("3").await?;
        let mut qrcode = qr_data.qrcode.unwrap_or_default();
        let mut qrcode_url = qr_data.qrcode_img_content.unwrap_or_default();
        let mut current_base_url = self.api.base_url.clone();

        // 更新当前二维码状态
        self.update_qr_status(&qrcode, &qrcode_url, "waiting").await;

        info!("新二维码已生成: {}", qrcode_url);

        while std::time::Instant::now() < deadline {
            let status_resp = self
                .api
                .get_qrcode_status(&current_base_url, &qrcode)
                .await?;
            let status = status_resp.status.as_deref().unwrap_or("wait");

            match status {
                "wait" => {
                    // 继续轮询
                }
                "scaned" => {
                    self.update_qr_status(&qrcode, &qrcode_url, "scaned").await;
                    info!("二维码已被扫描，等待确认...");
                }
                "scaned_but_redirect" => {
                    if let Some(ref redirect_host) = status_resp.redirect_host {
                        current_base_url = format!("https://{redirect_host}");
                        info!("IDC 重定向到: {}", current_base_url);
                    }
                }
                "expired" => {
                    qr_refresh_count += 1;
                    if qr_refresh_count >= MAX_QR_REFRESH {
                        self.clear_qr().await;
                        return Ok(ScanResult {
                            success: false,
                            bot_token: String::new(),
                            account_id: String::new(),
                            base_url: String::new(),
                            user_id: String::new(),
                            error: Some("二维码多次过期".to_string()),
                        });
                    }

                    warn!(
                        "二维码过期，刷新中 ({}/{})",
                        qr_refresh_count, MAX_QR_REFRESH
                    );
                    let new_qr = self.api.get_bot_qrcode("3").await?;
                    qrcode = new_qr.qrcode.unwrap_or_default();
                    qrcode_url = new_qr.qrcode_img_content.unwrap_or_default();
                    self.update_qr_status(&qrcode, &qrcode_url, "waiting").await;
                    info!("新二维码: {}", qrcode_url);
                }
                "confirmed" => {
                    let user_id = status_resp.ilink_user_id.clone().unwrap_or_default();

                    // 更新状态为已确认
                    {
                        let mut qr = self.current_qr.write().await;
                        if let Some(ref mut info) = *qr {
                            info.status = "confirmed".to_string();
                            info.confirmed_user_id = Some(user_id.clone());
                        }
                    }

                    let result = ScanResult {
                        success: true,
                        bot_token: status_resp.bot_token.unwrap_or_default(),
                        account_id: status_resp.ilink_bot_id.unwrap_or_default(),
                        base_url: status_resp
                            .baseurl
                            .unwrap_or_else(|| current_base_url.clone()),
                        user_id,
                        error: None,
                    };

                    info!(
                        "扫码确认! user={} account={}",
                        result.user_id, result.account_id
                    );

                    return Ok(result);
                }
                other => {
                    warn!("未知的扫码状态: {}", other);
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        self.clear_qr().await;
        Ok(ScanResult {
            success: false,
            bot_token: String::new(),
            account_id: String::new(),
            base_url: String::new(),
            user_id: String::new(),
            error: Some("扫码等待超时".to_string()),
        })
    }

    /// 更新当前二维码状态
    async fn update_qr_status(&self, qrcode: &str, url: &str, status: &str) {
        let mut qr = self.current_qr.write().await;
        *qr = Some(QRCodeInfo {
            qrcode: qrcode.to_string(),
            qrcode_img_url: url.to_string(),
            status: status.to_string(),
            confirmed_user_id: None,
        });
    }

    /// 清除当前二维码
    async fn clear_qr(&self) {
        let mut qr = self.current_qr.write().await;
        *qr = None;
    }
}
