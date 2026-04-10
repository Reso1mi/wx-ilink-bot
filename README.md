# 微信 iLink Bot 消息处理服务 (Rust)

基于微信 iLink Bot API（`@tencent-weixin/openclaw-weixin`）构建的消息处理服务，使用 Rust 实现。

## 工作原理

iLink Bot 是腾讯通过 OpenClaw 框架正式开放的合法 Bot API，基于 HTTP/JSON 长轮询协议。

### Bot 身份

- Bot 拥有**独立的 `@im.bot` 身份**（区别于普通用户的 `@im.wechat`）
- Bot 通过 `bot_type=3` 标识，是微信官方支持的 ClawBot 类型
- 腾讯仅作为消息收发的"管道"，不提供 AI 服务本身

### 多账号架构

**核心发现**：iLink 协议中，**每次扫码都会创建一个全新的 Bot 账号**（独立的 `@im.bot` 身份 + `bot_token`），而非"将用户绑定到已有 Bot"。因此系统采用多账号架构：

```
1. 启动服务 → HTTP 管理接口就绪 → 自动恢复已保存的账号
   ↓
2. 任何用户通过 HTTP 接口获取二维码 → 微信扫码确认
   ↓
3. 创建新 Account（独立 bot_token）→ 自动启动独立 getupdates 长轮询
   ↓
4. 所有 Account 的消息统一路由到同一个 MessageRouter
   ↓
5. 重启后自动从 state 目录恢复所有账号，无需重新扫码
```

> **关键**：每个 Account 拥有独立的 `bot_token`、`account_id`（`xxx@im.bot`）和 `getupdates` 消息流。没有管理员概念，所有用户平等，均通过 HTTP 接口按需添加。

### 核心机制：context_token

- **每条入站消息都携带 `context_token`**，这是 Bot 回复该用户的唯一凭证
- **Bot 无法主动联系没有 `context_token` 的用户** — 用户必须先给 Bot 发消息
- `context_token` 会随消息更新，系统始终保存最新值并持久化到磁盘

## 功能特性

- 🔐 **扫码登录** — 二维码登录、过期刷新、IDC 重定向
- 📩 **消息接收** — 长轮询获取多用户消息，自动维护同步游标
- 📝 **消息解析** — 支持文本/图片/语音/文件/视频 5 种消息类型
- 🔀 **智能路由** — 前缀/精确/正则/包含/类型 多种匹配方式
- 📤 **精准回复** — 通过 `to_user_id` + `context_token` 回复指定用户
- 🔌 **可插拔模块** — 业务逻辑完全解耦，3 步新增功能模块
- 💾 **状态持久化** — context_token / 同步游标 / 凭证自动持久化
- 🛡️ **容错设计** — 会话过期处理、连续失败退避、自动重连
- 🌐 **HTTP 管理接口** — 提供二维码获取、状态查看、用户列表等 API

## 快速开始

### 前置条件

- Rust 1.70+

### 安装与运行

```bash
# 克隆项目
git clone <repo-url>
cd wx-ilink-bot

# 复制配置文件
cp .env.example .env

# 编译运行
cargo run --release
```

启动后：
1. 自动恢复之前保存的账号（如有），恢复长轮询
2. HTTP 管理接口启动在 `http://localhost:3000`
3. 通过 `POST /account/add` 或 `/account/qrcode` 添加新账号

### Docker 部署

项目现在提供了 `Dockerfile` 和 `docker-compose.yml`，推荐直接用 Compose 启动。

```bash
# 如需自定义配置，先复制配置文件；不复制也会使用默认值
cp .env.example .env

# 创建状态目录（用于持久化 bot_token、context_token、游标等）
mkdir -p state

# 构建并后台启动
docker compose up -d --build
```

启动后：
1. 管理后台默认地址为 `http://localhost:3000/admin`
2. 容器内状态目录固定为 `/app/state`
3. 宿主机 `./state` 会挂载到容器内，重启容器不会丢失状态

如果你不需要自定义配置，也可以跳过 `cp .env.example .env`，Compose 会直接使用内置默认值。

常用命令：

```bash
# 查看日志
docker compose logs -f

# 停止服务
docker compose down
```

如果只想构建镜像并手动运行：

```bash
# 构建镜像
docker build -t wx-ilink-bot .

# 运行容器
docker run -d \
  --name wx-ilink-bot \
  --restart unless-stopped \
  -e BOT_STATE_DIR=/app/state \
  -p 3000:3000 \
  -v "$(pwd)/state:/app/state" \
  wx-ilink-bot
```

如需自定义环境变量，可在 `docker run` 后额外加上 `--env-file .env`。

### 配置

编辑 `.env` 文件：

| 环境变量 | 说明 | 默认值 |
|---------|------|--------|
| `BOT_STATE_DIR` | 状态文件存储目录 | `./state` |
| `BOT_LOG_LEVEL` | 日志级别 | `info` |
| `BOT_BASE_URL` | iLink API 地址 | `https://ilinkai.weixin.qq.com` |
| `BOT_APP_ID` | App ID | `bot` |
| `BOT_VERSION` | 客户端版本号 | `1.0.0` |
| `BOT_HTTP_PORT` | HTTP 管理接口端口 | `3000` |

## HTTP 管理接口

| 端点 | 方法 | 说明 |
|------|------|------|
| `/` | GET | 可用接口列表 |
| `/health` | GET | 健康检查 |
| `/status` | GET | Bot 状态（是否在线、账号数量、账号列表） |
| `/accounts` | GET | 所有账号列表 |
| `/users` | GET | 所有已连接用户（按账号分组） |
| `/account/add` | POST | 同步添加账号 — 生成二维码并阻塞等待扫码完成 |
| `/account/qrcode` | POST | 异步添加账号 Step 1 — 生成二维码 |
| `/account/status?qrcode=xxx` | GET | 异步添加账号 Step 2 — 轮询扫码状态 |
| `/message/send` | POST | 发送消息给指定用户（跨账号自动调度） |

### 添加新账号

**方式一：同步添加**（适合后台/脚本调用）

```bash
curl -X POST http://localhost:3000/account/add
# 阻塞等待用户扫码，返回:
# { "success": true, "account_id": "xxx@im.bot", "user_id": "xxx@im.wechat" }
```

**方式二：异步添加**（适合前端轮询）

```bash
# Step 1: 获取二维码
curl -X POST http://localhost:3000/account/qrcode
# → { "success": true, "qrcode": "xxx", "qrcode_img_url": "https://..." }

# Step 2: 轮询状态（每 1~2 秒）
curl "http://localhost:3000/account/status?qrcode=xxx"
# → { "status": "wait" }      等待扫码
# → { "status": "scaned" }    已扫码，等待确认
# → { "status": "confirmed", "account_id": "xxx@im.bot", "user_id": "xxx@im.wechat" }
# → { "status": "expired" }   过期，需重新调用 /account/qrcode
```

### 发送消息（跨账号自动调度）

```bash
curl -X POST http://localhost:3000/message/send \
  -H "Content-Type: application/json" \
  -d '{"to_user_id": "xxx@im.wechat", "text": "你好"}'
# → { "success": true, "to_user_id": "xxx@im.wechat" }
```

系统会自动查找该用户对应的 Bot 账号并发送消息。用户 A 的 Bot 也可以通过 "通知" 命令调度用户 B 的 Bot 来发送消息。

## 项目结构

```
src/
├── main.rs                 # 入口 — 初始化、注册模块、启动
├── config.rs               # 配置管理
│
├── core/                   # 核心层
│   ├── api.rs              # iLink API 封装
│   ├── auth.rs             # 扫码登录
│   ├── bot.rs              # Bot 主类 — 多账号管理、长轮询
│   ├── parser.rs           # 消息解析器
│   ├── router.rs           # 消息路由器
│   └── session.rs          # context_token 存储
│
├── modules/                # 业务模块层
│   ├── base.rs             # ModuleHandler trait
│   ├── echo_module.rs      # 回声模块
│   ├── query_module.rs     # 查询模块
│   ├── notify_module.rs    # 通知模块
│   └── help_module.rs      # 帮助模块
│
└── utils/
    ├── crypto.rs           # AES-128-ECB 加解密
    └── logger.rs           # 日志
```

## 新增业务模块

只需 3 步：

**Step 1**: 创建 `src/modules/my_module.rs`

```rust
use anyhow::Result;
use async_trait::async_trait;
use crate::core::parser::ParsedMessage;
use crate::modules::base::{reply, MessageSender, ModuleHandler};

pub struct MyModule;

#[async_trait]
impl ModuleHandler for MyModule {
    async fn handle(&self, msg: &ParsedMessage, sender: &dyn MessageSender) -> Result<()> {
        reply(sender, msg, "处理结果").await
    }
    fn name(&self) -> &str { "MyModule" }
}
```

**Step 2**: 在 `modules/mod.rs` 中添加 `pub mod my_module;`

**Step 3**: 在 `main.rs` 中注册路由

```rust
bot.router.register(
    RouteRule::new("my_rule", "我的命令", MatchType::Prefix),
    Arc::new(MyModule),
);
```

## 运维注意事项

1. **多账号机制** — 每次扫码创建一个独立的 Bot 账号（`@im.bot`），各自拥有独立的 `bot_token` 和消息流
2. **Bot 身份** — Bot 是 `@im.bot` 类型，不是个人微信号
3. **消息接收** — 用户给 Bot 发消息后，系统自动获取 `context_token`，Bot 即可回复该用户
4. **会话过期** — `bot_token` 由 iLink 服务端控制有效期，过期需重新扫码创建新账号
5. **独立轮询** — 每个账号运行独立的 `getupdates` 长轮询任务
6. **状态备份** — `state/` 目录包含 context_token、同步游标、凭证等关键数据

## License

MIT
