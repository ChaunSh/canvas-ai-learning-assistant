//! Study planning service: gathers inputs, delegates the timetable to the
//! deterministic scheduler, and persists the result.

pub mod scheduler;

use std::collections::HashSet;

use chrono::{Duration, NaiveDate};

use crate::domain::{
    GeneratePlanInput, PlanResult, StudyPlan, StudyTask, UnscheduledTask,
};
use crate::error::{AppError, AppResult};
use crate::storage::{course_repo, coursework_repo, document_repo, planning_repo, Db};
use crate::time::{parse_date, today_in_zone};

/// Build (and persist) a study plan.
pub fn generate(db: &Db, input: GeneratePlanInput) -> AppResult<PlanResult> {
    let preferences = db.with(|c| planning_repo::get_preferences(c))?;
    let today = today_in_zone(&preferences.timezone);

    let start_date = match input.start_date.as_deref() {
        Some(value) => parse_date(value)
            .ok_or_else(|| AppError::invalid("开始日期格式应为 YYYY-MM-DD"))?,
        None => today,
    };
    let end_date = match input.end_date.as_deref() {
        Some(value) => parse_date(value).ok_or_else(|| AppError::invalid("结束日期格式应为 YYYY-MM-DD"))?,
        None => start_date + Duration::days(13),
    };
    if end_date < start_date {
        return Err(AppError::invalid("结束日期不能早于开始日期"));
    }
    if (end_date - start_date).num_days() > 120 {
        return Err(AppError::invalid("计划跨度不能超过 120 天"));
    }

    let kind = input.kind.clone().unwrap_or_else(|| "weekly".into());
    if !matches!(kind.as_str(), "daily" | "weekly" | "exam" | "assignment" | "custom") {
        return Err(AppError::invalid("计划类型无效"));
    }

    let name = input.name.clone().unwrap_or_else(|| {
        match kind.as_str() {
            "daily" => "每日学习计划".to_string(),
            "exam" => "考前复习计划".to_string(),
            "assignment" => "作业准备计划".to_string(),
            "custom" => "自定义学习计划".to_string(),
            _ => "每周学习计划".to_string(),
        }
    });

    let course_ids: HashSet<String> = input.course_ids.iter().cloned().collect();
    let range_from = crate::time::to_iso(
        start_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc(),
    );
    let range_to = crate::time::to_iso(
        (end_date + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc(),
    );

    // --- candidates --------------------------------------------------------
    let mut candidates: Vec<scheduler::TaskCandidate> = Vec::new();
    let mut explanation: Vec<String> = Vec::new();

    if input.include_assignments {
        let assignments = db.with(|c| coursework_repo::list_assignments(c, None, false, 500))?;
        let mut added = 0;
        for assignment in assignments {
            if assignment.submission_status == "graded" {
                continue;
            }
            let Some(due) = assignment.due_at.as_deref().and_then(crate::time::parse_iso) else {
                continue;
            };
            if due < chrono::Utc::now() {
                continue;
            }
            if !course_ids.is_empty() {
                if !course_ids.contains(&assignment.course_id) {
                    continue;
                }
            }
            let due_local = due.date_naive();
            if due_local < start_date || due_local > end_date + Duration::days(1) {
                continue;
            }
            let minutes = estimate_assignment_minutes(&assignment);
            candidates.push(scheduler::TaskCandidate {
                title: format!("准备：{}", assignment.title),
                estimated_minutes: minutes,
                deadline: Some(due),
                priority: if assignment.points_possible.unwrap_or(0.0) >= 20.0 { 2 } else { 3 },
                course_id: Some(assignment.course_id.clone()),
                material_id: None,
                assignment_id: Some(assignment.id.clone()),
                source_references: vec![format!("assignment:{}", assignment.id)],
            });
            added += 1;
        }
        if added > 0 {
            explanation.push(format!("从作业截止时间生成 {added} 个准备任务。"));
        }
    }

    if input.include_materials {
        let materials = db.with(|c| {
            document_repo::list(
                c,
                &crate::domain::MaterialFilter {
                    course_id: None,
                    kind: None,
                    query: None,
                    favorites_only: false,
                    index_status: None,
                    limit: Some(200),
                    offset: None,
                },
            )
        })?;
        let mut added = 0;
        for material in materials {
            let Some(course_id) = material.course_id.clone() else {
                continue;
            };
            if !course_ids.is_empty() && !course_ids.contains(&course_id) {
                continue;
            }
            if material.index_status == "failed" {
                continue;
            }
            let minutes = estimate_material_minutes(&material);
            candidates.push(scheduler::TaskCandidate {
                title: format!("阅读：{}", material.title),
                estimated_minutes: minutes,
                deadline: None,
                priority: 4,
                course_id: Some(course_id),
                material_id: Some(material.id.clone()),
                assignment_id: None,
                source_references: vec![format!("material:{}", material.id)],
            });
            added += 1;
        }
        if added > 0 {
            explanation.push(format!("为本学期 {added} 份课件生成阅读任务。"));
        }
    }

    if candidates.is_empty() {
        return Err(AppError::invalid(
            "没有可用于生成计划的任务：请先添加作业或导入课件，或在设置中调整计划范围。",
        ));
    }

    let occupied = db.with(|c| planning_repo::occupied_intervals(c, &range_from, &range_to))?;
    let existing: Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = occupied
        .into_iter()
        .filter_map(|(from, to, _)| {
            Some((crate::time::parse_iso(&from)?, crate::time::parse_iso(&to)?))
        })
        .collect();

    let output = scheduler::plan(
        start_date,
        end_date,
        &preferences,
        &existing,
        candidates.clone(),
    );
    explanation.extend(output.explanation.clone());

    // --- persistence -------------------------------------------------------
    let plan = db.transaction(|tx| {
        let plan = planning_repo::create_plan(
            tx,
            &name,
            &kind,
            &start_date.to_string(),
            &end_date.to_string(),
            input.notes.as_deref(),
        )?;

        let mut created = Vec::new();
        for block in &output.blocks {
            let candidate = &candidates[block.candidate_index];
            let task = planning_repo::create_task(
                tx,
                &planning_repo::TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: candidate.course_id.clone(),
                    material_id: candidate.material_id.clone(),
                    assignment_id: candidate.assignment_id.clone(),
                    title: candidate.title.clone(),
                    description: None,
                    estimated_minutes: block.minutes,
                    scheduled_start: Some(crate::time::to_iso(block.start)),
                    scheduled_end: Some(crate::time::to_iso(block.end)),
                    deadline: candidate.deadline.map(crate::time::to_iso),
                    priority: candidate.priority,
                    generated_by: "rule".into(),
                    source_references: candidate.source_references.clone(),
                },
            )?;
            created.push(task);
        }
        Ok((plan, created))
    })?;

    Ok(PlanResult {
        plan: plan.0,
        scheduled: plan.1,
        unscheduled: output.unscheduled,
        explanation,
    })
}

/// Replan an existing plan, preserving locked and user-created tasks.
pub fn replan(db: &Db, plan_id: &str) -> AppResult<PlanResult> {
    let plan = db.with(|c| planning_repo::get_plan(c, plan_id))?;
    let removed = db.with(|c| planning_repo::clear_generated_tasks(c, plan_id))?;
    tracing::info!(plan = %plan_id, removed, "cleared auto-generated tasks before replanning");

    let result = generate(
        db,
        GeneratePlanInput {
            kind: Some(plan.kind.clone()),
            start_date: Some(plan.start_date.clone()),
            end_date: Some(plan.end_date.clone()),
            name: Some(plan.name.clone()),
            course_ids: Vec::new(),
            include_assignments: true,
            include_materials: true,
            notes: plan.notes.clone(),
        },
    )?;
    // The freshly generated plan supersedes the old one.
    db.with(|c| planning_repo::set_plan_status(c, plan_id, "archived"))?;
    Ok(result)
}

pub fn list_plan_tasks(db: &Db, plan_id: &str) -> AppResult<Vec<StudyTask>> {
    db.with(|c| planning_repo::list_tasks(c, Some(plan_id), None, None, None, true))
}

pub fn task_range(
    db: &Db,
    from: Option<&str>,
    to: Option<&str>,
) -> AppResult<Vec<StudyTask>> {
    db.with(|c| planning_repo::list_tasks(c, None, None, from, to, true))
}

pub fn most_recent_plan(db: &Db) -> AppResult<Option<StudyPlan>> {
    let plans = db.with(|c| planning_repo::list_plans(c))?;
    Ok(plans.into_iter().find(|p| p.status != "archived"))
}

/// Reading-time estimate from extracted character counts.
fn estimate_material_minutes(material: &crate::domain::Material) -> i64 {
    let chars = material.char_count.max(0);
    if chars == 0 {
        return 30;
    }
    // ~1000 characters (≈500 CJK chars) per minute of careful reading.
    let minutes = (chars as f64 / 1000.0 * 12.0).round() as i64;
    minutes.clamp(15, 180)
}

fn estimate_assignment_minutes(assignment: &crate::domain::Assignment) -> i64 {
    let description_len = assignment
        .description
        .as_deref()
        .map(|d| d.chars().count())
        .unwrap_or(0) as f64;
    let base = 45.0 + (description_len / 400.0) * 15.0;
    let weighted = base + assignment.points_possible.unwrap_or(0.0) * 1.5;
    (weighted.round() as i64).clamp(30, 240)
}

/// Convert unscheduled entries into a human summary for the UI.
pub fn summarize_unscheduled(entries: &[UnscheduledTask]) -> String {
    if entries.is_empty() {
        return "全部任务均已安排。".into();
    }
    let minutes: i64 = entries.iter().map(|e| e.estimated_minutes).sum();
    format!(
        "{} 项任务共 {} 分钟未能安排，请延长计划周期或增加每日学习时长。",
        entries.len(),
        minutes
    )
}

/// Aggregate progress across a plan for the dashboard ring.
pub fn plan_progress(db: &Db, plan_id: &str) -> AppResult<f64> {
    let tasks = list_plan_tasks(db, plan_id)?;
    if tasks.is_empty() {
        return Ok(0.0);
    }
    let done = tasks.iter().filter(|t| t.status == "done").count() as f64;
    Ok(done / tasks.len() as f64)
}

/// Convenience for the "upcoming deadlines" widget.
pub fn upcoming(db: &Db, horizon_days: i64, limit: i64) -> AppResult<Vec<crate::domain::UpcomingDeadline>> {
    db.with(|c| coursework_repo::upcoming_deadlines(c, horizon_days, limit))
}

/// Courses that currently contain open tasks, used to scope AI planning hints.
pub fn courses_with_open_tasks(db: &Db) -> AppResult<Vec<String>> {
    let tasks = db.with(|c| planning_repo::list_tasks(c, None, None, None, None, false))?;
    Ok(tasks
        .into_iter()
        .filter_map(|t| t.course_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect())
}

/// Ensure the scheduler never places work in the past.
pub fn first_available_date(timezone: &str) -> NaiveDate {
    today_in_zone(timezone)
}

/// Expose course names for plan descriptions.
pub fn course_name(db: &Db, course_id: &str) -> AppResult<Option<String>> {
    match db.with(|c| course_repo::get(c, course_id)) {
        Ok(course) => Ok(Some(course.name)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn seed(db: &Db) {
        db.with(|c| {
            c.execute(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                 VALUES ('k1','微积分','manual','local',0,1,'t','t')",
                [],
            )?;
            let due = crate::time::to_iso(chrono::Utc::now() + Duration::days(3));
            c.execute(
                "INSERT INTO assignments (id, course_id, title, due_at, submission_status, source, created_at, updated_at)
                 VALUES ('a1','k1','作业 1',?1,'not_submitted','manual','t','t')",
                [due],
            )?;
            c.execute(
                "INSERT INTO materials (id, course_id, title, filename, kind, sha256, storage_path, source, index_status, char_count, created_at, updated_at)
                 VALUES ('m1','k1','第一讲','l1.pdf','pdf','h1','p1','import','indexed',12000,'t','t')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn generates_and_persists_a_plan() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let today = today_in_zone("Asia/Shanghai");
        let result = generate(
            &db,
            GeneratePlanInput {
                kind: Some("weekly".into()),
                start_date: Some(today.to_string()),
                end_date: Some((today + Duration::days(6)).to_string()),
                name: None,
                course_ids: vec![],
                include_assignments: true,
                include_materials: true,
                notes: None,
            },
        )
        .unwrap();
        assert!(!result.scheduled.is_empty());
        assert!(!result.explanation.is_empty());
        let stored = list_plan_tasks(&db, &result.plan.id).unwrap();
        assert_eq!(stored.len(), result.scheduled.len());
        assert!(stored.iter().all(|t| t.generated_by == "rule"));
        assert!(stored.iter().all(|t| t.scheduled_start.is_some()));
    }

    #[test]
    fn rejects_empty_candidate_set() {
        let db = Db::open_in_memory().unwrap();
        let err = generate(
            &db,
            GeneratePlanInput {
                kind: Some("daily".into()),
                start_date: None,
                end_date: None,
                name: None,
                course_ids: vec![],
                include_assignments: true,
                include_materials: true,
                notes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }

    #[test]
    fn rejects_inverted_range() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let err = generate(
            &db,
            GeneratePlanInput {
                kind: Some("weekly".into()),
                start_date: Some("2026-11-10".into()),
                end_date: Some("2026-11-01".into()),
                name: None,
                course_ids: vec![],
                include_assignments: true,
                include_materials: true,
                notes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }

    #[test]
    fn material_estimate_is_clamped() {
        let mut material = crate::domain::Material {
            id: "m".into(),
            course_id: None,
            course_name: None,
            module_id: None,
            external_id: None,
            title: "t".into(),
            filename: "f".into(),
            mime_type: None,
            kind: "pdf".into(),
            byte_size: 0,
            sha256: "h".into(),
            storage_path: "p".into(),
            source: "import".into(),
            remote_url: None,
            index_status: "indexed".into(),
            index_error: None,
            page_count: 0,
            slide_count: 0,
            char_count: 0,
            needs_ocr: false,
            is_favorite: false,
            tags: vec![],
            ai_upload_allowed: true,
            last_opened_at: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            chunk_count: 0,
        };
        assert_eq!(estimate_material_minutes(&material), 30);
        material.char_count = 10_000_000;
        assert_eq!(estimate_material_minutes(&material), 180);
        material.char_count = 5000;
        assert!((15..=180).contains(&estimate_material_minutes(&material)));
    }
}
