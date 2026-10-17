# 架构说明

## 1. 总览

```
┌──────────────────────────────────────────────────────────────┐
│  WebView（Windows: WebView2 / Android: System WebView）        │
│  React 19 + TypeScript + Tailwind 4 + React Router (hash)      │
│  ── 页面、状态（TanStack Query + Zustand）、中英文文案          │
└───────────────▲───────────────────────────┬──────────────────┘
                │ Tauri IPC（JSON）          │ 事件（stream / progress）
┌───────────────┴───────────────────────────▼──────────────────┐
│  Rust 核心（crate: canvas_ai_assistant_lib）                   │
│                                                                │
│  commands/      ~95 个 #[tauri::command]，参数校验 + 窗口守卫    │
│  domain/        线格式数据结构（serde camelCase）               │
│  storage/       rusqlite + 手写迁移 + 各实体仓储               │
│  documents/     PDF/PPTX/DOCX/文本/ICS 解析 → 分块 → 索引      │
│  retrieval/     FTS5(BM25) + 向量 + RRF 融合                   │
│  ai/            OpenAI 兼容客户端、提示词、流式、引用校验        │
│  planning/      确定性调度器（纯函数）                          │
│  integrations/canvas/  网页登录窗口 + 同源取数 + 能力探测 + 同步 │
│  security/      TLS 提供者、密钥存储、路径守卫                  │
└───────────────┬──────────────────────────────────────────────┘
                │
      ┌─────────▼─────────┐        ┌──────────────────────────┐
      │ SQLite（单文件）   │        │ 课件文件目录（按哈希命名） │
      │ + FTS5 虚表        │        │                          │
      └───────────────────┘        └──────────────────────────┘
```

## 2. 关键设计决策

### 2.1 为什么是「本地优先」

- 课件与对话属于个人学习数据，不上传第三方存储；
- 检索在本地完成，只有被选中的片段进入提示词；
- 断网时除 AI 之外的功能全部可用（检索、阅读、计划、日历）。

### 2.2 为什么单连接 + `parking_lot::Mutex`

SQLite 单写者模型下，一个连接 + 互斥比连接池更简单且不会出现 `SQLITE_BUSY`。
所有跨多表的写入走 `Db::transaction`，读取走 `Db::with`。
代价是并发度为 1，但本地应用的数据量（万级 chunk）完全够用。

### 2.3 为什么迁移用 `PRAGMA user_version`

不引入迁移框架：`migrations/001_init.sql` 以 `include_str!` 编译进二进制，
按版本号顺序执行，每个文件在一个事务里完成。好处是可离线、可审计、零额外依赖。

### 2.4 为什么中文用 bigram

FTS5 的 `unicode61` 分词器不切分中文，整段会被当成一个 token。
方案：入库前把文本预分词成「拉丁词小写 + 中文重叠二元组」，查询用**同一个函数**处理，
因此不会出现「索引里有、查不到」的经典错配。

```
"导数的定义" → "导数 数的 的定 定义"
```

### 2.5 为什么页码必须来自解析而不是模型

模型只会输出 `[S 3]` 这样的证据编号。编号 → chunk → page 的映射完全由本地检索数据决定，
模型无法伪造页码。若格式没有分页（DOCX/Markdown/纯文本），界面只显示章节，不显示页码。

### 2.6 为什么调度器是纯函数

`planning::scheduler::schedule(...)` 不碰数据库、不碰时钟、不调 AI：
输入「可用时间窗 + 候选任务 + 已占用时间」，输出「排期块 + 未排期原因 + 说明」。
因此它可以被穷举测试，也保证「AI 建议内容，但不决定时间表」。

### 2.7 为什么 Canvas 数据在页面内取

网页登录得到的是**浏览器会话**，不是 API Token。
若 Rust 侧直接发请求，就必须读取 Cookie —— 这是任务书明令禁止的。
因此：请求在 Canvas 页面内以同源 `fetch` 执行（浏览器自动附带会话），
结果通过一次**受控导航**回传给 Rust。

```
Rust: 创建 canvas-workspace 窗口 → 注入脚本 → 页面同源 fetch
      → 脚本把结果编码进 /__caa_bridge__/{nonce}?status=...&body=...
      → Rust 的 on_navigation 拦截该 URL 并取消导航 → 解析结果
```

关键点：
- 该窗口**没有**被授予任何应用命令能力（capabilities 只含 `main`）；
- 桥接 URL 带一次性随机 nonce，且只接受本窗口发起的导航；
- 结果体有大小上限，避免把大响应塞进 URL。

### 2.8 命令层的双重守卫

Tauri 2 的应用命令默认对所有窗口可用，而 `canvas-workspace` 加载的是远程页面。
因此每个命令入口都调用 `require_main_window`：只有 `main` 窗口能调用应用命令。
这比依赖 capability 配置更直接，也更容易审计。

## 3. 主要数据流

### 3.1 导入课件

```
选择文件 → import_materials
  ├─ 计算 SHA-256（流式）→ 命中已有课件则 duplicate=true 直接返回
  ├─ 复制到 materials/<hash 前两位>/<hash>.<ext>
  ├─ 判定 kind（扩展名 + MIME）
  ├─ ZIP → archive::extract_zip（限制条目数/总字节/深度，拒绝穿越）
  └─ 写 materials 行，index_status = pending
→ 后台 index_material
  ├─ documents::extract::extract（按 kind 分派）
  ├─ documents::chunker::chunk_pages（按标题层级 + 目标 token 切块）
  ├─ 写 document_pages / document_chunks
  ├─ 写 chunk_fts（预分词后的 index_text）
  └─ index_status = indexed / partial / failed
```

进度通过 `materials://index` 事件推送到界面。

### 3.2 提问

```
ask_question
  ├─ 解析 scope（全部 / 课程 / 单课件 / 不检索）
  ├─ retrieval::search
  │    ├─ FTS5 BM25（AND → 失败回退 OR）
  │    ├─ 若配置了嵌入模型：向量检索 + 余弦
  │    └─ RRF 融合（k=60）+ 同课件多样性上限
  ├─ prompt::build：系统约束 + <课件证据> 包裹的片段
  ├─ 写 user message / assistant message（status=streaming）
  └─ 返回 AskAccepted{ requestId, ... } → 后台流式生成
       → ai://stream 事件：started / delta / citations / usage / completed / failed
```

生成结束后：
1. 从回答里提取 `[S n]`；
2. 只保留**真实存在**的证据编号，其余记日志丢弃；
3. 写 `chat_citations`（含材料、页码、章节、摘录）；
4. `completed` 事件带回完整消息（含引用），前端替换临时消息。

### 3.3 生成学习计划

```
generate_study_plan
  ├─ 读取 study_preferences（每日分钟数、时间窗、学习日、时区）
  ├─ 候选任务：未完成作业（按截止时间）+ 课件复习（按页数估算）
  ├─ scheduler::schedule(时间窗, 已占用, 候选) → 排期块 + 未排期
  ├─ 写 study_plans / study_tasks（generated_by = rule）
  └─ 返回 PlanResult{ plan, scheduled, unscheduled, explanation }
```

`replan_schedule` 会保留 `locked=true` 与 `generated_by='user'` 的任务，只重排其余。

## 4. 前端结构

| 层 | 职责 |
| --- | --- |
| `app/router.tsx` | hash 路由（打包后自定义协议下刷新不 404），PDF 阅读器按需加载 |
| `components/` | UI 基元（Button/Card/Dialog/Tabs…）、布局（侧边栏/底部导航）、Markdown 渲染 |
| `pages/` | 12 个功能页面 |
| `services/api.ts` | 命令封装 + 错误归一化为 `IpcError{code,message}` |
| `services/events.ts` | 事件订阅（流式、索引进度、同步进度） |
| `stores/` | 设置（主题/语言/隐私，localStorage 即时生效 + 数据库镜像）、Toast |

设计系统集中在 `src/styles/index.css`：品牌色、`--app-*` 变量、深色模式、
安全区工具类、Markdown/KaTeX/代码高亮样式、`prefers-reduced-motion` 适配。

## 5. 跨平台差异

| 关注点 | Windows | Android |
| --- | --- | --- |
| WebView | WebView2（Chromium 155） | 系统 WebView（版本随设备，要求 ≥ Chrome 110 特性集） |
| 密钥存储 | Credential Manager（`advapi32` FFI） | Keystore + Kotlin `SecureStore` + JNI |
| 布局 | 侧边栏 | 顶部栏 + 底部导航 + 安全区 |
| 数据目录 | `%APPDATA%\com.hkustgz.canvasai\` | 应用私有目录 |
| 文件选择 | 系统对话框（`plugin-dialog`） | Android SAF |

前端代码 100% 共用，差异仅在少量 `useIsDesktop()` 分支与后端 `#[cfg]`。
