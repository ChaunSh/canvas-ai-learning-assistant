//! Parsing of Canvas JSON into typed records.
//!
//! All identifiers are kept as `String`. Canvas ids are 64-bit integers; parsing
//! them into JavaScript/`f64` would silently corrupt values above 2^53, so the
//! deserialisers here accept either a JSON number or a string and stringify.
//!
//! Every struct is tolerant of missing fields: Canvas omits keys depending on the
//! caller's permissions, and a missing field must not be mistaken for "empty
//! course".

use serde::{Deserialize, Deserializer};

/// Deserialize an id that may be a JSON number or a string.
pub fn de_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        other => Err(D::Error::custom(format!("unexpected id type: {other}"))),
    }
}

/// Same as [`de_id`] but for optional ids.
pub fn de_opt_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(Some(s)),
        serde_json::Value::Number(n) => Ok(Some(n.to_string())),
        serde_json::Value::Null => Ok(None),
        other => Err(D::Error::custom(format!("unexpected id type: {other}"))),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawUser {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub short_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTerm {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTeacher {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawCourse {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub course_code: Option<String>,
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub term: Option<RawTerm>,
    #[serde(default)]
    pub teachers: Option<Vec<RawTeacher>>,
    #[serde(default)]
    pub syllabus_body: Option<String>,
    #[serde(default)]
    pub access_restricted_by_date: Option<bool>,
}

impl RawCourse {
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| self.course_code.clone())
            .unwrap_or_else(|| format!("课程 {}", self.id))
    }

    pub fn instructor(&self) -> Option<String> {
        let names: Vec<String> = self
            .teachers
            .as_ref()?
            .iter()
            .filter_map(|t| t.display_name.clone())
            .filter(|n| !n.trim().is_empty())
            .collect();
        if names.is_empty() {
            None
        } else {
            Some(names.join(", "))
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawModule {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub unlock_at: Option<String>,
    #[serde(default)]
    pub items_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawFile {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default, rename = "content-type")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl RawFile {
    pub fn file_name(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| self.filename.clone())
            .unwrap_or_else(|| format!("file-{}", self.id))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawSubmission {
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub grade: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAssignment {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub unlock_at: Option<String>,
    #[serde(default)]
    pub points_possible: Option<f64>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub submission: Option<RawSubmission>,
}

impl RawAssignment {
    pub fn submission_status(&self) -> &'static str {
        match self.submission.as_ref().and_then(|s| s.workflow_state.as_deref()) {
            Some("submitted") => "submitted",
            Some("graded") | Some("pending_review") => "graded",
            Some("unsubmitted") => "not_submitted",
            Some("excused") => "excused",
            Some(_) => "pending",
            None => "unknown",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAuthor {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAnnouncement {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub posted_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub author: Option<RawAuthor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawCalendarEvent {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location_name: Option<String>,
    #[serde(default)]
    pub start_at: Option<String>,
    #[serde(default)]
    pub end_at: Option<String>,
    #[serde(default)]
    pub all_day: Option<bool>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

impl RawCalendarEvent {
    /// Distinguish an exam-like event from a generic one using Canvas metadata and
    /// the event title. This is a heuristic and is labelled as such in the UI.
    pub fn event_kind(&self) -> &'static str {
        let title = self.title.as_deref().unwrap_or("").to_lowercase();
        if title.contains("exam") || title.contains("考试") || title.contains("测验") {
            return "exam";
        }
        match self.kind.as_deref() {
            Some("assignment") => "assignment_due",
            Some("calendar_event") | Some("event") => "event",
            _ => "event",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawGrades {
    #[serde(default)]
    pub current_score: Option<f64>,
    #[serde(default)]
    pub current_grade: Option<String>,
    #[serde(default)]
    pub final_score: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawEnrollment {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub course_id: Option<String>,
    #[serde(default)]
    pub grades: Option<RawGrades>,
    #[serde(default)]
    pub computed_current_score: Option<f64>,
    #[serde(default)]
    pub computed_current_grade: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawDiscussion {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub posted_at: Option<String>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub discussion_subentry_count: Option<i64>,
    #[serde(default)]
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPage {
    #[serde(default)]
    pub page_id: Option<serde_json::Value>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
}

impl RawPage {
    pub fn external_id(&self) -> String {
        match &self.page_id {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => self.url.clone().unwrap_or_default(),
        }
    }
}

/// Canvas paginates with `Link` headers that the bridge cannot see, so callers
/// page explicitly. A full page means "there may be more".
pub const PAGE_SIZE: usize = 100;

pub fn has_more(items: &[serde_json::Value]) -> bool {
    items.len() >= PAGE_SIZE
}

/// Parse a JSON array body, tolerating an object wrapper.
pub fn as_array(value: &serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_ids_survive_as_strings() {
        // 2^53 + 1 would lose precision as f64.
        let json = r#"{"id": 9007199254740993, "name": "Math"}"#;
        let course: RawCourse = serde_json::from_str(json).unwrap();
        assert_eq!(course.id, "9007199254740993");
    }

    #[test]
    fn string_ids_are_accepted() {
        let json = r#"{"id": "123", "name": "Math"}"#;
        let course: RawCourse = serde_json::from_str(json).unwrap();
        assert_eq!(course.id, "123");
    }

    #[test]
    fn course_defaults_and_instructor() {
        let json = r#"{"id": 1, "course_code": "COMP 2012",
                       "teachers": [{"display_name": "Dr. A"}, {"display_name": "Dr. B"}]}"#;
        let course: RawCourse = serde_json::from_str(json).unwrap();
        assert_eq!(course.display_name(), "COMP 2012");
        assert_eq!(course.instructor().as_deref(), Some("Dr. A, Dr. B"));
    }

    #[test]
    fn submission_status_mapping() {
        let make = |state: &str| -> &'static str {
            let json = format!(r#"{{"id":1,"submission":{{"workflow_state":"{state}"}}}}"#);
            let a: RawAssignment = serde_json::from_str(&json).unwrap();
            a.submission_status()
        };
        assert_eq!(make("unsubmitted"), "not_submitted");
        assert_eq!(make("submitted"), "submitted");
        assert_eq!(make("graded"), "graded");
        assert_eq!(make("excused"), "excused");
        assert_eq!(make("weird"), "pending");
    }

    #[test]
    fn assignment_without_submission_is_unknown() {
        let a: RawAssignment = serde_json::from_str(r#"{"id": 5, "name": "Quiz"}"#).unwrap();
        assert_eq!(a.submission_status(), "unknown");
        assert_eq!(a.name.as_deref(), Some("Quiz"));
    }

    #[test]
    fn exam_heuristic() {
        let event: RawCalendarEvent =
            serde_json::from_str(r#"{"id":1,"title":"Final Exam","type":"event"}"#).unwrap();
        assert_eq!(event.event_kind(), "exam");
        let regular: RawCalendarEvent =
            serde_json::from_str(r#"{"id":2,"title":"Guest lecture","type":"event"}"#).unwrap();
        assert_eq!(regular.event_kind(), "event");
        let assignment: RawCalendarEvent =
            serde_json::from_str(r#"{"id":3,"title":"HW1","type":"assignment"}"#).unwrap();
        assert_eq!(assignment.event_kind(), "assignment_due");
    }

    #[test]
    fn page_external_id_falls_back_to_url() {
        let page: RawPage =
            serde_json::from_str(r#"{"url":"week-1","title":"Week 1"}"#).unwrap();
        assert_eq!(page.external_id(), "week-1");
    }

    #[test]
    fn missing_optional_fields_do_not_fail() {
        let course: RawCourse = serde_json::from_str(r#"{"id": 7}"#).unwrap();
        assert_eq!(course.display_name(), "课程 7");
        assert!(course.instructor().is_none());
        assert!(course.term.is_none());
    }

    #[test]
    fn file_name_prefers_display_name() {
        let file: RawFile = serde_json::from_str(
            r#"{"id": 1, "display_name": "Lecture 1.pdf", "filename": "l1.pdf"}"#,
        )
        .unwrap();
        assert_eq!(file.file_name(), "Lecture 1.pdf");
    }

    #[test]
    fn array_helper_is_tolerant() {
        assert_eq!(as_array(&serde_json::json!([1, 2])).len(), 2);
        assert!(as_array(&serde_json::json!({"error": "x"})).is_empty());
    }
}
