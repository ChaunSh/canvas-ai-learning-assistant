-- Canvas course files.
--
-- Canvas is the source of truth for what exists in a course; this table is the
-- local mirror plus the download state machine. A row starts as `pending` when
-- a sync lists it, becomes `downloading` while the webview streams it to disk,
-- and ends `downloaded` (with the material it produced) or `failed`.
--
-- The download URL is a capability URL: it only works for the signed-in webview
-- session. It is stored so a retry does not need a fresh sync, and it is never
-- sent to the frontend.

CREATE TABLE IF NOT EXISTS canvas_files (
    id                TEXT PRIMARY KEY,
    course_id         TEXT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    canvas_file_id    TEXT NOT NULL,
    display_name      TEXT NOT NULL,
    content_type      TEXT,
    byte_size         INTEGER,
    download_url      TEXT NOT NULL,
    remote_updated_at TEXT,
    state             TEXT NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'downloading', 'downloaded', 'failed', 'unsupported')),
    material_id       TEXT REFERENCES materials (id) ON DELETE SET NULL,
    error             TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (course_id, canvas_file_id)
);

CREATE INDEX IF NOT EXISTS idx_canvas_files_state ON canvas_files (course_id, state);
