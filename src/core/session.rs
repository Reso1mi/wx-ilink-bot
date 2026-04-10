use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Context Token 管理
///
/// context_token 由 iLink 服务端在每条消息中下发，
/// 回复时必须携带对应用户的 context_token，否则消息可能无法送达。
///
/// 存储维度: accountId:userId → context_token
/// 持久化: 内存缓存 + JSON 文件（服务重启后恢复）
#[derive(Clone)]
pub struct ContextTokenStore {
    store: Arc<RwLock<HashMap<String, String>>>,
    state_dir: PathBuf,
}

impl ContextTokenStore {
    pub fn new(state_dir: &str) -> Self {
        let path = PathBuf::from(state_dir);
        // 确保目录存在
        std::fs::create_dir_all(&path).ok();

        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            state_dir: path,
        }
    }

    /// 生成存储 key
    fn key(account_id: &str, user_id: &str) -> String {
        format!("{}:{}", account_id, user_id)
    }

    /// 获取持久化文件路径
    fn file_path(&self, account_id: &str) -> PathBuf {
        self.state_dir
            .join(format!("{}.context-tokens.json", account_id))
    }

    /// 存储 context_token（内存 + 磁盘）
    pub async fn set(&self, account_id: &str, user_id: &str, token: &str) {
        let key = Self::key(account_id, user_id);
        {
            let mut store = self.store.write().await;
            store.insert(key, token.to_string());
        }
        // 异步持久化
        self.persist(account_id).await;
        debug!(
            "ContextToken 已存储: account={} user={}",
            account_id, user_id
        );
    }

    /// 获取 context_token
    pub async fn get(&self, account_id: &str, user_id: &str) -> Option<String> {
        let key = Self::key(account_id, user_id);
        let store = self.store.read().await;
        store.get(&key).cloned()
    }

    /// 获取某个 account 下所有用户的 context_token
    #[allow(dead_code)]
    pub async fn get_all_users(&self, account_id: &str) -> HashMap<String, String> {
        let prefix = format!("{}:", account_id);
        let store = self.store.read().await;
        store
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k[prefix.len()..].to_string(), v.clone()))
            .collect()
    }

    /// 清除某个 account 的所有 token
    #[allow(dead_code)]
    pub async fn clear_account(&self, account_id: &str) {
        let prefix = format!("{}:", account_id);
        {
            let mut store = self.store.write().await;
            store.retain(|k, _| !k.starts_with(&prefix));
        }
        let file_path = self.file_path(account_id);
        let _ = tokio::fs::remove_file(&file_path).await;
    }

    /// 从磁盘恢复 context_token（服务启动时调用）
    pub async fn restore(&self, account_id: &str) {
        let file_path = self.file_path(account_id);

        if !file_path.exists() {
            return;
        }

        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => match serde_json::from_str::<HashMap<String, String>>(&content) {
                Ok(tokens) => {
                    let mut count = 0;
                    let mut store = self.store.write().await;
                    for (user_id, token) in tokens {
                        if !token.is_empty() {
                            let key = Self::key(account_id, &user_id);
                            store.insert(key, token);
                            count += 1;
                        }
                    }
                    info!("已恢复 {} 个 context_token (account={})", count, account_id);
                }
                Err(e) => {
                    warn!("解析 context_token 文件失败: {}", e);
                }
            },
            Err(e) => {
                warn!("读取 context_token 文件失败: {}", e);
            }
        }
    }

    /// 持久化到磁盘
    async fn persist(&self, account_id: &str) {
        let prefix = format!("{}:", account_id);
        let tokens: HashMap<String, String> = {
            let store = self.store.read().await;
            store
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(k, v)| (k[prefix.len()..].to_string(), v.clone()))
                .collect()
        };

        let file_path = self.file_path(account_id);
        match serde_json::to_string_pretty(&tokens) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&file_path, json).await {
                    warn!("持久化 context_token 失败: {}", e);
                }
            }
            Err(e) => {
                warn!("序列化 context_token 失败: {}", e);
            }
        }
    }

    /// 保存同步游标到磁盘
    pub async fn save_sync_buf(&self, account_id: &str, buf: &str) {
        let file_path = self.state_dir.join(format!("{}.sync.json", account_id));

        let data = serde_json::json!({
            "get_updates_buf": buf,
        });

        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&file_path, json).await {
                    warn!("保存同步游标失败: {}", e);
                }
            }
            Err(e) => {
                warn!("序列化同步游标失败: {}", e);
            }
        }
    }

    /// 从磁盘恢复同步游标
    pub async fn restore_sync_buf(&self, account_id: &str) -> String {
        let file_path = self.state_dir.join(format!("{}.sync.json", account_id));

        if !file_path.exists() {
            return String::new();
        }

        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(data) => data["get_updates_buf"].as_str().unwrap_or("").to_string(),
                Err(_) => String::new(),
            },
            Err(_) => String::new(),
        }
    }

    /// 保存 Bot 凭证到磁盘
    pub async fn save_credentials(
        &self,
        account_id: &str,
        bot_token: &str,
        base_url: &str,
        user_id: &str,
    ) {
        let file_path = self.state_dir.join(format!("{}.json", account_id));

        let data = serde_json::json!({
            "bot_token": bot_token,
            "base_url": base_url,
            "user_id": user_id,
            "account_id": account_id,
        });

        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&file_path, json).await {
                    warn!("保存凭证失败: {}", e);
                }
                // 设置文件权限（Unix 系统）
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(0o600);
                    let _ = std::fs::set_permissions(&file_path, perms);
                }
            }
            Err(e) => {
                warn!("序列化凭证失败: {}", e);
            }
        }
    }

    /// 从磁盘恢复 Bot 凭证
    #[allow(dead_code)]
    pub async fn restore_credentials(&self, account_id: &str) -> Option<(String, String, String)> {
        let file_path = self.state_dir.join(format!("{}.json", account_id));

        if !file_path.exists() {
            return None;
        }

        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(data) => {
                    let bot_token = data["bot_token"].as_str()?.to_string();
                    let base_url = data["base_url"].as_str()?.to_string();
                    let user_id = data["user_id"].as_str().unwrap_or("").to_string();
                    Some((bot_token, base_url, user_id))
                }
                Err(_) => None,
            },
            Err(_) => None,
        }
    }
}
