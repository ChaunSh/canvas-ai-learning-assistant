# Canvas 接口调研记录

> 本文件记录**实际观察到的** Canvas 接口行为与网页结构，作为实现依据。
> 未经实测的部分会明确写「未验证」，不把推断当作结论。

## 1. 站点信息

| 项 | 值 | 来源 |
| --- | --- | --- |
| 站点地址 | `https://hkust-gz.instructure.com` | 任务书 |
| 产品 | Canvas LMS（Instructure） | 站点页面标题 / 登录页 DOM |
| 登录入口 | `/login` → 学校 SSO | 未实测 |
| 前端 | Canvas 自带 React/Backbone 混合 | 未实测 |

> 观察方式说明：本项目在开发阶段只访问了公开可访问的登录页以确认站点可达性，
> **未使用任何账号登录**，因此以下「接口行为」均来自 Canvas 公开文档与常见部署形态，
> 并已在代码中按「能力需实测」处理。

## 2. 认证模型

Canvas 支持三种访问方式：

| 方式 | 是否使用 | 原因 |
| --- | --- | --- |
| 网页会话（Cookie） | ✅ 唯一入口 | 任务书要求 |
| Personal Access Token | ❌ | 任务书禁止作为入口 |
| OAuth2（Developer Key） | ❌ | 需学校配置，且任务书禁止 |

**关键结论**：网页登录后，浏览器持有 `_normandy_session` 之类的会话 Cookie，
Canvas 的 `/api/v1/*` 接口接受该会话（同源请求）。
但这**不等于**拿到可独立使用的 API Token —— 因此不能在 Rust 侧复现请求，
只能在页面内同源调用。这正是本项目的桥接方案存在的原因。

## 3. 接口形态（按 Canvas REST API 约定）

### 3.1 通用

- 前缀：`/api/v1`
- 分页：`Link` 响应头（`rel="next"`），`per_page` 最大 100
- 错误：`{"status":"unauthorized","errors":[{"message":"..."}]}`
- 内容协商：`Accept: application/json`

### 3.2 本项目使用的端点

| 用途 | 方法与路径 | 关键参数 |
| --- | --- | --- |
| 当前用户 | `GET /api/v1/users/self` | — |
| 课程 | `GET /api/v1/courses` | `enrollment_state=active`, `per_page=100` |
| 模块 | `GET /api/v1/courses/:id/modules` | `include[]=items` |
| 课程页面 | `GET /api/v1/courses/:id/pages` | `per_page=100` |
| 文件 | `GET /api/v1/courses/:id/files` | `per_page=100` |
| 作业 | `GET /api/v1/courses/:id/assignments` | `per_page=100` |
| 公告 | `GET /api/v1/courses/:id/discussion_topics` | `only_announcements=true` |
| 日历 | `GET /api/v1/calendar_events` | `type=event`, `start_date`, `end_date` |
| 成绩 | `GET /api/v1/courses/:id/enrollments` | `include[]=grades`, `user_id=self` |
| 讨论 | `GET /api/v1/courses/:id/discussion_topics` | `per_page=100` |
| 提交状态 | `GET /api/v1/courses/:id/assignments/:aid/submissions/self` | — |

### 3.3 权限敏感性

- `enrollments`（成绩）常因课程设置被拒 → 预期可能出现 `403`，界面显示「无权限」；
- `files` 只返回元数据；实际下载需要 `url` 字段（带时效令牌）；
- 部分学校对 `discussion_topics` 关闭 API 访问。

因此能力探测必须**逐项**进行，不能因为「课程能拿到」就推断「成绩也能拿到」。

## 4. 网页结构要点（用于登录判定）

未实测，仅记录设计假设：

- 登录成功后 URL 会跳到 `/` 或 `/courses`，但**跳转本身不可作为成功依据**
  （可能是缓存的登录页、也可能跳到权限提示页）；
- 判定依据改为「`/api/v1/users/self` 返回 200 且含 `id`」；
- SSO 流程通常涉及 302 到学校 IdP，可能有 MFA 页面，因此窗口内要允许完整导航；
- 部分部署会启用「新登录页」（React），DOM 结构不稳定 → 不做 DOM 选择器依赖。

## 5. 未验证项（明确列出）

1. HKUST(GZ) 是否启用全部上述端点；
2. 学生账号对 `enrollments`、`files`、`discussion_topics` 的实际权限；
3. 是否存在额外的 CSRF 校验（同源 fetch 通常自动带 `X-CSRF-Token`，但未实测）；
4. 会话有效期与续期行为；
5. 文件下载 URL 的时效与是否绑定 IP。

这些都会在首次真实登录后补齐，并同步更新
[docs/canvas-capabilities.md](canvas-capabilities.md) 与
[docs/implementation-status.md](implementation-status.md)。

## 6. 参考

- Canvas LMS REST API 文档：`https://canvas.instructure.com/doc/api/`
- Canvas 开源仓库：`https://github.com/instructure/canvas-lms`

> 上述文档用于确认端点形态；本项目的实现不复制其代码，
> 认证与取数策略由本仓库的安全约束（见 `docs/security.md`）决定。
