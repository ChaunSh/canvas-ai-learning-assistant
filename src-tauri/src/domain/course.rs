use serde::{Deserialize, Serialize};

/// A course. `id` is a local UUID; `externalId` preserves the Canvas id as a
/// string so 64-bit identifiers never lose precision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub id: String,
    pub external_id: Option<String>,
    pub code: Option<String>,
    pub name: String,
    pub term: Option<String>,
    pub instructor: Option<String>,
    pub description: Option<String>,
    /// canvas | manual | import
    pub source: String,
    /// local | synced | stale | error
    pub sync_status: String,
    pub archived: bool,
    pub color: Option<String>,
    pub position: i64,
    pub last_synced_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,

    // Aggregates joined in for list views. Not stored on the courses table.
    #[serde(default)]
    pub material_count: i64,
    #[serde(default)]
    pub open_task_count: i64,
    #[serde(default)]
    pub due_soon_count: i64,
    #[serde(default)]
    pub progress: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCourseInput {
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub term: Option<String>,
    #[serde(default)]
    pub instructor: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCourseInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub term: Option<String>,
    #[serde(default)]
    pub instructor: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseModule {
    pub id: String,
    pub course_id: String,
    pub external_id: Option<String>,
    pub name: String,
    pub position: i64,
    pub unlock_at: Option<String>,
    pub item_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModuleInput {
    pub course_id: String,
    pub name: String,
    #[serde(default)]
    pub unlock_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub id: String,
    pub course_id: String,
    pub course_name: Option<String>,
    pub external_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub due_at: Option<String>,
    pub unlock_at: Option<String>,
    pub points_possible: Option<f64>,
    /// unknown | not_submitted | submitted | graded | excused | pending
    pub submission_status: String,
    pub html_url: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAssignmentInput {
    pub course_id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub points_possible: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAssignmentInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Tri-state: absent = leave, null = clear, value = set.
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub due_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub points_possible: Option<Option<f64>>,
    #[serde(default)]
    pub submission_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: String,
    pub course_id: String,
    pub course_name: Option<String>,
    pub external_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub author: Option<String>,
    pub posted_at: Option<String>,
    pub html_url: Option<String>,
    pub read: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Calendar event. `startsAt` is ISO-8601 UTC. `ics_uid` de-duplicates repeat
/// imports of the same ICS feed (task specification §36).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub external_id: Option<String>,
    pub ics_uid: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub all_day: bool,
    /// event | assignment_due | exam | study | personal
    pub kind: String,
    /// canvas | manual | import | ai
    pub source: String,
    pub assignable_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEventInput {
    #[serde(default)]
    pub course_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeRecord {
    pub id: String,
    pub course_id: String,
    pub course_name: Option<String>,
    pub external_id: Option<String>,
    pub assignment_name: String,
    pub score: Option<f64>,
    pub points_possible: Option<f64>,
    pub grade: Option<String>,
    pub feedback: Option<String>,
    pub posted_at: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscussionTopic {
    pub id: String,
    pub course_id: String,
    pub external_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub author: Option<String>,
    pub posted_at: Option<String>,
    pub reply_count: i64,
    pub html_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscussionEntry {
    pub id: String,
    pub topic_id: String,
    pub external_id: Option<String>,
    pub parent_id: Option<String>,
    pub author: Option<String>,
    pub body: Option<String>,
    pub posted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoursePage {
    pub id: String,
    pub course_id: String,
    pub external_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub url: Option<String>,
    pub updated_at_remote: Option<String>,
    pub html_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Aggregated payload for the dashboard so the first screen needs one round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub courses: Vec<Course>,
    pub upcoming: Vec<Assignment>,
    pub today_tasks: Vec<super::planning::StudyTask>,
    pub recent_materials: Vec<super::material::Material>,
    pub announcements: Vec<Announcement>,
    pub stats: DashboardStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub course_count: i64,
    pub material_count: i64,
    pub indexed_count: i64,
    pub page_count: i64,
    pub chat_count: i64,
    pub open_tasks: i64,
    pub done_tasks: i64,
    pub due_this_week: i64,
}

/// Simple `{ id, name }` pair for filter dropdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdName {
    pub id: String,
    pub name: String,
}
