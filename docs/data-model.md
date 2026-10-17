# 数据模型

数据库：SQLite（单文件），位于

- Windows：`%APPDATA%\com.hkustgz.canvasai\canvas-ai.db`
- Android：应用私有目录（`<app data>/canvas-ai.db`）

迁移：`src-tauri/migrations/*.sql`，用 `PRAGMA user_version` 顺序执行，启动时自动完成。
当前版本：**3**（`001_init.sql` 建表，`002_citation_page_kind.sql` 给引用加页码来源列，`003_canvas_files.sql` 增加 Canvas 文件清单与下载状态）。

时间约定：**所有时间戳为 ISO-8601 UTC（带 `Z`）**；
学习计划的**日期边界**用本地日历日 `YYYY-MM-DD`；默认时区 `Asia/Shanghai`。

---

## 1. 实体关系（简化）

```
courses ─┬─ course_modules
         ├─ materials ─┬─ material_versions
         │             ├─ document_pages
         │             ├─ document_chunks ─┬─ chunk_embeddings
         │             │                   └─ chunk_fts (虚表)
         │             └─ document_summaries
         ├─ assignments
         ├─ announcements
         ├─ grade_records
         ├─ discussion_topics ── discussion_entries
         ├─ course_pages
         └─ calendar_events

ai_providers ── ai_model_configs
courses ── canvas_files ──> materials (下载后)
chat_sessions ── chat_messages ── chat_citations ──> document_chunks
study_plans ── study_tasks
study_preferences / study_progress / study_sessions
integration_connections ── integration_capabilities / sync_runs ── sync_errors
sync_checkpoints / resource_mappings
document_processing_jobs / import_jobs
app_settings
```

---

## 2. 表说明

### 2.1 课程与课程内容

| 表 | 关键列 | 说明 |
| --- | --- | --- |
| `courses` | `id`, `external_id`, `name`, `code`, `term`, `instructor`, `source`, `sync_status`, `archived`, `position` | `external_id` 仅 Canvas 来源有值；`(source, external_id)` 唯一 |
| `course_modules` | `course_id`, `external_id`, `name`, `position`, `unlock_at` | 章节 |
| `assignments` | `course_id`, `external_id`, `title`, `due_at`, `points_possible`, `submission_status` | 作业；`due_at` 是学习计划的主要输入 |
| `announcements` | `course_id`, `external_id`, `title`, `body`, `posted_at`, `read` | 公告（只读） |
| `grade_records` | `course_id`, `assignment_name`, `score`, `points_possible`, `grade`, `source` | 成绩；手动录入会标记 `source='manual'` |
| `discussion_topics` / `discussion_entries` | `course_id`, `external_id`, `title`, `reply_count` | 只读快照 |
| `course_pages` | `course_id`, `external_id`, `title`, `body`, `html_url` | 课程页面 |
| `calendar_events` | `course_id`, `title`, `starts_at`, `ends_at`, `all_day`, `kind`, `ics_uid`, `assignable_id` | `ics_uid` 用于 ICS 往返去重 |

### 2.2 课件与索引

| 表 | 关键列 | 说明 |
| --- | --- | --- |
| `materials` | `id`, `course_id`, `title`, `filename`, `mime_type`, `kind`, `byte_size`, `sha256`, `storage_path`, `index_status`, `page_count`, `slide_count`, `char_count`, `needs_ocr`, `is_favorite`, `ai_upload_allowed`, `last_opened_at` | `sha256` 唯一 → 内容去重；`index_status ∈ pending/queued/processing/indexed/partial/failed/skipped` |
| `material_versions` | `material_id`, `sha256`, `byte_size`, `storage_path`, `created_at` | 同一课件内容变更时保留历史 |
| `document_pages` | `material_id`, `page_index`, `label`, `width`, `height`, `text`, `char_count`, `has_image` | 真实分页；`has_image` 用于标记「需要视觉识别」 |
| `document_chunks` | `material_id`, `page_index`, `ordinal`, `kind`, `section`, `content`, `context_prefix`, `token_estimate`, `start_char`, `end_char` | 检索与引用的最小单位；`(material_id, ordinal)` 唯一 |
| `chunk_embeddings` | `chunk_id`, `model`, `dim`, `vector` | `vector` 为 L2 归一化后的 f32 小端 BLOB |
| `chunk_fts` | `chunk_id`, `material_id`, `course_id`, `page_index`, `index_text` | FTS5 虚表；`index_text` 是预分词结果 |
| `document_summaries` | `material_id`, `kind`, `scope`, `content_md`, `model`, `source_sha256`, `stale` | `scope` 用空串（不是 NULL）表示「整篇」，否则 UNIQUE 约束失效 |
| `document_processing_jobs` | `material_id`, `stage`, `status`, `error` | 解析/索引任务状态 |

**`chunk_fts` 的 `index_text` 如何生成**：拉丁字符转小写按词切分，CJK 连续段切成重叠二元组。
查询时用同一函数，因此索引与查询的 token 空间一致。

### 2.3 AI

| 表 | 关键列 | 说明 |
| --- | --- | --- |
| `ai_providers` | `id`, `name`, `base_url`, `chat_model`, `vision_model`, `embedding_model`, `temperature`, `max_output_tokens`, `timeout_seconds`, `is_default`, `last_test_*` | **没有 key 列**：密钥在系统凭据存储中 |
| `ai_model_configs` | `provider_id`, `capability`, `model`, `dim` | 按能力（chat/vision/embedding）选模型 |
| `chat_sessions` | `id`, `title`, `provider_id`, `model`, `scope_json`, `archived` | `scope` 记录检索范围 |
| `chat_messages` | `session_id`, `role`, `content`, `status`, `error`, `retrieval_query`, `prompt_tokens`, `completion_tokens` | `status ∈ streaming/complete/stopped/error` |
| `canvas_files` | `course_id`, `canvas_file_id`, `display_name`, `download_url`, `state`, `material_id` | Canvas 文件清单；`download_url` 只在后端使用，`state` 表示 pending / downloading / downloaded / failed |
| `chat_citations` | `message_id`, `document_id`, `chunk_id`, `filename`, `page`, `page_kind`, `slide`, `section`, `excerpt`, `score`, `ordinal` | 只保存模型**真实引用**的来源；`page_kind` 记录页码来源（`native` / `slide` / `generated`） |
| `ai_usage_records` | `provider_id`, `model`, `prompt_tokens`, `completion_tokens`, `ok`, `error` | 用量与失败统计 |

### 2.4 学习计划

| 表 | 关键列 | 说明 |
| --- | --- | --- |
| `study_preferences` | `daily_minutes`, `session_minutes`, `break_minutes`, `preferred_start`, `preferred_end`, `study_days`(CSV 1-7), `timezone`, `avoid_weekends` | 单行 |
| `study_plans` | `name`, `kind`, `start_date`, `end_date`, `status` | `start_date/end_date` 是本地日历日 |
| `study_tasks` | `plan_id`, `course_id`, `material_id`, `assignment_id`, `title`, `estimated_minutes`, `scheduled_start`, `scheduled_end`, `deadline`, `priority`, `status`, `locked`, `generated_by`, `source_references` | `locked=1` 表示用户固定，重排时不动；`generated_by ∈ ai/rule/user` |
| `study_progress` | `plan_id`, `date`, `completed_minutes` | 每日完成量 |
| `study_sessions` | `task_id`, `course_id`, `started_at`, `ended_at`, `minutes` | 计时器记录 |

### 2.5 集成与同步

| 表 | 关键列 | 说明 |
| --- | --- | --- |
| `integration_connections` | `provider`, `base_url`, `state`, `display_name`, `last_checked_at`, `last_verified_at` | `state ∈ not-connected/login-required/web-authenticated/data-access-verified/session-expired/unsupported/error` |
| `integration_capabilities` | `connection_id`, `key`, `status`, `detail`, `checked_at` | 每类数据一行 |
| `sync_runs` | `id`, `status`, `scope`, `course_id`, `stats_json`, `error`, `started_at`, `finished_at` | 一次同步一条 |
| `sync_checkpoints` | `scope`, `cursor`, `updated_at` | 增量同步游标 |
| `sync_errors` | `run_id`, `resource`, `message`, `retryable` | 单点失败不整体失败 |
| `resource_mappings` | `local_type`, `local_id`, `remote_type`, `remote_id` | 外部 ID ↔ 本地 ID |
| `import_jobs` | `kind`, `status`, `total`, `processed`, `errors_json` | 批量导入 |

### 2.6 其他

| 表 | 说明 |
| --- | --- |
| `app_settings` | 键值对（主题、语言、隐私开关、引导完成标记） |

---

## 3. 索引与约束（要点）

- `materials.sha256` 唯一：内容寻址，天然去重；
- `document_chunks(material_id, ordinal)` 唯一：重排 chunk 时先删后插；
- `document_summaries(material_id, kind, scope)` 唯一 + `scope NOT NULL DEFAULT ''`：
  用空串而不是 NULL 表示「整篇」，否则 SQLite 的 NULL 互不相等会让 upsert 变成插入；
- `calendar_events(ics_uid)` 唯一（非空时）：ICS 往返不产生重复；
- 所有外键带 `ON DELETE CASCADE`，删除课程会级联清理其内容；
- 时间列统一 `TEXT`（ISO-8601），便于跨平台与字符串比较。

## 4. 备份与恢复

- `backup_database` 使用 `VACUUM INTO` 生成一致快照（无需停写）；
- 备份文件是一个完整的 SQLite 文件，可直接用 `sqlite3` 打开；
- 集成测试验证「备份 → 重新打开 → 数据仍可读」；
- 恢复：关闭应用，用备份文件替换 `canvas-ai.db`（界面目前不提供一键恢复，
  因为热替换存在半写入风险）。
