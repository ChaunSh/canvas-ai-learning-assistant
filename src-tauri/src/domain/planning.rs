use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyPlan {
    pub id: String,
    pub name: String,
    /// daily | weekly | exam | assignment | custom
    pub kind: String,
    /// Local calendar day `YYYY-MM-DD`.
    pub start_date: String,
    pub end_date: String,
    /// draft | active | archived
    pub status: String,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub task_count: i64,
    #[serde(default)]
    pub done_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyTask {
    pub id: String,
    pub plan_id: Option<String>,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub material_id: Option<String>,
    pub material_title: Option<String>,
    pub assignment_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub estimated_minutes: i64,
    pub scheduled_start: Option<String>,
    pub scheduled_end: Option<String>,
    pub deadline: Option<String>,
    pub priority: i64,
    /// todo | in_progress | done | skipped
    pub status: String,
    /// Locked tasks are never moved by replanning.
    pub locked: bool,
    /// ai | rule | user
    pub generated_by: String,
    pub source_references: Vec<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyPreferences {
    pub daily_minutes: i64,
    pub session_minutes: i64,
    pub break_minutes: i64,
    /// `HH:MM`
    pub preferred_start: String,
    /// `HH:MM`
    pub preferred_end: String,
    /// ISO weekdays, Monday = 1 … Sunday = 7.
    pub study_days: Vec<i64>,
    pub timezone: String,
    pub avoid_weekends: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreferencesInput {
    #[serde(default)]
    pub daily_minutes: Option<i64>,
    #[serde(default)]
    pub session_minutes: Option<i64>,
    #[serde(default)]
    pub break_minutes: Option<i64>,
    #[serde(default)]
    pub preferred_start: Option<String>,
    #[serde(default)]
    pub preferred_end: Option<String>,
    #[serde(default)]
    pub study_days: Option<Vec<i64>>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub avoid_weekends: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePlanInput {
    /// daily | weekly | exam | assignment | custom
    #[serde(default)]
    pub kind: Option<String>,
    /// Local calendar day. Defaults to today in the user's timezone.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Inclusive last day of the plan.
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Restrict to these courses; empty means all active courses.
    #[serde(default)]
    pub course_ids: Vec<String>,
    /// Turn assignment deadlines into study blocks.
    #[serde(default = "default_true")]
    pub include_assignments: bool,
    /// Turn un-read materials into study blocks.
    #[serde(default = "default_true")]
    pub include_materials: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Result of planning. Unscheduled tasks could not fit before their deadline
/// and are surfaced explicitly instead of being silently dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanResult {
    pub plan: StudyPlan,
    pub scheduled: Vec<StudyTask>,
    pub unscheduled: Vec<UnscheduledTask>,
    /// Deterministic, human-readable explanation of the scheduling decisions.
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnscheduledTask {
    pub title: String,
    pub estimated_minutes: i64,
    pub deadline: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub estimated_minutes: Option<i64>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub scheduled_start: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub scheduled_end: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub deadline: Option<Option<String>>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub course_id: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub course_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub estimated_minutes: Option<i64>,
    #[serde(default)]
    pub scheduled_start: Option<String>,
    #[serde(default)]
    pub scheduled_end: Option<String>,
    #[serde(default)]
    pub deadline: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
}

/// A deadline the planner should respect, with its provenance so the UI never
/// presents an unverified date as a confirmed assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingDeadline {
    pub id: String,
    /// assignment | task | event
    pub kind: String,
    pub title: String,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub due_at: String,
    /// canvas | manual | import | ai
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySessionRecord {
    pub id: String,
    pub task_id: Option<String>,
    pub course_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub minutes: i64,
    pub notes: Option<String>,
    pub created_at: String,
}
