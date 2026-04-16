use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 单条待办
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// 自增编号
    pub id: u32,
    /// 待办内容
    pub content: String,
    /// 是否完成
    pub done: bool,
    /// 创建时间戳（毫秒）
    pub created_at: i64,
    /// 完成时间戳（毫秒）
    pub completed_at: Option<i64>,
}

/// 用户的待办列表
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserTodos {
    /// 下一个可用编号
    pub next_id: u32,
    /// 待办列表
    pub items: Vec<TodoItem>,
}

/// TODO 存储
///
/// 按用户维度管理待办事项。
/// 持久化: 内存缓存 + JSON 文件（每个用户一个文件）。
///
/// 存储文件: `{state_dir}/todos/{user_id_hash}.json`
#[derive(Clone)]
pub struct TodoStore {
    /// user_id → UserTodos
    store: Arc<RwLock<HashMap<String, UserTodos>>>,
    state_dir: PathBuf,
}

impl TodoStore {
    pub async fn new(state_dir: &str) -> Self {
        let path = PathBuf::from(state_dir).join("todos");
        std::fs::create_dir_all(&path).ok();

        let store = Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            state_dir: path,
        };
        store.restore().await;
        store
    }

    /// 用户的持久化文件路径
    fn file_path(&self, user_id: &str) -> PathBuf {
        // 使用 user_id 的 MD5 前16位作为文件名，避免特殊字符
        let hash = format!("{:x}", md5::compute(user_id.as_bytes()));
        self.state_dir.join(format!("{}.json", &hash[..16]))
    }

    /// 添加待办
    pub async fn add(&self, user_id: &str, content: &str) -> TodoItem {
        let mut store = self.store.write().await;
        let todos = store.entry(user_id.to_string()).or_default();

        todos.next_id += 1;
        let item = TodoItem {
            id: todos.next_id,
            content: content.to_string(),
            done: false,
            created_at: chrono::Utc::now().timestamp_millis(),
            completed_at: None,
        };

        todos.items.push(item.clone());
        drop(store);

        self.persist(user_id).await;
        debug!("待办已添加: user={} id={}", user_id, item.id);
        item
    }

    /// 完成待办
    pub async fn complete(&self, user_id: &str, item_id: u32) -> Option<TodoItem> {
        let mut store = self.store.write().await;
        let todos = store.get_mut(user_id)?;

        let item = todos.items.iter_mut().find(|i| i.id == item_id && !i.done)?;
        item.done = true;
        item.completed_at = Some(chrono::Utc::now().timestamp_millis());
        let result = item.clone();
        drop(store);

        self.persist(user_id).await;
        debug!("待办已完成: user={} id={}", user_id, item_id);
        Some(result)
    }

    /// 删除待办
    pub async fn delete(&self, user_id: &str, item_id: u32) -> Option<TodoItem> {
        let mut store = self.store.write().await;
        let todos = store.get_mut(user_id)?;

        let pos = todos.items.iter().position(|i| i.id == item_id)?;
        let removed = todos.items.remove(pos);
        drop(store);

        self.persist(user_id).await;
        debug!("待办已删除: user={} id={}", user_id, item_id);
        Some(removed)
    }

    /// 获取未完成的待办列表
    pub async fn list_pending(&self, user_id: &str) -> Vec<TodoItem> {
        let store = self.store.read().await;
        match store.get(user_id) {
            Some(todos) => todos.items.iter().filter(|i| !i.done).cloned().collect(),
            None => Vec::new(),
        }
    }

    /// 获取所有待办（含已完成）
    pub async fn list_all(&self, user_id: &str) -> Vec<TodoItem> {
        let store = self.store.read().await;
        match store.get(user_id) {
            Some(todos) => todos.items.clone(),
            None => Vec::new(),
        }
    }

    /// 获取已完成数量
    pub async fn count_done(&self, user_id: &str) -> usize {
        let store = self.store.read().await;
        match store.get(user_id) {
            Some(todos) => todos.items.iter().filter(|i| i.done).count(),
            None => 0,
        }
    }

    /// 清空已完成的待办
    pub async fn clear_done(&self, user_id: &str) -> usize {
        let mut store = self.store.write().await;
        let todos = match store.get_mut(user_id) {
            Some(t) => t,
            None => return 0,
        };

        let before = todos.items.len();
        todos.items.retain(|i| !i.done);
        let removed = before - todos.items.len();
        drop(store);

        if removed > 0 {
            self.persist(user_id).await;
            debug!("已清空 {} 条已完成待办: user={}", removed, user_id);
        }
        removed
    }

    /// 从磁盘恢复所有用户的待办
    pub async fn restore(&self) {
        let entries = match std::fs::read_dir(&self.state_dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        let mut total = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }

            match tokio::fs::read_to_string(entry.path()).await {
                Ok(content) => {
                    match serde_json::from_str::<UserTodosFile>(&content) {
                        Ok(file_data) => {
                            let mut store = self.store.write().await;
                            store.insert(file_data.user_id.clone(), file_data.todos);
                            total += 1;
                        }
                        Err(e) => {
                            warn!("解析待办文件 {} 失败: {}", name, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("读取待办文件 {} 失败: {}", name, e);
                }
            }
        }

        if total > 0 {
            info!("已恢复 {} 个用户的待办数据", total);
        }
    }

    /// 持久化到磁盘
    async fn persist(&self, user_id: &str) {
        let todos = {
            let store = self.store.read().await;
            match store.get(user_id) {
                Some(t) => t.clone(),
                None => return,
            }
        };

        let file_data = UserTodosFile {
            user_id: user_id.to_string(),
            todos,
        };

        let file_path = self.file_path(user_id);
        match serde_json::to_string_pretty(&file_data) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&file_path, json).await {
                    warn!("持久化待办失败: {}", e);
                }
            }
            Err(e) => {
                warn!("序列化待办失败: {}", e);
            }
        }
    }
}

/// 持久化文件结构（包含 user_id 以便恢复时还原映射）
#[derive(Debug, Serialize, Deserialize)]
struct UserTodosFile {
    user_id: String,
    todos: UserTodos,
}
