/**
 * Typed wrapper around the Tauri command surface.
 *
 * Every command is invoked by name with camelCase arguments, matching the Rust
 * `#[tauri::command]` signatures. Errors are normalised to `IpcError` so the UI
 * can branch on a stable `code` instead of parsing messages.
 */
import { invoke } from "@tauri-apps/api/core";

import type {
  AiProvider,
  Announcement,
  AppInfo,
  AskAccepted,
  Assignment,
  CacheInfo,
  CalendarEvent,
  CanvasConnection,
  CanvasFile,
  Capability,
  CourseFileGroup,
  DownloadReport,
  ChatDetail,
  ChatScope,
  ChatSession,
  Course,
  CourseDetail,
  CourseModule,
  CoursePage,
  Dashboard,
  DiscussionEntry,
  DiscussionTopic,
  DocumentChunk,
  DocumentPage,
  DocumentSummary,
  GradeRecord,
  ImportOutcome,
  IndexProgress,
  LibraryStats,
  Material,
  PlanResult,
  ProviderInput,
  ProviderTestResult,
  ProviderView,
  SearchResponse,
  StudyPlan,
  StudyPreferences,
  StudyTask,
  SyncRun,
  UpcomingDeadline,
  UsageStats,
} from "@/types/api";

export class IpcError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "IpcError";
    this.code = code;
  }
}

function normalizeError(raw: unknown): IpcError {
  if (raw && typeof raw === "object" && "message" in raw) {
    const value = raw as { code?: string; message?: string };
    return new IpcError(value.code ?? "unknown", value.message ?? "未知错误");
  }
  if (typeof raw === "string") return new IpcError("unknown", raw);
  return new IpcError("unknown", "命令执行失败");
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw normalizeError(error);
  }
}

export const api = {
  // --- system -----------------------------------------------------------
  appInfo: () => call<AppInfo>("app_info"),
  settingsGet: (key: string) => call<string | null>("settings_get", { key }),
  settingsSet: (key: string, value: string) => call<void>("settings_set", { key, value }),
  settingsAll: () =>
    call<{ key: string; value: string; updatedAt: string | null }[]>("settings_all"),

  // --- courses ----------------------------------------------------------
  listCourses: (includeArchived = false) =>
    call<Course[]>("list_courses", { includeArchived }),
  getCourse: (courseId: string) => call<Course>("get_course", { courseId }),
  createCourse: (input: {
    name: string;
    code?: string | null;
    term?: string | null;
    instructor?: string | null;
    description?: string | null;
    color?: string | null;
  }) => call<Course>("create_course", { input }),
  updateCourse: (
    courseId: string,
    patch: Partial<{
      name: string;
      code: string;
      term: string;
      instructor: string;
      description: string;
      color: string;
      archived: boolean;
      position: number;
    }>,
  ) => call<Course>("update_course", { courseId, patch }),
  deleteCourse: (courseId: string) => call<void>("delete_course", { courseId }),
  listCourseModules: (courseId: string) =>
    call<CourseModule[]>("list_course_modules", { courseId }),
  createCourseModule: (input: { courseId: string; name: string; unlockAt?: string | null }) =>
    call<CourseModule>("create_course_module", { input }),
  deleteCourseModule: (moduleId: string) => call<void>("delete_course_module", { moduleId }),
  courseOptions: () => call<{ id: string; name: string }[]>("course_options"),
  getCourseDetail: (courseId: string) =>
    call<CourseDetail>("get_course_detail", { courseId }),

  // --- coursework -------------------------------------------------------
  listAssignments: (courseId?: string | null, upcomingOnly = false, limit = 200) =>
    call<Assignment[]>("list_assignments", { courseId, upcomingOnly, limit }),
  createAssignment: (input: {
    courseId: string;
    title: string;
    description?: string | null;
    dueAt?: string | null;
    pointsPossible?: number | null;
  }) => call<Assignment>("create_assignment", { input }),
  updateAssignment: (assignmentId: string, patch: Record<string, unknown>) =>
    call<Assignment>("update_assignment", { assignmentId, patch }),
  deleteAssignment: (assignmentId: string) =>
    call<void>("delete_assignment", { assignmentId }),
  listAnnouncements: (courseId?: string | null, limit = 50) =>
    call<Announcement[]>("list_announcements", { courseId, limit }),
  markAnnouncementRead: (announcementId: string, read: boolean) =>
    call<void>("mark_announcement_read", { announcementId, read }),
  listGrades: (courseId?: string | null) => call<GradeRecord[]>("list_grades", { courseId }),
  listDiscussions: (courseId: string) => call<DiscussionTopic[]>("list_discussions", { courseId }),
  listDiscussionEntries: (topicId: string) =>
    call<DiscussionEntry[]>("list_discussion_entries", { topicId }),
  listCoursePages: (courseId: string) => call<CoursePage[]>("list_course_pages", { courseId }),
  getDashboard: () => call<Dashboard>("get_dashboard"),

  // --- materials --------------------------------------------------------
  importMaterials: (input: {
    paths: string[];
    courseId?: string | null;
    moduleId?: string | null;
    expandArchives?: boolean;
  }) => call<ImportOutcome>("import_materials", { input }),
  listMaterials: (filter: {
    courseId?: string | null;
    kind?: string | null;
    query?: string | null;
    favoritesOnly?: boolean;
    indexStatus?: string | null;
    limit?: number;
    offset?: number;
  }) => call<Material[]>("list_materials", { filter }),
  getMaterial: (materialId: string) => call<Material>("get_material", { materialId }),
  updateMaterial: (materialId: string, patch: Record<string, unknown>) =>
    call<Material>("update_material", { materialId, patch }),
  deleteMaterial: (materialId: string) => call<void>("delete_material", { materialId }),
  /** Raw bytes as an ArrayBuffer (Tauri IPC binary response). */
  materialBytes: (materialId: string) =>
    call<ArrayBuffer>("material_bytes", { materialId }),
  materialPages: (materialId: string) =>
    call<DocumentPage[]>("material_pages", { materialId }),
  materialChunks: (materialId: string) =>
    call<DocumentChunk[]>("material_chunks", { materialId }),
  indexMaterial: (materialId: string) => call<void>("index_material", { materialId }),
  cancelIndexing: (materialId: string) => call<void>("cancel_indexing", { materialId }),
  searchMaterials: (query: {
    query: string;
    courseId?: string | null;
    materialIds?: string[];
    limit?: number;
  }) => call<SearchResponse>("search_materials", { query }),
  listSummaries: (materialId: string) =>
    call<DocumentSummary[]>("list_summaries", { materialId }),
  generateSummary: (input: {
    materialId: string;
    kind?: string;
    scope?: string | null;
    language?: string;
    force?: boolean;
  }) => call<DocumentSummary>("generate_summary", { input }),
  deleteSummary: (materialId: string, kind: string, scope?: string | null) =>
    call<void>("delete_summary", { materialId, kind, scope }),
  reindexAll: () => call<void>("reindex_all"),
  libraryStats: () => call<LibraryStats>("library_stats"),
  indexingStatus: (materialId: string) =>
    call<IndexProgress | null>("indexing_status", { materialId }),

  // --- ai ---------------------------------------------------------------
  listProviders: () => call<ProviderView>("list_providers"),
  saveProvider: (input: ProviderInput) => call<AiProvider>("save_provider", { input }),
  deleteProvider: (providerId: string) => call<void>("delete_provider", { providerId }),
  setDefaultProvider: (providerId: string) =>
    call<void>("set_default_provider", { providerId }),
  testProvider: (providerId: string) =>
    call<ProviderTestResult>("test_provider", { providerId }),
  listModels: (providerId: string) =>
    call<{ id: string; ownedBy: string | null }[]>("list_models", { providerId }),
  askQuestion: (input: {
    sessionId?: string | null;
    question: string;
    scope?: ChatScope | null;
    providerId?: string | null;
    model?: string | null;
    topK?: number;
    noRetrieval?: boolean;
  }) => call<AskAccepted>("ask_question", { input }),
  cancelGeneration: (requestId: string) =>
    call<boolean>("cancel_generation", { requestId }),
  listChatSessions: (limit = 100) => call<ChatSession[]>("list_chat_sessions", { limit }),
  getChatSession: (sessionId: string) => call<ChatDetail>("get_chat_session", { sessionId }),
  deleteChatSession: (sessionId: string) =>
    call<void>("delete_chat_session", { sessionId }),
  renameChatSession: (sessionId: string, title: string) =>
    call<void>("rename_chat_session", { sessionId, title }),
  updateChatScope: (sessionId: string, scope: ChatScope) =>
    call<void>("update_chat_scope", { sessionId, scope }),
  aiUsage: () => call<UsageStats>("ai_usage"),

  // --- planning ---------------------------------------------------------
  getStudyPreferences: () => call<StudyPreferences>("get_study_preferences"),
  updateStudyPreferences: (patch: Record<string, unknown>) =>
    call<StudyPreferences>("update_study_preferences", { patch }),
  generateStudyPlan: (input: {
    kind?: string;
    startDate?: string;
    endDate?: string;
    name?: string;
    courseIds?: string[];
    includeAssignments?: boolean;
    includeMaterials?: boolean;
    notes?: string | null;
  }) => call<PlanResult>("generate_study_plan", { input }),
  replanSchedule: (planId: string) => call<PlanResult>("replan_schedule", { planId }),
  listStudyPlans: () => call<StudyPlan[]>("list_study_plans"),
  deleteStudyPlan: (planId: string) => call<void>("delete_study_plan", { planId }),
  setStudyPlanStatus: (planId: string, status: string) =>
    call<void>("set_study_plan_status", { planId, status }),
  listStudyTasks: (options?: {
    planId?: string | null;
    courseId?: string | null;
    from?: string | null;
    to?: string | null;
    includeDone?: boolean;
  }) =>
    call<StudyTask[]>("list_study_tasks", {
      planId: options?.planId ?? null,
      courseId: options?.courseId ?? null,
      from: options?.from ?? null,
      to: options?.to ?? null,
      includeDone: options?.includeDone ?? true,
    }),
  createStudyTask: (input: {
    planId?: string | null;
    courseId?: string | null;
    title: string;
    description?: string | null;
    estimatedMinutes?: number;
    scheduledStart?: string | null;
    scheduledEnd?: string | null;
    deadline?: string | null;
    priority?: number;
  }) => call<StudyTask>("create_study_task", { input }),
  updateStudyTask: (taskId: string, patch: Record<string, unknown>) =>
    call<StudyTask>("update_study_task", { taskId, patch }),
  completeStudyTask: (taskId: string, done: boolean) =>
    call<StudyTask>("complete_study_task", { taskId, done }),
  deleteStudyTask: (taskId: string) => call<void>("delete_study_task", { taskId }),
  upcomingDeadlines: (horizonDays = 30, limit = 40) =>
    call<UpcomingDeadline[]>("upcoming_deadlines", { horizonDays, limit }),
  startStudySession: (taskId?: string | null, courseId?: string | null) =>
    call<string>("start_study_session", { taskId, courseId }),
  finishStudySession: (sessionId: string, minutes: number) =>
    call<void>("finish_study_session", { sessionId, minutes }),
  listStudySessions: (limit = 50) =>
    call<
      {
        id: string;
        taskId: string | null;
        courseId: string | null;
        startedAt: string;
        endedAt: string | null;
        minutes: number;
        notes: string | null;
        createdAt: string;
      }[]
    >("list_study_sessions", { limit }),

  // --- calendar ---------------------------------------------------------
  listCalendarEvents: (from?: string | null, to?: string | null) =>
    call<CalendarEvent[]>("list_calendar_events", { from, to }),
  createCalendarEvent: (input: {
    courseId?: string | null;
    title: string;
    description?: string | null;
    location?: string | null;
    startsAt: string;
    endsAt?: string | null;
    allDay?: boolean;
    kind?: string;
  }) => call<CalendarEvent>("create_calendar_event", { input }),
  deleteCalendarEvent: (eventId: string) =>
    call<void>("delete_calendar_event", { eventId }),
  importIcs: (path: string) =>
    call<{ inserted: number; updated: number; recurring: number }>("import_ics", { path }),
  exportIcs: (path: string) => call<number>("export_ics", { path }),
  calendarMonth: (year: number, month: number) =>
    call<CalendarEvent[]>("calendar_month", { year, month }),

  // --- canvas -----------------------------------------------------------
  getCanvasConnection: () => call<CanvasConnection>("get_canvas_connection"),
  openCanvasLogin: () =>
    call<{ baseUrl: string; state: string; message: string }>("open_canvas_login"),
  openCanvasPage: (url: string) => call<void>("open_canvas_page", { url }),
  disconnectCanvas: () => call<void>("disconnect_canvas"),
  checkCanvasConnection: () => call<CanvasConnection>("check_canvas_connection"),
  getCanvasCapabilities: () => call<Capability[]>("get_canvas_capabilities"),
  syncAll: () => call<SyncRun>("sync_all"),
  syncCourse: (courseId: string) => call<SyncRun>("sync_course", { courseId }),
  getSyncHistory: (limit = 30) => call<SyncRun[]>("get_sync_history", { limit }),
  syncHistoryDetailed: (limit = 30) =>
    call<{ run: SyncRun; summary: string }[]>("sync_history_detailed", { limit }),
  getSyncDiagnostics: () =>
    call<{
      errors: { runId: string; resource: string | null; message: string; retryable: boolean }[];
      log: unknown;
    }>("get_sync_diagnostics"),
  exportSyncLog: (path: string) => call<void>("export_sync_log", { path }),
  listCourseFiles: (courseId?: string | null) =>
    call<CourseFileGroup[]>("list_course_files", { courseId: courseId ?? null }),
  downloadCanvasFile: (fileId: string) =>
    call<CanvasFile>("download_canvas_file", { fileId }),
  downloadCourseFiles: (courseId?: string | null) =>
    call<DownloadReport>("download_course_files", { courseId: courseId ?? null }),

  canvasIntegrationInfo: () =>
    call<{
      baseUrl: string;
      authModel: string;
      fileDownloadSupported: boolean;
      notes: string[];
    }>("canvas_integration_info"),

  // --- data -------------------------------------------------------------
  exportUserData: (path: string) => call<number>("export_user_data", { path }),
  importUserData: (path: string) =>
    call<{
      coursesAdded: number;
      plansAdded: number;
      tasksAdded: number;
      skipped: number;
    }>("import_user_data", { path }),
  backupDatabase: () => call<{ path: string; bytes: number }>("backup_database"),
  cacheInfo: () => call<CacheInfo>("cache_info"),
  clearCachedFiles: () => call<number>("clear_cached_files"),
  deleteCourseData: (courseId: string) => call<number>("delete_course_data", { courseId }),
  resetApplicationData: (confirm: string) =>
    call<void>("reset_application_data", { confirm }),
  storageOverview: () =>
    call<{
      dataDir: string;
      materialsDir: string;
      backupsDir: string;
      secretsBacking: string;
      secretsSecure: boolean;
    }>("storage_overview"),
};

export type Api = typeof api;
