use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 用户昵称存储
///
/// 维护 user_id → nickname 的双向映射。
/// 持久化: 内存缓存 + JSON 文件（服务重启后恢复）。
///
/// 存储文件: `{state_dir}/nicknames.json`
#[derive(Clone)]
pub struct NicknameStore {
    /// user_id → nickname
    store: Arc<RwLock<HashMap<String, String>>>,
    state_dir: PathBuf,
}

impl NicknameStore {
    pub async fn new(state_dir: &str) -> Self {
        let path = PathBuf::from(state_dir);
        std::fs::create_dir_all(&path).ok();

        let store = Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            state_dir: path,
        };
        store.restore().await;
        store
    }

    /// 持久化文件路径
    fn file_path(&self) -> PathBuf {
        self.state_dir.join("nicknames.json")
    }

    /// 设置昵称
    pub async fn set(&self, user_id: &str, nickname: &str) {
        {
            let mut store = self.store.write().await;
            store.insert(user_id.to_string(), nickname.to_string());
        }
        self.persist().await;
        debug!("昵称已设置: user={} nickname={}", user_id, nickname);
    }

    /// 获取昵称
    pub async fn get(&self, user_id: &str) -> Option<String> {
        let store = self.store.read().await;
        store.get(user_id).cloned()
    }

    /// 通过昵称反查 user_id（返回第一个匹配的）
    pub async fn find_by_nickname(&self, nickname: &str) -> Option<String> {
        let store = self.store.read().await;
        for (uid, nick) in store.iter() {
            if nick == nickname {
                return Some(uid.clone());
            }
        }
        None
    }

    /// 获取所有昵称映射
    #[allow(dead_code)]
    pub async fn get_all(&self) -> HashMap<String, String> {
        let store = self.store.read().await;
        store.clone()
    }

    /// 获取用户的显示名称（有昵称返回昵称，否则返回脱敏 ID）
    pub async fn display_name(&self, user_id: &str) -> String {
        match self.get(user_id).await {
            Some(nickname) => nickname,
            None => desensitize_id(user_id),
        }
    }

    /// 从磁盘恢复（启动时调用）
    pub async fn restore(&self) {
        let file_path = self.file_path();
        if !file_path.exists() {
            return;
        }

        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => match serde_json::from_str::<HashMap<String, String>>(&content) {
                Ok(data) => {
                    let count = data.len();
                    let mut store = self.store.write().await;
                    *store = data;
                    info!("已恢复 {} 个用户昵称", count);
                }
                Err(e) => {
                    warn!("解析昵称文件失败: {}", e);
                }
            },
            Err(e) => {
                warn!("读取昵称文件失败: {}", e);
            }
        }
    }

    /// 持久化到磁盘
    async fn persist(&self) {
        let data = {
            let store = self.store.read().await;
            store.clone()
        };

        let file_path = self.file_path();
        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&file_path, json).await {
                    warn!("持久化昵称失败: {}", e);
                }
            }
            Err(e) => {
                warn!("序列化昵称失败: {}", e);
            }
        }
    }
}

/// 脱敏用户 ID：取 `@` 前的部分，只显示前4位 + 后4位
///
/// 示例: `abcdefgh1234@im.wechat` → `用户_abcd…1234`
pub fn desensitize_id(user_id: &str) -> String {
    let local = user_id.split('@').next().unwrap_or(user_id);
    if local.len() <= 8 {
        format!("用户_{}", local)
    } else {
        format!("用户_{}…{}", &local[..4], &local[local.len() - 4..])
    }
}
