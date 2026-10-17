# Canvas 能力矩阵

## 1. 如何判定「可用」

只有满足以下**全部**条件才标记 `supported`：

1. 请求在已登录的 Canvas 页面内同源发出；
2. HTTP 200；
3. 响应体是预期的 JSON 结构（数组/对象字段齐全）；
4. 数据能成功映射到本地实体并入库。

若返回 200 但结构不符（例如返回了 HTML 登录页），判定为 `unsupported`，
不会因为「状态码是 200」就认为成功。

空数组是**合法空结果**：表示该课程确实没有作业，此时仍为 `supported`。

## 2. 能力清单与端点

| Key | 端点 | 说明 | 当前状态 |
| --- | --- | --- | --- |
| `courses` | `GET /api/v1/courses` | 学生可见课程 | ⬜ 未测试 |
| `modules` | `GET /api/v1/courses/:id/modules?include[]=items` | 章节与条目 | ⬜ 未测试 |
| `pages` | `GET /api/v1/courses/:id/pages` | 课程页面列表 | ⬜ 未测试 |
| `files` | `GET /api/v1/courses/:id/files` | 文件元数据（不含正文） | ⬜ 未测试 |
| `assignments` | `GET /api/v1/courses/:id/assignments` | 作业与截止时间 | ⬜ 未测试 |
| `announcements` | `GET /api/v1/courses/:id/discussion_topics?only_announcements=true` | 公告 | ⬜ 未测试 |
| `calendar` | `GET /api/v1/calendar_events?type=event&start_date=…&end_date=…` | 日历事件 | ⬜ 未测试 |
| `grades` | `GET /api/v1/courses/:id/enrollments?include[]=grades&user_id=self` | 成绩 | ⬜ 未测试 |
| `discussions` | `GET /api/v1/courses/:id/discussion_topics` | 讨论主题（只读） | ⬜ 未测试 |
| `submissions` | `GET /api/v1/courses/:id/assignments/:aid/submissions/self` | 提交状态 | ⬜ 未测试 |

> 当前开发环境没有学校账号，**全部为「未测试」**。上表「当前状态」会随实测更新。

## 3. 状态枚举与界面表现

| 状态 | 含义 | 界面 |
| --- | --- | --- |
| `supported` | 真实请求成功且结构正确 | 绿色「可用」 |
| `unsupported` | 端点返回了非预期内容（例如被重定向到 HTML） | 灰色「不支持」 |
| `permission-denied` | 403，账号无权访问该项 | 黄色「无权限」 |
| `login-required` | 401，需要重新登录 | 蓝色「需登录」 |
| `not-tested` | 尚未验证（初始值） | 灰色「未测试」 |
| `temporarily-unavailable` | 429 / 5xx / 网络错误 | 黄色「暂不可用」 |

`permission-denied` 与 `unsupported` 的区别很重要：前者是账号权限问题（换课程或申请权限可解），
后者是接口形态问题（需要改代码适配）。

## 4. 明确不支持

| 能力 | 原因 | 替代方案 |
| --- | --- | --- |
| 文件正文自动下载 | 需要页面内临时令牌 + 会话，无法在不提取 Cookie 的前提下由 Rust 发起 | 手动下载后「导入课件」 |
| 提交作业 / 发帖 / 改设置 | 只读集成，避免误操作学校系统 | 在 Canvas 网页内操作 |
| 使用 PAT / OAuth | 任务书禁止作为登录入口 | — |
| 读取学校 SSO 令牌 | 安全约束禁止 | — |

## 5. 同步行为

```
sync_all
  ├─ 检查会话（401 → session-expired，停止并提示重新登录）
  ├─ 课程列表 → upsert courses（以 external_id 为键）
  ├─ 每门课程并发受限地拉取：modules / pages / files / assignments /
  │   announcements / discussions / enrollments
  ├─ 逐项记录能力状态（成功/无权限/不支持）
  ├─ 写 sync_runs.stats（每类数量、跳过数、下载字节）
  └─ 失败项写 sync_errors（含可重试标记），不中断整体同步
```

原则：

- **单点失败不整体失败**：某门课的讨论区 403 不会让整个同步报错；
- **幂等**：以 `external_id` upsert，重复同步不产生重复行；
- **可解释**：每类资源的处理数量都写入统计，界面可见；
- **进度可见**：`canvas://sync` 事件推送「当前阶段 / 已处理 / 总数」。

## 6. 实测记录格式

在有账号的环境中实测后，请按下表更新 `docs/implementation-status.md` 与本文件：

| 日期 | 能力 | 结果 | 证据 |
| --- | --- | --- | --- |
| — | — | 尚无实测 | — |
