# 测试说明

## 1. 总览

| 层次 | 工具 | 数量 | 位置 |
| --- | --- | --- | --- |
| Rust 单元测试 | `cargo test --lib` | 185 | 各模块内 `#[cfg(test)]` |
| Rust 集成测试 | `cargo test --test integration` | 8 | `src-tauri/tests/integration.rs` |
| AI 传输测试 | `cargo test --test ai_transport` | 7 | `src-tauri/tests/ai_transport.rs`（真实 TCP + SSE） |
| 前端类型检查 | `tsc --noEmit` | — | 全量 `src/` |
| 前端构建 | `vite build` | — | 产出 `dist/` |
| 前端测试 | `vitest` | 13 | `src/tests/*.test.ts` |
| 应用冒烟 | WebView2 CDP 场景回放 | 3 个场景 | 见第 5 节 |
| 安装包验证 | NSIS 静默安装/运行/卸载 | 1 轮 | 见第 6 节 |

当前结果（2026-10-17，Windows 11 / GNU 工具链）：

```
lib        : 185 passed
integration: 8 passed
ai_transport: 7 passed
vitest     : 13 passed (2 files)
tsc        : 0 errors
vite       : built in ~0.9s
cargo check --all-targets: 0 warnings
```

> `cargo test` 链接阶段会出现一条无害警告：
> `ld.exe: .rsrc merge failure: multiple non-default manifests`。
> 原因是我们为测试目标额外嵌入了一个 Common Controls v6 清单，而二进制本身
> 已由 `tauri-winres` 嵌入一个；GNU ld 合并资源时报告重复。
> 两个清单内容兼容，产物可正常运行（安装包启动/运行已实测）。

## 2. 运行方式

```powershell
. .\scripts\env.ps1

# Rust
cd src-tauri
cargo test

# 前端
cd ..
npx tsc --noEmit
npx vitest run
npx vite build
```

一条命令跑完全部门禁（类型检查 + 前端测试 + Rust 测试 + 警告检查 + 生产构建）：

```powershell
powershell -File scripts/verify.ps1
# 跳过打包（更快）：
powershell -File scripts/verify.ps1 -SkipBundle
```

产物验收（安装 → 运行 → 卸载，以及 APK 内的 ABI/签名检查）：

```powershell
powershell -File scripts/verify-artifacts.ps1
```

### 关于 `cargo test` 的清单问题

Windows GNU 目标下，测试程序会因为缺少 Common Controls v6 清单而在启动时报
`STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)` —— 因为 `tao` 导入了只有 v6 才有的
`TaskDialogIndirect`。`build.rs` 会编译一个清单资源并通过 `cargo::rustc-link-arg`
链接到本包的所有可链接目标，因此测试可以正常启动。

> 为什么不是 `rustc-link-arg-tests`：Cargo 的 `-tests` 只作用于 `[[test]]` 目标，
> 不会作用于**库自身的单元测试 harness**，而后者同样需要该清单。

同时 `scripts/stage-webview2.ps1` 会把 `WebView2Loader.dll` 复制到
`deps\`，否则测试程序也会因为找不到 DLL 而退出。

## 3. 覆盖内容

### 3.1 文档解析

- PDF：`ToUnicode` CMap 的 `bfchar` 成对映射、`bfrange`、十六进制字符串解码、
  连字换行合并（`deriva-\ntion` → `derivation`）、空白折叠；
- DOCX：标题层级（含自闭合 `<w:pStyle/>`）、表格单元格聚合、实体解码
  （`GeneralRef` 事件，`&amp;` / `&lt;`）；
- PPTX：`slideN.xml` 序号、实体解码、段落拼接不过度 trim；
- Markdown/文本：按标题与行切分；
- ICS：渲染与解析往返保持 UTC 瞬时值。

### 3.2 分块与分词

- 标题层级作为 `section` 面包屑；
- 超长段落按目标 token 切分且不切断 CJK 二元组边界；
- 中文 bigram 分词的**索引与查询一致性**（这是检索能否命中的关键）。

### 3.3 检索

- BM25 命中、AND 无结果时回退 OR 的策略标记；
- 向量检索按余弦排序 + 课程/材料范围过滤；
- RRF 融合去重与同课件多样性上限。

### 3.4 存储

- 迁移幂等（重复打开不重复执行）；
- 课件去重（SHA-256）；
- 摘要 upsert（`scope` 空串而非 NULL 的唯一约束行为）；
- 删除级联、索引统计；
- 备份可重新打开并读到数据（`VACUUM INTO`）。

### 3.5 学习计划调度

截止时间优先级、时间窗边界、休息间隔、跨天拆分、锁定任务不被移动、
容量不足时给出未排期原因、无效时间窗被拒绝。

### 3.6 AI 提示词与引用

- 证据块包裹格式；
- 只提取真实存在的 `[S n]`，丢弃模型编造编号；
- 无证据时提示词要求模型说明「资料未覆盖」。

### 3.7 安全

- ZIP 路径穿越（`..`、绝对路径、盘符）被拒绝；
- 导入路径校验；
- 密钥不落库（`ai_providers` 无 key 列，由 schema 保证）。

## 4. 集成测试清单（`src-tauri/tests/integration.rs`）

| 测试 | 验证点 |
| --- | --- |
| `end_to_end_markdown_import_and_retrieval` | 建课程 → 导入 Markdown → 解析分块 → 写 FTS → 中文检索命中；断言 Markdown **不产生页码**，只有章节 |
| `docx_is_parsed_with_sections_and_searchable` | 真实 DOCX（ZIP + OOXML）解析出中文段落，可检索，**不编造页码** |
| `duplicate_import_is_detected_by_hash` | 相同字节的两个文件路径只入库一次 |
| `zip_import_expands_and_rejects_traversal` | 只导入安全条目，`../escape.md` 被拒绝且未落盘 |
| `database_backup_produces_a_reopenable_snapshot` | 备份文件可独立打开并读到课程 |
| `calendar_ics_roundtrip_keeps_utc_instants` | ICS 往返后 `starts_at` 仍是同一 UTC 瞬时 |
| `material_filtering_and_deletion_cleanup` | 列表统计、删除后 FTS/向量/行被清理 |
| `real_pdf_reports_true_page_numbers` | **真实 LaTeX PDF**：中文版（ToUnicode CMap）与英文版（Type1 无 CMap）各 2 页；断言页码真实、章节不跨页泄漏 |

### 4.1 关于 PDF 测试数据

`fixtures/pdf/` 下是**用真实 LaTeX 编译出来的 PDF**（源文件 `.tex` 一并保留，可复现）：

- `sample-zh.pdf`：`xelatex` + `ctexart`，带完整 `ToUnicode` CMap，覆盖 CJK 提取的正确路径；
- `sample-en.pdf`：`pdflatex`，Type1 字体、无 `ToUnicode`，覆盖回退编码路径。

测试断言的是**语义**（"第 1 页含导数的定义"、"第 2 页含链式法则"、"两页互不污染"），
而不是字节或布局，因此对解析器的实现细节不过度耦合。

> 为什么这很重要：页码是引用的可信度基础。只有在真实分页 PDF 上验证过
> 「第 N 页确实是那一段」，才能说引用页码是真的。

这些测试全部使用**运行时生成的真实文件**（ZIP/OOXML/ICS 由代码构造），
不依赖模拟数据，也不会把假数据当成真实课件。

### 4.2 AI 传输层（`tests/ai_transport.rs`）

不 mock 客户端：测试在回环地址上起一个最小 HTTP/1.1 服务器，真实
`OpenAiClient`（reqwest + rustls + serde）通过真实 socket 与之通信，覆盖单测无法触达的部分：

| 测试 | 验证点 |
| --- | --- |
| `streaming_completion_collects_deltas_usage_and_finish_reason` | SSE 按事件边界切分（响应被故意拆成多次 write）、增量回调顺序、`usage` 与 `finish_reason` 落点 |
| `streaming_stops_early_when_the_callback_returns_false` | 取消路径：回调返回 `false` 后立即停止消费且不报错（对应界面「停止生成」） |
| `non_streaming_completion_returns_content_and_usage` | 非流式 JSON 解析 |
| `gateway_ignoring_stream_falls_back_to_json_body` | 网关忽略 `stream:true`、返回普通 JSON 时自动回退 |
| `models_endpoint_is_parsed_and_sorted` | `/models` 解析与排序 |
| `embeddings_endpoint_returns_one_vector_per_input` | `/embeddings` 与输入一一对应 |
| `unauthorized_response_becomes_an_actionable_message` | 401 映射为「认证失败 + API Key」的可操作提示，并保留上游 detail |

> 这覆盖的是**协议实现**，不是某个真实供应商。真实联网仍需填入 API Key，见第 7 节。

### 4.3 导出 / 导入（`commands::data::tests`）

| 测试 | 验证点 |
| --- | --- |
| `export_then_import_restores_courses_plans_and_tasks` | 导出 → JSON 文本 → 解析 → 导入到空库，课程/计划/任务及 `plan_id` 关联全部恢复 |
| `importing_the_same_export_twice_is_idempotent` | 二次导入全部记为 skipped，不产生重复 |
| `canvas_owned_courses_are_left_to_sync` | Canvas 来源课程不被导入；引用它的任务随之跳过而**不是**让整个导入失败 |
| `export_declares_its_format_and_never_contains_secrets` | 导出带格式/版本标记，且文本中不含 `api_key` / `sk-` / `bearer` 等密钥痕迹 |

> 第四个用例之外，第三个用例在首次运行时就暴露了一个真实缺陷：跳过课程后仍插入引用它的任务，
> 触发 `FOREIGN KEY constraint failed` 并**整体回滚**导入。现已改为按条跳过并计入报告。

### 4.4 AI 总结（`ai::tests`）

总结的**来源组装**被抽成纯函数 `compose_summary_source(chunks, scope, budget)`，因为它决定
「总结到底看的是哪段内容」——这类错误在真实调用里很难被发现（模型会自信地总结错内容）：

| 测试 | 验证点 |
| --- | --- |
| `summary_source_includes_context_and_section_headers` | 上下文前缀与章节标题进入来源文本 |
| `summary_source_honours_the_section_scope` | 只取指定章节；未知章节返回空（而不是静默总结全部） |
| `summary_source_stops_at_the_token_budget` | 超预算即停止，且不回填后面更小的片段（保持阅读顺序） |
| `summary_source_with_zero_budget_is_empty` | 预算为 0 / 无内容时返回空，由调用方给出明确错误 |
| `summary_instructions_differ_per_kind_and_language` | 9 种类型 × 2 语言的提示词互不相同，未知类型回退到完整总结 |
### 4.5 页码来源（i::tests + etrieval::tests）

| 测试 | 验证点 |
| --- | --- |
| `every_hit_carries_a_jumpable_page_with_honest_provenance` | PDF 片段 `page_kind = native`；DOCX 片段同样有页码但 `page_kind = generated`，不会冒充原文页码 |
| `generated_pages_are_labelled_as_generated` | 证据行写明「文档未分页，页码由应用按内容生成」 |
| `slides_keep_slide_wording` | PPTX 仍称「第 N 张幻灯片」 |
| `pdf_pages_stay_plain` | PDF 只称「第 N 页」，不带生成说明 |

> 这些测试锁定了这轮修改的核心承诺：**每个片段都能按页码跳转，但合成页码必须被标注**。

## 5. 应用冒烟验证（真实可执行文件）

方法：启动打包后的 exe，并给 WebView2 加上 `--remote-debugging-port=9222`，
用 CDP 观察真实 DOM 并驱动点击/输入。

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
& "D:\caa-target\debug\canvas-ai-assistant.exe"

# 另开终端列目标
Invoke-RestMethod "http://127.0.0.1:9222/json/list"
```

已执行的场景：

| 场景 | 观察结果 |
| --- | --- |
| 首次启动 | URL `#/welcome`，可访问性树含「欢迎使用 Canvas AI 学习助手」「连接 Canvas」「开始使用」；控制台无错误 |
| 点击「开始使用」 | 路由切到 `#/`；IPC `POST /settings_set` → 200、`POST /get_dashboard` → 200；首页统计卡片、空状态、侧边栏全部渲染 |
| 新建课程 | 导航 `#/courses`（`list_courses` → 200）→ 打开对话框 → 填入名称/代码 → `POST /create_course` → 200 → 列表刷新出现课程卡片 |

截图保存在 `artifacts/`。

> 这一步证明的是**前后端真实连通**（命令名、参数形状、序列化、SQLite 写入、
> 查询失效重取）都能跑通，而不是「窗口打开了」。

## 6. 安装包验证（Windows）

```powershell
# 构建
pnpm tauri:build

# 静默安装 → 验证 → 卸载
& "D:\caa-target\release\bundle\nsis\Canvas AI Learning Assistant_0.1.0_x64-setup.exe" /S
Test-Path "$env:LOCALAPPDATA\Canvas AI Learning Assistant\canvas-ai-assistant.exe"
& "$env:LOCALAPPDATA\Canvas AI Learning Assistant\uninstall.exe" /S
```

实测结果：

| 检查点 | 结果 |
| --- | --- |
| 安装包大小 | 4.93 MiB |
| 安装目录内容 | `canvas-ai-assistant.exe`(12.5 MB)、`uninstall.exe`、`WebView2Loader.dll` |
| 安装后启动 | 进程存活，日志 `database ready`，WebView2 目标标题为「Canvas AI 学习助手」 |
| 卸载 | 退出码 0，安装目录被删除，无残留卸载注册项 |
| 用户数据 | `%APPDATA%\com.hkustgz.canvasai\canvas-ai.db` 保留（卸载不删数据） |

> 注意：`WebView2Loader.dll` 必须与 exe 同级，否则启动即退出（`0xc0000139`）。
> 该 DLL 由打包流程自动带入；开发时用 `scripts/stage-webview2.ps1` 复制。

## 7. 明确未覆盖

1. **Canvas 取数路径**：无账号，未跑过任何真实请求；
2. **AI 流式回答**：无 API Key，未跑过真实供应商；
3. **PDF 在 WebView 中的渲染**：pdf.js 集成已编译进包，但未用真实 PDF 点击验证；
4. **文件导入对话框**：系统原生对话框无法被自动化点击，导入路径由集成测试覆盖；
5. **Android 运行时**：无 AVD/真机；
6. **安装包安装/卸载**：打包配置完整，尚未实际安装验证。

这些缺口都在 `docs/implementation-status.md` 中如实标注，不会因为「代码写了」
就标记为已支持。

## 8. 回归建议

每次改动后至少执行：

```powershell
. .\scripts\env.ps1
cd src-tauri; cargo test; cd ..
npx tsc --noEmit
npx vite build
```

改动解析器或检索逻辑时，务必同时跑集成测试 —— 它们覆盖了「入库 → 索引 → 命中」
这条最容易静默出错的链路。
