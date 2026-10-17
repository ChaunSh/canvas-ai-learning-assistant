# Canvas 网页登录集成

## 1. 约束与结论

任务书要求：

> Canvas 网页登录是唯一的 Canvas 身份认证入口。
> 不提取浏览器 Cookie / 不将 Cookie 复制到 Rust HTTP 客户端 / 不绕过 CORS / CSRF / SSO/MFA / 不禁用同源策略。

> **2026-10-18 变更**：经用户明确授权，「课件下载」一项改为读取 Canvas 窗口的会话 Cookie 并由 Rust 发起下载；其余约束（不读密码、不绕 CSRF/MFA、不禁用同源策略）继续成立。详见 implementation-status.md 变更记录。
> 登录状态必须由可验证的正常页面行为确认，不允许仅凭页面重定向或 HTTP 200 就判断登录成功。
> 网页成功登录不等于获得独立 Canvas API Access Token。

因此本集成的形态是：**浏览器窗口负责认证，页面内同源请求负责取数，Rust 只做编排。**

明确不做的事：

| 不做 | 原因 |
| --- | --- |
| Personal Access Token | 任务书禁止作为入口 |
| OAuth / Developer Key | 同上，且需要学校侧配置 |
| 读取 `document.cookie` / Cookie 数据库 | 违反「不提取 Cookie」；**课件下载是唯一例外**（cookies_for_url，仅内存、仅一次请求） |
| 把会话注入 Rust `reqwest` | 等同复制会话 |
| 关闭 WebView 同源策略 | 违反安全约束 |
| 用「发生跳转」或「HTTP 200」判定登录成功 | 任务书明确禁止 |

## 2. 窗口拓扑

```
main 窗口（本地 UI）            canvas-workspace 窗口（远程站点）
  ├─ 可以调用所有应用命令          ├─ 没有被授予任何应用命令能力
  ├─ 负责显示同步结果与诊断        ├─ 由用户完成学校登录（SSO / MFA）
  └─ 不接触远程页面 DOM            └─ 只执行 Rust 注入的取数脚本
```

能力配置文件 `src-tauri/capabilities/default.json` 的 `windows` 只有 `["main"]`，
远程窗口即使被注入恶意脚本也无法调用 `export_user_data` 之类的命令。

## 3. 登录流程

1. **打开**：`open_canvas_login` 创建（或复用）`canvas-workspace` 窗口，
   导航到 `https://hkust-gz.instructure.com/login`。
2. **用户操作**：在窗口内完成学校账号登录（含 SSO 跳转、MFA）。
   应用**不读取也不保存**密码、Cookie、令牌。
3. **探测**：Rust 注入一段脚本，在页面内同源请求 `GET /api/v1/users/self`。
4. **判定**：只有脚本返回 `status=200` 且响应体含有效用户对象，才把状态置为
   `web-authenticated`；随后再由一次真实数据请求（课程列表）成功，才置为
   `data-access-verified`。
5. **失败分类**：401 → `login-required`；403 → `permission-denied`；
   非 JSON / 非预期结构 → `unsupported`；网络错误 → `error`。

> 「页面跳转到了 dashboard」不作为成功依据，因为它可能来自缓存、也可能权限不足。

## 4. 同源取数桥

```
Rust                                WebView（远程页面上下文）
 │  eval(script)                            │
 ├─────────────────────────────────────────►│
 │                                          │ fetch('/api/v1/courses', {credentials:'include'})
 │                                          │   ↑ 浏览器自动附带会话，Rust 从不接触 Cookie
 │                                          │ 把 status/headers/body 编码进桥接 URL
 │  on_navigation 拦截                      │
 │◄─────────────────────────────────────────┤ location.replace('/__caa_bridge__/{nonce}?...')
 │  取消导航，解析查询参数，销毁使用的 nonce    │
```

设计要点：

- **一次性 nonce**：每次调用生成新的随机 nonce，只接受匹配该 nonce 的导航；
- **导航即返回**：不依赖 `postMessage` 或自定义协议注册；
- **不落盘**：响应体只存在于内存，用完即释放（同步结果写库的是解析后的实体）；
- **体积上限**：结果体超过阈值时返回 `truncated`，避免 URL 过长导致导航失败；
- **取消导航**：`on_navigation` 返回 `false`，页面不会真的跳到桥接地址。

## 5. 能力探测

每类数据独立探测，记录 `supported / unsupported / permission-denied / login-required /
temporarily-unavailable / not-tested`，并保存最近一次检查时间与错误摘要。

探测规则（对每类资源）：

| 结果 | 判定 |
| --- | --- |
| 200 + 合法 JSON 数组（可为空） | `supported`，空数组是**合法空结果** |
| 200 + 结构不符（HTML 登录页等） | `unsupported` |
| 401 | `login-required` |
| 403 | `permission-denied` |
| 429 / 5xx | `temporarily-unavailable` |

能力状态会显示在「Canvas 连接」页的表格里，用户能直接看到哪些数据拿不到。

## 6. 当前状态

**未实测。** 开发环境没有 HKUST(GZ) 账号，因此：

- 所有能力初始为 `not-tested`；
- 界面明确显示「需登录」而不是假装可用；
- `docs/implementation-status.md` 第 5 节全部标 ⬜。

在有账号的环境里，验证顺序建议：

1. `open_canvas_login` → 手动登录 → 确认窗口停在 Canvas 页面；
2. `check_canvas_connection` → 期望 `data-access-verified`；
3. `get_canvas_capabilities` → 记录每项真实结果；
4. `sync_all` → 查看同步统计与错误列表；
5. 若某能力返回 403，界面应显示「无权限」而不是「失败」。

## 7. 文件正文为什么不能自动下载

Canvas 的文件下载 URL 通常带有时效性令牌，且需要会话 Cookie。
在不提取 Cookie 的前提下，Rust 侧无法独立发起下载；当前实现按用户授权改为读取会话 Cookie（见上）。

因此：

- `canvas_integration_info.file_download_supported = false`；
- 文件只同步**元数据**（名称、大小、类型、更新时间）；
- 界面引导用户把课件手动下载后，通过「导入课件」加入本地库；
- 这样既遵守安全约束，也保证离线阅读与检索可用。

## 8. 断线与过期

- 会话过期表现为 401：状态置为 `session-expired`，提示重新登录，已同步的本地数据不受影响；
- 网络不可用：同步进入 `retry_pending`，界面显示离线提示条；
- 退出/断开：`disconnect_canvas` 清空该窗口的浏览数据并把能力重置为 `not-tested`。
