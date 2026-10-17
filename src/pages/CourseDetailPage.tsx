import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, BookOpen, FileText, Plus, RefreshCw } from "lucide-react";

import { LinkButton } from "@/components/LinkButton";
import { PageBody, PageHeader } from "@/components/layout";
import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Dialog,
  EmptyState,
  ErrorState,
  Field,
  Input,
  LoadingBlock,
  Tab,
  Tabs,
  Textarea,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { errorMessage, formatDateTime } from "@/lib/utils";
import { toast } from "@/stores/toasts";

const TABS = ["materials", "assignments", "modules", "grades", "discussions", "pages"] as const;
type TabKey = (typeof TABS)[number];

export function CourseDetailPage() {
  const { courseId = "" } = useParams();
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<TabKey>("materials");
  const [creatingAssignment, setCreatingAssignment] = useState(false);
  const [newModule, setNewModule] = useState("");

  const detail = useQuery({
    queryKey: ["course", courseId],
    queryFn: () => api.getCourseDetail(courseId),
    enabled: Boolean(courseId),
  });

  const materials = useQuery({
    queryKey: ["materials", "course", courseId],
    queryFn: () => api.listMaterials({ courseId, limit: 200 }),
    enabled: Boolean(courseId) && tab === "materials",
  });

  const assignments = useQuery({
    queryKey: ["assignments", courseId],
    queryFn: () => api.listAssignments(courseId, false, 200),
    enabled: Boolean(courseId) && tab === "assignments",
  });

  const grades = useQuery({
    queryKey: ["grades", courseId],
    queryFn: () => api.listGrades(courseId),
    enabled: Boolean(courseId) && tab === "grades",
  });

  const discussions = useQuery({
    queryKey: ["discussions", courseId],
    queryFn: () => api.listDiscussions(courseId),
    enabled: Boolean(courseId) && tab === "discussions",
  });

  const pages = useQuery({
    queryKey: ["pages", courseId],
    queryFn: () => api.listCoursePages(courseId),
    enabled: Boolean(courseId) && tab === "pages",
  });

  const sync = useMutation({
    mutationFn: () => api.syncCourse(courseId),
    onSuccess: (run) => {
      toast(
        run.status === "completed" ? "success" : "error",
        run.status === "completed" ? "同步完成" : "同步失败",
        run.error ?? undefined,
      );
      void queryClient.invalidateQueries();
    },
    onError: (error) => toast("error", "同步失败", errorMessage(error)),
  });

  // 课件不再支持本地上传：课程文件统一来自 Canvas，在「课件文件」里下载。

  const addModule = useMutation({
    mutationFn: (name: string) => api.createCourseModule({ courseId, name }),
    onSuccess: () => {
      setNewModule("");
      void queryClient.invalidateQueries({ queryKey: ["course", courseId] });
      toast("success", "章节已添加");
    },
    onError: (error) => toast("error", "添加章节失败", errorMessage(error)),
  });

  if (detail.isLoading) {
    return (
      <PageBody>
        <LoadingBlock />
      </PageBody>
    );
  }

  if (detail.isError || !detail.data) {
    return (
      <PageBody>
        <ErrorState
          message="无法加载课程"
          detail={errorMessage(detail.error)}
          onRetry={() => void detail.refetch()}
        />
      </PageBody>
    );
  }

  const { course, modules } = detail.data;

  return (
    <PageBody>
      <Link to="/courses" className="mb-3 inline-flex items-center gap-1 text-xs text-muted hover:underline">
        <ArrowLeft className="size-3.5" />
        {t("nav.courses")}
      </Link>

      <PageHeader
        title={course.name}
        subtitle={[course.code, course.term, course.instructor].filter(Boolean).join(" · ") || "未填写课程信息"}
        actions={
          <>
            {course.source === "canvas" ? (
              <Button
                variant="secondary"
                size="sm"
                loading={sync.isPending}
                onClick={() => sync.mutate()}
                icon={<RefreshCw className="size-4" />}
              >
                {t("courses.syncOne")}
              </Button>
            ) : null}
            <LinkButton to="/materials" variant="primary" size="sm">
              {t("materials.title")}
            </LinkButton>
          </>
        }
      />

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <Badge tone={course.source === "canvas" ? "brand" : "neutral"}>
          {course.source === "canvas" ? "Canvas 同步" : "手动课程"}
        </Badge>
        <Badge tone="neutral">{course.materialCount} 份课件</Badge>
        <Badge tone={course.openTaskCount > 0 ? "warning" : "neutral"}>{course.openTaskCount} 项待办</Badge>
        {course.lastSyncedAt ? (
          <span className="text-[11px] text-muted">最近同步：{formatDateTime(course.lastSyncedAt, locale)}</span>
        ) : null}
      </div>

      <Tabs value={tab} onChange={(value) => setTab(value as TabKey)}>
        <Tab value="materials">{t("courses.materials")}</Tab>
        <Tab value="assignments">{t("courses.assignments")}</Tab>
        <Tab value="modules">{t("courses.modules")}</Tab>
        <Tab value="grades">{t("courses.grades")}</Tab>
        <Tab value="discussions">{t("courses.discussions")}</Tab>
        <Tab value="pages">{t("courses.pages")}</Tab>
      </Tabs>

      <div className="pt-4">
        {tab === "materials" ? (
          materials.isLoading ? (
            <LoadingBlock />
          ) : (materials.data ?? []).length === 0 ? (
            <Card>
              <EmptyState
                icon={<BookOpen className="size-5" />}
                title={t("materials.empty")}
                description={t("materials.emptyHint")}
                action={
                  <LinkButton to="/canvas" variant="primary" size="sm">
                    同步 Canvas 课件
                  </LinkButton>
                }
              />
            </Card>
          ) : (
            <div className="grid gap-2">
              {materials.data!.map((material) => (
                <Card key={material.id}>
                  <Link to={`/materials/${material.id}`} className="flex items-center justify-between gap-3 px-4 py-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{material.title}</p>
                      <p className="truncate text-[11px] text-muted">
                        {material.kind.toUpperCase()} · {material.pageCount || material.slideCount} 页 · {material.chunkCount} 片段
                      </p>
                    </div>
                    <Badge
                      tone={
                        material.indexStatus === "indexed"
                          ? "success"
                          : material.indexStatus === "failed"
                            ? "danger"
                            : "neutral"
                      }
                    >
                      {material.indexStatus}
                    </Badge>
                  </Link>
                </Card>
              ))}
            </div>
          )
        ) : null}

        {tab === "assignments" ? (
          <div className="space-y-3">
            <div className="flex justify-end">
              <Button size="sm" variant="primary" onClick={() => setCreatingAssignment(true)} icon={<Plus className="size-4" />}>
                {t("courses.assignments")}
              </Button>
            </div>
            {assignments.isLoading ? <LoadingBlock /> : null}
            {(assignments.data ?? []).length === 0 && !assignments.isLoading ? (
              <Card>
                <EmptyState title="暂无作业" description="同步 Canvas 或手动添加作业，以便生成学习计划。" />
              </Card>
            ) : (
              <Card>
                <ul className="divide-y divide-[color:var(--app-border)]">
                  {assignments.data!.map((assignment) => (
                    <li key={assignment.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                      <div className="min-w-0">
                        <p className="truncate text-sm">{assignment.title}</p>
                        <p className="text-[11px] text-muted">
                          {assignment.dueAt ? formatDateTime(assignment.dueAt, locale) : "无截止时间"}
                          {assignment.pointsPossible != null ? ` · ${assignment.pointsPossible} 分` : ""}
                        </p>
                      </div>
                      <Badge tone={assignment.submissionStatus === "graded" ? "success" : "neutral"}>
                        {assignment.submissionStatus}
                      </Badge>
                    </li>
                  ))}
                </ul>
              </Card>
            )}
          </div>
        ) : null}

        {tab === "modules" ? (
          <div className="space-y-3">
            <Card>
              <CardHeader title={t("courses.modules")} subtitle={`${modules.length} 个章节`} />
              <CardBody className="space-y-2">
                {modules.length === 0 ? (
                  <p className="text-sm text-muted">暂无章节。可手动添加，或从 Canvas 同步模块。</p>
                ) : (
                  <ol className="space-y-1.5">
                    {modules.map((module, index) => (
                      <li key={module.id} className="flex items-center justify-between gap-2 rounded-lg border border-app px-3 py-2">
                        <span className="truncate text-sm">
                          <span className="mr-2 font-mono text-xs text-muted">{index + 1}</span>
                          {module.name}
                        </span>
                        <Badge tone="neutral">{module.itemCount} 项</Badge>
                      </li>
                    ))}
                  </ol>
                )}
                <div className="flex gap-2 pt-1">
                  <Input
                    value={newModule}
                    onChange={(event) => setNewModule(event.target.value)}
                    placeholder="新章节名称"
                  />
                  <Button
                    variant="secondary"
                    onClick={() => newModule.trim() && addModule.mutate(newModule.trim())}
                    loading={addModule.isPending}
                  >
                    {t("courses.addModule")}
                  </Button>
                </div>
              </CardBody>
            </Card>
          </div>
        ) : null}

        {tab === "grades" ? (
          grades.isLoading ? (
            <LoadingBlock />
          ) : (grades.data ?? []).length === 0 ? (
            <Card>
              <EmptyState
                title="暂无成绩数据"
                description="若 Canvas 允许读取成绩，同步后会显示作业成绩与课程总评；否则只会显示手动录入的估算。"
              />
            </Card>
          ) : (
            <Card>
              <ul className="divide-y divide-[color:var(--app-border)]">
                {grades.data!.map((grade) => (
                  <li key={grade.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                    <div className="min-w-0">
                      <p className="truncate text-sm">{grade.assignmentName}</p>
                      <p className="text-[11px] text-muted">
                        {grade.source === "canvas" ? "来自 Canvas" : "手动录入（非官方成绩）"}
                      </p>
                    </div>
                    <div className="text-right">
                      <p className="text-sm font-medium">
                        {grade.score != null ? `${grade.score}${grade.pointsPossible ? ` / ${grade.pointsPossible}` : ""}` : grade.grade ?? "—"}
                      </p>
                    </div>
                  </li>
                ))}
              </ul>
            </Card>
          )
        ) : null}

        {tab === "discussions" ? (
          discussions.isLoading ? (
            <LoadingBlock />
          ) : (discussions.data ?? []).length === 0 ? (
            <Card>
              <EmptyState title="暂无讨论" description="仅当 Canvas 允许读取时才可显示讨论主题（只读）。" />
            </Card>
          ) : (
            <Card>
              <ul className="divide-y divide-[color:var(--app-border)]">
                {discussions.data!.map((topic) => (
                  <li key={topic.id} className="px-4 py-2.5">
                    <p className="truncate text-sm">{topic.title}</p>
                    <p className="mt-0.5 text-[11px] text-muted">
                      {topic.author ?? "未知作者"} · {topic.replyCount} 回复 ·{" "}
                      {topic.postedAt ? formatDateTime(topic.postedAt, locale) : "—"}
                    </p>
                  </li>
                ))}
              </ul>
            </Card>
          )
        ) : null}

        {tab === "pages" ? (
          pages.isLoading ? (
            <LoadingBlock />
          ) : (pages.data ?? []).length === 0 ? (
            <Card>
              <EmptyState icon={<FileText className="size-5" />} title="暂无课程页面" description="Canvas 课程页面会在同步后显示。" />
            </Card>
          ) : (
            <div className="grid gap-2">
              {pages.data!.map((page) => (
                <Card key={page.id}>
                  <CardBody>
                    <p className="text-sm font-medium">{page.title}</p>
                    <p className="mt-1 line-clamp-3 whitespace-pre-wrap text-xs text-muted">
                      {page.body?.slice(0, 400) ?? "（无正文）"}
                    </p>
                  </CardBody>
                </Card>
              ))}
            </div>
          )
        ) : null}
      </div>

      <CreateAssignmentDialog
        open={creatingAssignment}
        courseId={courseId}
        onClose={() => setCreatingAssignment(false)}
        onCreated={() => {
          setCreatingAssignment(false);
          void queryClient.invalidateQueries({ queryKey: ["assignments", courseId] });
        }}
      />
    </PageBody>
  );
}

function CreateAssignmentDialog({
  open,
  courseId,
  onClose,
  onCreated,
}: {
  open: boolean;
  courseId: string;
  onClose: () => void;
  onCreated: () => void;
}) {
  const t = useT();
  const [title, setTitle] = useState("");
  const [dueAt, setDueAt] = useState("");
  const [points, setPoints] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!title.trim()) {
      toast("error", "请填写作业标题");
      return;
    }
    setBusy(true);
    try {
      await api.createAssignment({
        courseId,
        title: title.trim(),
        description: description.trim() || null,
        dueAt: dueAt ? new Date(dueAt).toISOString() : null,
        pointsPossible: points ? Number(points) : null,
      });
      toast("success", "作业已添加");
      setTitle("");
      setDueAt("");
      setPoints("");
      setDescription("");
      onCreated();
    } catch (error) {
      toast("error", "添加作业失败", errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("courses.assignments")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" loading={busy} onClick={() => void submit()}>
            {t("common.create")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label={t("calendar.eventTitle")}>
          <Input value={title} onChange={(event) => setTitle(event.target.value)} autoFocus />
        </Field>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="截止时间">
            <Input type="datetime-local" value={dueAt} onChange={(event) => setDueAt(event.target.value)} />
          </Field>
          <Field label="满分">
            <Input type="number" min={0} value={points} onChange={(event) => setPoints(event.target.value)} />
          </Field>
        </div>
        <Field label={t("courses.description")}>
          <Textarea value={description} onChange={(event) => setDescription(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
