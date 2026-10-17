//! Canvas synchronisation.
//!
//! Every request is a read-only GET executed inside the logged-in Canvas page,
//! so the browser supplies the session. Nothing here talks to Canvas over the
//! native HTTP client.
//!
//! Capability rule: a dataset is only marked `supported` when the request
//! succeeded (HTTP 2xx). An empty array is a legitimate supported result; 401,
//! 403, 404 and 5xx map to distinct non-supported states so the UI never claims
//! more than actually worked (task specification §10.1).

use serde_json::Value;

use super::parse::{self, PAGE_SIZE};
use super::{capability_from_probe, CanvasRuntime};
use crate::domain::{
    CapabilityStatus, CanvasConnection, CanvasSessionState, SyncProgressEvent, SyncRun, SyncStats,
    CANVAS_BASE_URL,
};
use crate::error::{AppError, AppResult};
use crate::storage::{course_repo, coursework_repo, settings, sync_repo, Db};
use crate::time::now_iso;

pub const SYNC_EVENT: &str = "canvas://sync";
const MAX_PAGES: usize = 10;

pub struct CanvasSync<'a> {
    pub db: &'a Db,
    pub runtime: &'a CanvasRuntime,
    pub app: &'a tauri::AppHandle,
}

impl<'a> CanvasSync<'a> {
    pub fn new(db: &'a Db, runtime: &'a CanvasRuntime, app: &'a tauri::AppHandle) -> Self {
        Self { db, runtime, app }
    }

    fn connection_id(&self) -> AppResult<String> {
        let connection = self
            .db
            .with(|c| sync_repo::ensure_connection(c, CANVAS_BASE_URL))?;
        Ok(connection.id)
    }

    /// Verify login and data access, updating the state machine accordingly.
    pub async fn check_connection(&self) -> AppResult<CanvasConnection> {
        let connection_id = self.connection_id()?;

        let me = self.runtime.probe(self.app, "/api/v1/users/self", "登录状态").await?;
        if !me.ok {
            let state = match me.http_status {
                Some(401) | Some(403) => CanvasSessionState::LoginRequired,
                _ => CanvasSessionState::Error,
            };
            self.db.with(|c| sync_repo::set_state(c, &connection_id, state))?;
            let mut connection = self.db.with(|c| sync_repo::get_connection(c, &connection_id))?;
            connection.diagnostics.push(crate::domain::DiagnosticEntry {
                at: now_iso(),
                level: "warn".into(),
                message: me
                    .error
                    .clone()
                    .unwrap_or_else(|| "未能确认登录状态".into()),
            });
            return Ok(connection);
        }

        let (display_name, user_ref) = me
            .body
            .as_ref()
            .and_then(|v| serde_json::from_value::<parse::RawUser>(v.clone()).ok())
            .map(|user| {
                let name = user.name.or(user.short_name).unwrap_or_else(|| "已登录用户".into());
                // Only a non-secret display hint is persisted.
                (Some(name), Some(format!("canvas-user:{}", user.id)))
            })
            .unwrap_or((None, None));

        self.db.with(|c| {
            sync_repo::set_identity(c, &connection_id, display_name.as_deref(), user_ref.as_deref())?;
            sync_repo::set_state(c, &connection_id, CanvasSessionState::WebAuthenticated)
        })?;

        // Confirm actual data access with the smallest possible request.
        let courses = self
            .runtime
            .probe(self.app, "/api/v1/courses?per_page=1&enrollment_state=active", "课程数据")
            .await?;
        let status = capability_from_probe(&courses);
        self.db.with(|c| {
            sync_repo::set_capability(
                c,
                &connection_id,
                "courses",
                status,
                courses.error.as_deref(),
            )
        })?;

        let final_state = if status == CapabilityStatus::Supported {
            CanvasSessionState::DataAccessVerified
        } else {
            CanvasSessionState::WebAuthenticated
        };
        self.db.with(|c| sync_repo::set_state(c, &connection_id, final_state))?;

        let mut connection = self.db.with(|c| sync_repo::get_connection(c, &connection_id))?;
        connection.diagnostics.push(crate::domain::DiagnosticEntry {
            at: now_iso(),
            level: if status == CapabilityStatus::Supported {
                "info".to_string()
            } else {
                "warn".to_string()
            },
            message: match status {
                CapabilityStatus::Supported => "已确认可以读取课程数据".into(),
                _ => format!(
                    "网页已登录，但课程数据不可读：{}",
                    courses.error.clone().unwrap_or_else(|| "未知原因".into())
                ),
            },
        });
        Ok(connection)
    }

    /// Synchronise every supported dataset.
    pub async fn sync_all(&self) -> AppResult<SyncRun> {
        let connection_id = self.connection_id()?;
        let run_id = self.db.with(|c| {
            sync_repo::create_sync_run(c, &connection_id, "all", None)
        })?;
        let mut stats = SyncStats::default();

        let result = self.sync_inner(&connection_id, &run_id, &mut stats, None).await;

        match &result {
            Ok(()) => {
                self.db.with(|c| {
                    sync_repo::finish_sync_run(c, &run_id, "completed", &stats, None)?;
                    settings::set(c, settings::KEY_LAST_SYNC, &now_iso())
                })?;
                self.set_state_data_access(&connection_id)?;
            }
            Err(error) => {
                self.db.with(|c| {
                    sync_repo::add_sync_error(c, &run_id, None, &error.to_string(), true)?;
                    sync_repo::finish_sync_run(c, &run_id, "failed", &stats, Some(&error.to_string()))
                })?;
            }
        }

        self.db.with(|c| sync_repo::get_sync_run(c, &run_id))
    }

    /// Synchronise one course (plus metadata-only discovery).
    pub async fn sync_course(&self, local_course_id: &str) -> AppResult<SyncRun> {
        let connection_id = self.connection_id()?;
        let external_id = self
            .db
            .with(|c| course_repo::get(c, local_course_id))?
            .external_id
            .ok_or_else(|| AppError::invalid("该课程不是从 Canvas 同步的，无法同步"))?;

        let run_id = self.db.with(|c| {
            sync_repo::create_sync_run(c, &connection_id, "course", Some(local_course_id))
        })?;
        let mut stats = SyncStats::default();
        let result = self
            .sync_inner(&connection_id, &run_id, &mut stats, Some(&external_id))
            .await;

        match &result {
            Ok(()) => {
                self.db.with(|c| {
                    sync_repo::finish_sync_run(c, &run_id, "completed", &stats, None)
                })?;
            }
            Err(error) => {
                self.db.with(|c| {
                    sync_repo::add_sync_error(c, &run_id, None, &error.to_string(), true)?;
                    sync_repo::finish_sync_run(c, &run_id, "failed", &stats, Some(&error.to_string()))
                })?;
            }
        }
        self.db.with(|c| sync_repo::get_sync_run(c, &run_id))
    }

    async fn sync_inner(
        &self,
        connection_id: &str,
        run_id: &str,
        stats: &mut SyncStats,
        only_external_course: Option<&str>,
    ) -> AppResult<()> {
        self.emit(run_id, "courses", "courses", 0, 1, Some("正在读取课程列表"))?;

        let courses_path = "/api/v1/courses?per_page=100&enrollment_state=active&include[]=term&include[]=teachers";
        let (courses_value, courses_status) = self.fetch_paged(courses_path, "courses").await?;
        self.record_capability(connection_id, "courses", courses_status)?;

        if !matches!(courses_status, CapabilityStatus::Supported) {
            return Err(AppError::Canvas(format!(
                "无法读取课程列表（{}）。请在 Canvas 窗口确认登录状态。",
                courses_status.as_str()
            )));
        }

        let mut local_courses: Vec<(String, String)> = Vec::new();
        let course_values = parse::as_array(&courses_value);
        self.db.transaction(|tx| {
            for item in &course_values {
                let Ok(raw) = serde_json::from_value::<parse::RawCourse>(item.clone()) else {
                    continue;
                };
                if only_external_course.map(|id| id != raw.id).unwrap_or(false) {
                    continue;
                }
                let seed = course_repo::CourseUpsert {
                    external_id: raw.id.clone(),
                    code: raw.course_code.clone(),
                    name: raw.display_name(),
                    term: raw.term.as_ref().and_then(|t| t.name.clone()),
                    instructor: raw.instructor(),
                    description: raw.syllabus_body.clone(),
                };
                let (local_id, created) = course_repo::upsert_canvas(tx, &seed)?;
                if created {
                    stats.courses += 1;
                }
                local_courses.push((raw.id.clone(), local_id));
            }
            Ok(())
        })?;

        self.emit(
            run_id,
            "courses",
            "courses",
            local_courses.len() as i64,
            local_courses.len() as i64,
            Some("课程列表已更新"),
        )?;

        let total = local_courses.len();
        for (index, (external_id, local_id)) in local_courses.iter().enumerate() {
            self.emit(
                run_id,
                "course",
                "course",
                index as i64,
                total as i64,
                Some(&format!("正在同步 {} / {}", index + 1, total)),
            )?;
            if let Err(error) = self
                .sync_course_data(connection_id, external_id, local_id, stats)
                .await
            {
                tracing::warn!(course = %external_id, error = %error, "course sync partially failed");
                self.db.with(|c| {
                    sync_repo::add_sync_error(
                        c,
                        run_id,
                        Some(external_id),
                        &error.to_string(),
                        true,
                    )
                })?;
            }
        }

        Ok(())
    }

    async fn sync_course_data(
        &self,
        connection_id: &str,
        external_course: &str,
        local_course: &str,
        stats: &mut SyncStats,
    ) -> AppResult<()> {
        // --- modules -------------------------------------------------------
        let path = format!("/api/v1/courses/{external_course}/modules?per_page=100&include[]=items");
        let (value, status) = self.fetch_paged(&path, "modules").await?;
        self.record_capability(connection_id, "modules", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for (index, item) in items.iter().enumerate() {
                    let Ok(raw) = serde_json::from_value::<parse::RawModule>(item.clone()) else {
                        continue;
                    };
                    course_repo::upsert_canvas_module(
                        tx,
                        local_course,
                        &raw.id,
                        raw.name.as_deref().unwrap_or("未命名章节"),
                        raw.position.unwrap_or(index as i64 + 1),
                        raw.unlock_at.as_deref(),
                    )?;
                    stats.modules += 1;
                }
                Ok(())
            })?;
        }

        // --- assignments ---------------------------------------------------
        let path = format!(
            "/api/v1/courses/{external_course}/assignments?per_page=100&include[]=submission"
        );
        let (value, status) = self.fetch_paged(&path, "assignments").await?;
        self.record_capability(connection_id, "assignments", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for item in &items {
                    let Ok(raw) = serde_json::from_value::<parse::RawAssignment>(item.clone()) else {
                        continue;
                    };
                    coursework_repo::upsert_canvas_assignment(
                        tx,
                        &coursework_repo::AssignmentUpsert {
                            course_id: local_course,
                            external_id: &raw.id,
                            title: raw.name.as_deref().unwrap_or("未命名作业"),
                            description: raw.description.as_deref(),
                            due_at: raw.due_at.as_deref(),
                            unlock_at: raw.unlock_at.as_deref(),
                            points_possible: raw.points_possible,
                            submission_status: raw.submission_status(),
                            html_url: raw.html_url.as_deref(),
                        },
                    )?;
                    stats.assignments += 1;
                }
                Ok(())
            })?;
        }

        // --- announcements -------------------------------------------------
        let path = format!(
            "/api/v1/courses/{external_course}/discussion_topics?per_page=50&only_announcements=true"
        );
        let (value, status) = self.fetch_paged(&path, "announcements").await?;
        self.record_capability(connection_id, "announcements", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for item in &items {
                    let Ok(raw) = serde_json::from_value::<parse::RawAnnouncement>(item.clone()) else {
                        continue;
                    };
                    coursework_repo::upsert_canvas_announcement(
                        tx,
                        &coursework_repo::AnnouncementUpsert {
                            course_id: local_course,
                            external_id: &raw.id,
                            title: raw.title.as_deref().unwrap_or("未命名公告"),
                            body: raw.message.as_deref(),
                            author: raw.author.as_ref().and_then(|a| a.display_name.as_deref()),
                            posted_at: raw.posted_at.as_deref(),
                            html_url: raw.html_url.as_deref(),
                        },
                    )?;
                    stats.announcements += 1;
                }
                Ok(())
            })?;
        }

        // --- calendar ------------------------------------------------------
        let path = format!(
            "/api/v1/calendar_events?per_page=100&context_codes[]=course_{external_course}&type=event"
        );
        let (value, status) = self.fetch_paged(&path, "calendar").await?;
        self.record_capability(connection_id, "calendar", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for item in &items {
                    let Ok(raw) = serde_json::from_value::<parse::RawCalendarEvent>(item.clone()) else {
                        continue;
                    };
                    let Some(start) = raw.start_at.as_deref() else {
                        continue;
                    };
                    if crate::time::parse_iso(start).is_none() {
                        continue;
                    }
                    coursework_repo::upsert_canvas_event(
                        tx,
                        &coursework_repo::CanvasEventInput {
                            external_id: &raw.id,
                            course_id: Some(local_course),
                            title: raw.title.as_deref().unwrap_or("未命名日程"),
                            description: raw.description.as_deref(),
                            location: raw.location_name.as_deref(),
                            starts_at: start,
                            ends_at: raw.end_at.as_deref(),
                            all_day: raw.all_day.unwrap_or(false),
                            kind: raw.event_kind(),
                        },
                    )?;
                    stats.events += 1;
                }
                Ok(())
            })?;
        }

        // --- grades --------------------------------------------------------
        let path = format!(
            "/api/v1/courses/{external_course}/enrollments?per_page=10&user_id=self"
        );
        let (value, status) = self.fetch_paged(&path, "grades").await?;
        self.record_capability(connection_id, "grades", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            if let Some(first) = items.first() {
                if let Ok(raw) = serde_json::from_value::<parse::RawEnrollment>(first.clone()) {
                    let score = raw
                        .grades
                        .as_ref()
                        .and_then(|g| g.current_score)
                        .or(raw.computed_current_score);
                    let grade = raw
                        .grades
                        .as_ref()
                        .and_then(|g| g.current_grade.clone())
                        .or(raw.computed_current_grade.clone());
                    if score.is_some() || grade.is_some() {
                        let external = format!("enrollment:{}", external_course);
                        self.db.transaction(|tx| {
                            coursework_repo::upsert_canvas_grade(
                                tx,
                                &coursework_repo::GradeUpsert {
                                    course_id: local_course,
                                    external_id: &external,
                                    assignment_name: "课程总评（Canvas）",
                                    score,
                                    points_possible: Some(100.0),
                                    grade: grade.as_deref(),
                                    feedback: None,
                                    posted_at: Some(&now_iso()),
                                },
                            )?;
                            stats.grades += 1;
                            Ok(())
                        })?;
                    }
                }
            }
        }

        // --- discussions ---------------------------------------------------
        let path = format!("/api/v1/courses/{external_course}/discussion_topics?per_page=50");
        let (value, status) = self.fetch_paged(&path, "discussions").await?;
        self.record_capability(connection_id, "discussions", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for item in &items {
                    let Ok(raw) = serde_json::from_value::<parse::RawDiscussion>(item.clone()) else {
                        continue;
                    };
                    coursework_repo::upsert_canvas_topic(
                        tx,
                        &coursework_repo::TopicUpsert {
                            course_id: local_course,
                            external_id: &raw.id,
                            title: raw.title.as_deref().unwrap_or("未命名讨论"),
                            body: raw.message.as_deref(),
                            author: raw.user_name.as_deref(),
                            posted_at: raw.posted_at.as_deref(),
                            reply_count: raw.discussion_subentry_count.unwrap_or(0),
                            html_url: raw.html_url.as_deref(),
                        },
                    )?;
                    stats.discussions += 1;
                }
                Ok(())
            })?;
        }

        // --- pages ---------------------------------------------------------
        let path = format!("/api/v1/courses/{external_course}/pages?per_page=100");
        let (value, status) = self.fetch_paged(&path, "pages").await?;
        self.record_capability(connection_id, "pages", status)?;
        if status == CapabilityStatus::Supported {
            let items = parse::as_array(&value);
            self.db.transaction(|tx| {
                for item in &items {
                    let Ok(raw) = serde_json::from_value::<parse::RawPage>(item.clone()) else {
                        continue;
                    };
                    let external = raw.external_id();
                    if external.is_empty() {
                        continue;
                    }
                    let body = raw.body.as_deref().map(crate::documents::plain::html_to_text);
                    coursework_repo::upsert_canvas_page(
                        tx,
                        &coursework_repo::PageUpsert {
                            course_id: local_course,
                            external_id: &external,
                            title: raw.title.as_deref().unwrap_or("未命名页面"),
                            body: body.as_deref(),
                            url: raw.url.as_deref(),
                            updated_at_remote: raw.updated_at.as_deref(),
                            html_url: raw.html_url.as_deref(),
                        },
                    )?;
                    stats.pages += 1;
                }
                Ok(())
            })?;
        }

        // --- files (metadata only) -----------------------------------------
        let path = format!("/api/v1/courses/{external_course}/files?per_page=100");
        let (value, status) = self.fetch_paged(&path, "files").await?;
        self.record_capability(connection_id, "files", status)?;
        if status == CapabilityStatus::Supported {
            let listed = parse::as_array(&value);
            stats.files += listed.len() as i64;
            // Folder metadata gives the Files tab Canvas' own layout. A failure
            // here is not fatal: files simply land in the course root.
            let folders = match self
                .fetch_paged(
                    &format!("/api/v1/courses/{external_course}/folders?per_page=100"),
                    "folders",
                )
                .await
            {
                Ok((folder_value, _)) => {
                    crate::storage::canvas_file_repo::parse_folders(&parse::as_array(&folder_value))
                }
                Err(error) => {
                    tracing::debug!(%error, "canvas folders unavailable");
                    Default::default()
                }
            };
            // Mirror the listing locally; state (downloaded/failed) is owned by
            // the downloader and is never reset here.
            self.db.with(|c| {
                crate::storage::canvas_file_repo::upsert_files(c, local_course, &listed, &folders)
            })?;
        }

        self.db
            .with(|c| course_repo::mark_synced(c, local_course))?;
        Ok(())
    }

    /// Fetch a paginated collection, returning the concatenated JSON array.
    async fn fetch_paged(
        &self,
        base_path: &str,
        resource: &str,
    ) -> AppResult<(Value, CapabilityStatus)> {
        let mut collected: Vec<Value> = Vec::new();
        let mut status = CapabilityStatus::NotTested;

        for page in 1..=MAX_PAGES {
            let separator = if base_path.contains('?') { '&' } else { '?' };
            let path = format!("{base_path}{separator}page={page}");
            let probe = self
                .runtime
                .probe(self.app, &path, resource)
                .await?;

            if !probe.ok {
                let mapped = capability_from_probe(&probe);
                if page == 1 {
                    return Ok((Value::Array(Vec::new()), mapped));
                }
                tracing::warn!(resource, page, "pagination stopped on error");
                break;
            }

            status = CapabilityStatus::Supported;
            let items = probe
                .body
                .as_ref()
                .map(parse::as_array)
                .unwrap_or_default();
            let count = items.len();
            collected.extend(items);
            if count < PAGE_SIZE {
                break;
            }
        }

        Ok((Value::Array(collected), status))
    }

    fn record_capability(
        &self,
        connection_id: &str,
        key: &str,
        status: CapabilityStatus,
    ) -> AppResult<()> {
        if status == CapabilityStatus::NotTested {
            return Ok(());
        }
        self.db.with(|c| {
            sync_repo::set_capability(
                c,
                connection_id,
                key,
                status,
                Some(match status {
                    CapabilityStatus::Supported => "本次同步已成功读取",
                    CapabilityStatus::PermissionDenied => "当前账号无权访问",
                    CapabilityStatus::LoginRequired => "需要重新登录",
                    CapabilityStatus::Unsupported => "该实例未启用的接口",
                    CapabilityStatus::TemporarilyUnavailable => "服务暂时不可用",
                    CapabilityStatus::NotTested => "尚未测试",
                }),
            )
        })
    }

    fn set_state_data_access(&self, connection_id: &str) -> AppResult<()> {
        self.db.with(|c| {
            sync_repo::set_state(c, connection_id, CanvasSessionState::DataAccessVerified)
        })
    }

    fn emit(
        &self,
        run_id: &str,
        stage: &str,
        resource: &str,
        processed: i64,
        total: i64,
        message: Option<&str>,
    ) -> AppResult<()> {
        use tauri::Emitter;
        let _ = self.app.emit(
            SYNC_EVENT,
            SyncProgressEvent {
                run_id: run_id.to_string(),
                stage: stage.to_string(),
                resource: resource.to_string(),
                processed,
                total,
                message: message.map(|m| m.to_string()),
            },
        );
        Ok(())
    }
}

/// Human summary for the sync history screen.
pub fn summarize(run: &SyncRun) -> String {
    let s = &run.stats;
    format!(
        "课程 {} · 章节 {} · 作业 {} · 公告 {} · 日历 {} · 成绩 {} · 讨论 {} · 页面 {}",
        s.courses, s.modules, s.assignments, s.announcements, s.events, s.grades, s.discussions, s.pages
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_mentions_every_dataset() {
        let run = SyncRun {
            id: "r".into(),
            status: "completed".into(),
            scope: "all".into(),
            course_id: None,
            stats: SyncStats {
                courses: 1,
                modules: 2,
                assignments: 3,
                announcements: 4,
                events: 5,
                grades: 6,
                discussions: 7,
                pages: 8,
                ..SyncStats::default()
            },
            error: None,
            started_at: None,
            finished_at: None,
            created_at: "t".into(),
        };
        let text = summarize(&run);
        assert!(text.contains("课程 1"));
        assert!(text.contains("页面 8"));
    }
}
