-- ============================================================================
-- 001_init.sql — Canvas AI Learning Assistant initial schema
--
-- Conventions
--   * ids            TEXT PRIMARY KEY holding a UUID v4 string.
--   * timestamps     TEXT ISO-8601 UTC with a trailing Z, e.g. 2026-10-17T12:00:00Z
--                    (lexicographically sortable, unambiguous, JS-friendly).
--   * booleans       INTEGER 0/1 with CHECK constraints.
--   * external ids   Always TEXT. Canvas ids are 64-bit and MUST NOT be parsed
--                    as JS numbers; keeping them as strings preserves precision.
--   * every table    carries created_at / updated_at.
--
-- Source separation is explicit on every synced entity through `source`:
--   canvas | manual | import | ai  (see 43.1 in the task specification)
-- ============================================================================

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- Application settings (small key/value store for UI + behaviour preferences)
-- ---------------------------------------------------------------------------
CREATE TABLE app_settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Courses
-- ---------------------------------------------------------------------------
CREATE TABLE courses (
    id              TEXT PRIMARY KEY,
    external_id     TEXT,
    code            TEXT,
    name            TEXT NOT NULL,
    term            TEXT,
    instructor      TEXT,
    description     TEXT,
    source          TEXT NOT NULL DEFAULT 'manual'
                    CHECK (source IN ('canvas', 'manual', 'import')),
    sync_status     TEXT NOT NULL DEFAULT 'local'
                    CHECK (sync_status IN ('local', 'synced', 'stale', 'error')),
    archived        INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    color           TEXT,
    position        INTEGER NOT NULL DEFAULT 0,
    last_synced_at  TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (source, external_id)
);

CREATE INDEX idx_courses_archived ON courses (archived, position);
CREATE INDEX idx_courses_term ON courses (term);

CREATE TABLE course_modules (
    id              TEXT PRIMARY KEY,
    course_id       TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id     TEXT,
    name            TEXT NOT NULL,
    position        INTEGER NOT NULL DEFAULT 0,
    unlock_at       TEXT,
    item_count      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

CREATE INDEX idx_course_modules_course ON course_modules (course_id, position);

-- ---------------------------------------------------------------------------
-- Materials (local files: Canvas downloads, manual imports, generated files)
-- ---------------------------------------------------------------------------
CREATE TABLE materials (
    id                  TEXT PRIMARY KEY,
    course_id           TEXT REFERENCES courses (id) ON DELETE SET NULL,
    module_id           TEXT REFERENCES course_modules (id) ON DELETE SET NULL,
    external_id         TEXT,
    title               TEXT NOT NULL,
    filename            TEXT NOT NULL,
    mime_type           TEXT,
    kind                TEXT NOT NULL DEFAULT 'other'
                        CHECK (kind IN ('pdf', 'pptx', 'docx', 'text', 'markdown',
                                        'image', 'ics', 'csv', 'json', 'zip', 'other')),
    byte_size           INTEGER NOT NULL DEFAULT 0,
    sha256              TEXT NOT NULL,
    storage_path        TEXT NOT NULL,
    source              TEXT NOT NULL DEFAULT 'import'
                        CHECK (source IN ('canvas', 'manual', 'import', 'ai')),
    remote_url          TEXT,
    -- Indexing pipeline state, see documents::pipeline
    index_status        TEXT NOT NULL DEFAULT 'pending'
                        CHECK (index_status IN ('pending', 'queued', 'processing',
                                                'indexed', 'partial', 'failed', 'skipped')),
    index_error         TEXT,
    page_count          INTEGER NOT NULL DEFAULT 0,
    slide_count         INTEGER NOT NULL DEFAULT 0,
    char_count          INTEGER NOT NULL DEFAULT 0,
    needs_ocr           INTEGER NOT NULL DEFAULT 0 CHECK (needs_ocr IN (0, 1)),
    is_favorite         INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
    tags                TEXT NOT NULL DEFAULT '[]',
    ai_upload_allowed   INTEGER NOT NULL DEFAULT 1 CHECK (ai_upload_allowed IN (0, 1)),
    last_opened_at      TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX idx_materials_course ON materials (course_id, created_at);
CREATE INDEX idx_materials_sha ON materials (sha256);
CREATE INDEX idx_materials_index_status ON materials (index_status);

-- Immutable-ish history when a material's bytes are replaced by a re-sync.
CREATE TABLE material_versions (
    id              TEXT PRIMARY KEY,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    sha256          TEXT NOT NULL,
    byte_size       INTEGER NOT NULL DEFAULT 0,
    storage_path    TEXT NOT NULL,
    note            TEXT,
    created_at      TEXT NOT NULL,
    UNIQUE (material_id, sha256)
);

-- ---------------------------------------------------------------------------
-- Coursework metadata
-- ---------------------------------------------------------------------------
CREATE TABLE assignments (
    id                  TEXT PRIMARY KEY,
    course_id           TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id         TEXT,
    title               TEXT NOT NULL,
    description         TEXT,
    due_at              TEXT,
    unlock_at           TEXT,
    points_possible     REAL,
    submission_status   TEXT NOT NULL DEFAULT 'unknown'
                        CHECK (submission_status IN ('unknown', 'not_submitted', 'submitted',
                                                     'graded', 'excused', 'pending')),
    html_url            TEXT,
    source              TEXT NOT NULL DEFAULT 'manual'
                        CHECK (source IN ('canvas', 'manual', 'import')),
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

CREATE INDEX idx_assignments_due ON assignments (due_at);
CREATE INDEX idx_assignments_course ON assignments (course_id, due_at);

CREATE TABLE announcements (
    id              TEXT PRIMARY KEY,
    course_id       TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id     TEXT,
    title           TEXT NOT NULL,
    body            TEXT,
    author          TEXT,
    posted_at       TEXT,
    html_url        TEXT,
    read            INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

CREATE INDEX idx_announcements_course ON announcements (course_id, posted_at);

CREATE TABLE calendar_events (
    id              TEXT PRIMARY KEY,
    course_id       TEXT REFERENCES courses (id) ON DELETE CASCADE,
    external_id     TEXT,
    ics_uid         TEXT,
    title           TEXT NOT NULL,
    description     TEXT,
    location        TEXT,
    starts_at       TEXT NOT NULL,
    ends_at         TEXT,
    all_day         INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
    kind            TEXT NOT NULL DEFAULT 'event'
                    CHECK (kind IN ('event', 'assignment_due', 'exam', 'study', 'personal')),
    source          TEXT NOT NULL DEFAULT 'manual'
                    CHECK (source IN ('canvas', 'manual', 'import', 'ai')),
    assignable_id   TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_calendar_starts ON calendar_events (starts_at);
CREATE UNIQUE INDEX idx_calendar_ics_uid ON calendar_events (ics_uid) WHERE ics_uid IS NOT NULL;

CREATE TABLE grade_records (
    id                TEXT PRIMARY KEY,
    course_id         TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id       TEXT,
    assignment_name   TEXT NOT NULL,
    score             REAL,
    points_possible   REAL,
    grade             TEXT,
    feedback          TEXT,
    posted_at         TEXT,
    -- 'canvas' = read from Canvas, 'manual' = user estimate, never labelled official
    source            TEXT NOT NULL DEFAULT 'canvas'
                      CHECK (source IN ('canvas', 'manual', 'import')),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

CREATE INDEX idx_grades_course ON grade_records (course_id);

CREATE TABLE discussion_topics (
    id              TEXT PRIMARY KEY,
    course_id       TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id     TEXT,
    title           TEXT NOT NULL,
    body            TEXT,
    author          TEXT,
    posted_at       TEXT,
    reply_count     INTEGER NOT NULL DEFAULT 0,
    html_url        TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

CREATE INDEX idx_discussions_course ON discussion_topics (course_id, posted_at);

CREATE TABLE discussion_entries (
    id              TEXT PRIMARY KEY,
    topic_id        TEXT NOT NULL REFERENCES discussion_topics (id) ON DELETE CASCADE,
    external_id     TEXT,
    parent_id       TEXT REFERENCES discussion_entries (id) ON DELETE CASCADE,
    author          TEXT,
    body            TEXT,
    posted_at       TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (topic_id, external_id)
);

-- Canvas Pages (course pages / wiki) metadata
CREATE TABLE course_pages (
    id              TEXT PRIMARY KEY,
    course_id       TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    external_id     TEXT,
    title           TEXT NOT NULL,
    body            TEXT,
    url             TEXT,
    updated_at_remote TEXT,
    html_url        TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (course_id, external_id)
);

-- ---------------------------------------------------------------------------
-- Document indexing
-- ---------------------------------------------------------------------------
CREATE TABLE document_processing_jobs (
    id              TEXT PRIMARY KEY,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    stage           TEXT NOT NULL DEFAULT 'queued',
    progress        REAL NOT NULL DEFAULT 0,
    attempts        INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_jobs_material ON document_processing_jobs (material_id, created_at);
CREATE INDEX idx_jobs_status ON document_processing_jobs (status);

CREATE TABLE document_pages (
    id              TEXT PRIMARY KEY,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    page_index      INTEGER NOT NULL,          -- 0-based
    label           TEXT,                      -- human label, e.g. "第 3 页" / "Slide 7"
    width           REAL,
    height          REAL,
    text            TEXT NOT NULL DEFAULT '',
    char_count      INTEGER NOT NULL DEFAULT 0,
    has_image       INTEGER NOT NULL DEFAULT 0 CHECK (has_image IN (0, 1)),
    created_at      TEXT NOT NULL,
    UNIQUE (material_id, page_index)
);

CREATE INDEX idx_pages_material ON document_pages (material_id, page_index);

CREATE TABLE document_chunks (
    id              TEXT PRIMARY KEY,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    page_index      INTEGER,                   -- 0-based page/slide the chunk starts on
    ordinal         INTEGER NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'paragraph'
                    CHECK (kind IN ('heading', 'paragraph', 'list', 'formula', 'definition',
                                    'theorem', 'code', 'table', 'caption', 'image')),
    section         TEXT,
    content         TEXT NOT NULL,
    context_prefix  TEXT,                      -- breadcrumb such as "第2章 › 2.3 导数"
    token_estimate  INTEGER NOT NULL DEFAULT 0,
    start_char      INTEGER NOT NULL DEFAULT 0,
    end_char        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    UNIQUE (material_id, ordinal)
);

CREATE INDEX idx_chunks_material ON document_chunks (material_id, ordinal);

-- Replaceable vector index: vectors live as little-endian f32 BLOBs and are
-- searched by the Rust `VectorIndex` implementation (see retrieval::vector).
CREATE TABLE chunk_embeddings (
    chunk_id        TEXT PRIMARY KEY REFERENCES document_chunks (id) ON DELETE CASCADE,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    model           TEXT NOT NULL,
    dim             INTEGER NOT NULL,
    vector          BLOB NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_embeddings_material ON chunk_embeddings (material_id);
CREATE INDEX idx_embeddings_model ON chunk_embeddings (model);

CREATE TABLE document_summaries (
    id              TEXT PRIMARY KEY,
    material_id     TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    kind            TEXT NOT NULL
                    CHECK (kind IN ('full', 'section', 'concepts', 'definitions',
                                    'formulas', 'theorems', 'pitfalls', 'review', 'self_test')),
    scope           TEXT NOT NULL DEFAULT '',   -- section name / page range when scoped
    content_md      TEXT NOT NULL,
    provider_id     TEXT,
    model           TEXT,
    source_sha256   TEXT,
    stale           INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0, 1)),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    -- Empty string (not NULL) for unscoped summaries: SQLite treats NULLs as
    -- distinct in UNIQUE constraints, which would break the upsert.
    UNIQUE (material_id, kind, scope)
);

-- ---------------------------------------------------------------------------
-- AI
-- ---------------------------------------------------------------------------
-- NOTE: no API key column exists. Keys are stored through security::SecretStore
-- keyed by provider id (Windows Credential Manager / Android Keystore).
CREATE TABLE ai_providers (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    base_url            TEXT NOT NULL,
    chat_model          TEXT NOT NULL,
    vision_model        TEXT,
    embedding_model     TEXT,
    max_output_tokens   INTEGER,
    temperature         REAL,
    timeout_seconds     INTEGER NOT NULL DEFAULT 120,
    is_default          INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    last_tested_at      TEXT,
    last_test_ok        INTEGER,
    last_test_error     TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE ai_model_configs (
    id              TEXT PRIMARY KEY,
    provider_id     TEXT NOT NULL REFERENCES ai_providers (id) ON DELETE CASCADE,
    model_id        TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'chat'
                    CHECK (role IN ('chat', 'vision', 'embedding')),
    display_name    TEXT,
    context_window  INTEGER,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_model_configs_provider ON ai_model_configs (provider_id, role);

CREATE TABLE chat_sessions (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    provider_id     TEXT REFERENCES ai_providers (id) ON DELETE SET NULL,
    model           TEXT,
    -- retrieval scope: what the assistant is allowed to look at
    scope_kind      TEXT NOT NULL DEFAULT 'course'
                    CHECK (scope_kind IN ('document', 'course', 'courses', 'all', 'none')),
    scope_course_ids TEXT NOT NULL DEFAULT '[]',
    scope_material_ids TEXT NOT NULL DEFAULT '[]',
    archived        INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_chat_sessions_updated ON chat_sessions (updated_at DESC);

CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES chat_sessions (id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant')),
    content         TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'complete'
                    CHECK (status IN ('streaming', 'complete', 'stopped', 'error')),
    error           TEXT,
    retrieval_query TEXT,
    prompt_tokens   INTEGER,
    completion_tokens INTEGER,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_chat_messages_session ON chat_messages (session_id, created_at);

CREATE TABLE chat_citations (
    id              TEXT PRIMARY KEY,
    message_id      TEXT NOT NULL REFERENCES chat_messages (id) ON DELETE CASCADE,
    document_id     TEXT NOT NULL,
    chunk_id        TEXT NOT NULL,
    course_id       TEXT,
    filename        TEXT NOT NULL,
    page            INTEGER,        -- 1-based page number as cited to the user
    slide           INTEGER,        -- 1-based slide number
    section         TEXT,
    excerpt         TEXT NOT NULL,
    source_type     TEXT NOT NULL,
    score           REAL,
    ordinal         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_citations_message ON chat_citations (message_id, ordinal);

CREATE TABLE ai_usage_records (
    id                  TEXT PRIMARY KEY,
    provider_id         TEXT,
    model               TEXT,
    purpose             TEXT NOT NULL
                        CHECK (purpose IN ('chat', 'summary', 'embedding', 'plan', 'vision', 'test')),
    prompt_tokens       INTEGER,
    completion_tokens   INTEGER,
    latency_ms          INTEGER,
    ok                  INTEGER NOT NULL DEFAULT 1 CHECK (ok IN (0, 1)),
    error               TEXT,
    created_at          TEXT NOT NULL
);

CREATE INDEX idx_usage_created ON ai_usage_records (created_at DESC);

-- ---------------------------------------------------------------------------
-- Study planning
-- ---------------------------------------------------------------------------
CREATE TABLE study_plans (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'weekly'
                    CHECK (kind IN ('daily', 'weekly', 'exam', 'assignment', 'custom')),
    start_date      TEXT NOT NULL,     -- YYYY-MM-DD (local calendar date)
    end_date        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('draft', 'active', 'archived')),
    notes           TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_plans_range ON study_plans (start_date, end_date);

CREATE TABLE study_tasks (
    id                  TEXT PRIMARY KEY,
    plan_id             TEXT REFERENCES study_plans (id) ON DELETE CASCADE,
    course_id           TEXT REFERENCES courses (id) ON DELETE CASCADE,
    material_id         TEXT REFERENCES materials (id) ON DELETE SET NULL,
    assignment_id       TEXT REFERENCES assignments (id) ON DELETE SET NULL,
    title               TEXT NOT NULL,
    description         TEXT,
    estimated_minutes   INTEGER NOT NULL DEFAULT 30,
    scheduled_start     TEXT,          -- ISO-8601 UTC
    scheduled_end       TEXT,
    deadline            TEXT,          -- ISO-8601 UTC
    priority            INTEGER NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
    status              TEXT NOT NULL DEFAULT 'todo'
                        CHECK (status IN ('todo', 'in_progress', 'done', 'skipped')),
    locked              INTEGER NOT NULL DEFAULT 0 CHECK (locked IN (0, 1)),
    generated_by        TEXT NOT NULL DEFAULT 'user'
                        CHECK (generated_by IN ('ai', 'rule', 'user')),
    source_references   TEXT NOT NULL DEFAULT '[]',
    completed_at        TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX idx_tasks_plan ON study_tasks (plan_id, scheduled_start);
CREATE INDEX idx_tasks_scheduled ON study_tasks (scheduled_start);
CREATE INDEX idx_tasks_status ON study_tasks (status, deadline);

CREATE TABLE study_preferences (
    id                  TEXT PRIMARY KEY,
    daily_minutes       INTEGER NOT NULL DEFAULT 120,
    session_minutes     INTEGER NOT NULL DEFAULT 50,
    break_minutes       INTEGER NOT NULL DEFAULT 10,
    preferred_start     TEXT NOT NULL DEFAULT '19:00',
    preferred_end       TEXT NOT NULL DEFAULT '22:30',
    study_days          TEXT NOT NULL DEFAULT '[1,2,3,4,5,6,7]',
    timezone            TEXT NOT NULL DEFAULT 'Asia/Shanghai',
    avoid_weekends      INTEGER NOT NULL DEFAULT 0 CHECK (avoid_weekends IN (0, 1)),
    updated_at          TEXT NOT NULL
);

CREATE TABLE study_progress (
    id                  TEXT PRIMARY KEY,
    course_id           TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    material_id         TEXT REFERENCES materials (id) ON DELETE CASCADE,
    pages_viewed        INTEGER NOT NULL DEFAULT 0,
    minutes_studied     INTEGER NOT NULL DEFAULT 0,
    last_position       TEXT,
    completion          REAL NOT NULL DEFAULT 0,
    updated_at          TEXT NOT NULL,
    UNIQUE (course_id, material_id)
);

CREATE TABLE study_sessions (
    id              TEXT PRIMARY KEY,
    task_id         TEXT REFERENCES study_tasks (id) ON DELETE SET NULL,
    course_id       TEXT REFERENCES courses (id) ON DELETE SET NULL,
    started_at      TEXT NOT NULL,
    ended_at        TEXT,
    minutes         INTEGER NOT NULL DEFAULT 0,
    notes           TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_study_sessions_started ON study_sessions (started_at DESC);

-- ---------------------------------------------------------------------------
-- Canvas / integration
-- ---------------------------------------------------------------------------
CREATE TABLE integration_connections (
    id                  TEXT PRIMARY KEY,
    provider            TEXT NOT NULL CHECK (provider IN ('canvas')),
    base_url            TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'not-connected',
    display_name        TEXT,
    user_ref            TEXT,        -- non-secret display hint, e.g. "已登录用户"
    last_checked_at     TEXT,
    last_verified_at    TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE integration_capabilities (
    id              TEXT PRIMARY KEY,
    connection_id   TEXT NOT NULL REFERENCES integration_connections (id) ON DELETE CASCADE,
    key             TEXT NOT NULL,     -- courses | modules | pages | files | ...
    status          TEXT NOT NULL DEFAULT 'not-tested'
                    CHECK (status IN ('supported', 'unsupported', 'permission-denied',
                                      'login-required', 'not-tested', 'temporarily-unavailable')),
    detail          TEXT,
    checked_at      TEXT,
    UNIQUE (connection_id, key)
);

CREATE TABLE sync_runs (
    id              TEXT PRIMARY KEY,
    connection_id   TEXT REFERENCES integration_connections (id) ON DELETE CASCADE,
    scope           TEXT NOT NULL DEFAULT 'all',
    course_id       TEXT,
    status          TEXT NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'completed', 'failed',
                                      'cancelled', 'retry_pending')),
    stats           TEXT NOT NULL DEFAULT '{}',
    error           TEXT,
    started_at      TEXT,
    finished_at     TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_sync_runs_created ON sync_runs (created_at DESC);

CREATE TABLE sync_checkpoints (
    id              TEXT PRIMARY KEY,
    sync_run_id     TEXT NOT NULL REFERENCES sync_runs (id) ON DELETE CASCADE,
    resource        TEXT NOT NULL,
    cursor          TEXT,
    updated_at      TEXT NOT NULL,
    UNIQUE (sync_run_id, resource)
);

CREATE TABLE sync_errors (
    id              TEXT PRIMARY KEY,
    sync_run_id     TEXT REFERENCES sync_runs (id) ON DELETE CASCADE,
    resource        TEXT,
    message         TEXT NOT NULL,
    retryable       INTEGER NOT NULL DEFAULT 0 CHECK (retryable IN (0, 1)),
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_sync_errors_run ON sync_errors (sync_run_id);

CREATE TABLE resource_mappings (
    id              TEXT PRIMARY KEY,
    local_type      TEXT NOT NULL,
    local_id        TEXT NOT NULL,
    remote_type     TEXT NOT NULL,
    remote_id       TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (local_type, local_id, remote_type, remote_id)
);

CREATE INDEX idx_mappings_remote ON resource_mappings (remote_type, remote_id);

CREATE TABLE import_jobs (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL,     -- material | ics | csv | json | zip | backup
    status          TEXT NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    source_path     TEXT,
    progress        REAL NOT NULL DEFAULT 0,
    total           INTEGER NOT NULL DEFAULT 0,
    processed       INTEGER NOT NULL DEFAULT 0,
    message         TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Full-text search over document chunks
--
-- SQLite's unicode61 tokenizer does not segment Chinese text, so the Rust
-- indexing layer writes a pre-tokenized copy of each chunk here (Latin words
-- lowercased, CJK runs expanded to overlapping bigrams). Queries are tokenized
-- the same way, which makes BM25 ranking meaningful for mixed zh/en content.
-- ---------------------------------------------------------------------------
CREATE VIRTUAL TABLE chunk_fts USING fts5 (
    chunk_id    UNINDEXED,
    material_id UNINDEXED,
    course_id   UNINDEXED,
    page_index  UNINDEXED,
    index_text,
    tokenize = 'unicode61 remove_diacritics 2'
);
