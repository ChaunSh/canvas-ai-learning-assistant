import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Archive, BookOpen, GraduationCap, Plus, Search, Trash2 } from "lucide-react";

import { LinkButton } from "@/components/LinkButton";
import { PageBody, PageHeader } from "@/components/layout";
import {
  Badge,
  Button,
  Card,
  CardBody,
  Dialog,
  EmptyState,
  ErrorState,
  Field,
  Input,
  LoadingBlock,
  Select,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { cn, errorMessage, formatRelative, percent } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { Course } from "@/types/api";

export function CoursesPage() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [includeArchived, setIncludeArchived] = useState(false);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<Course | null>(null);

  const courses = useQuery({
    queryKey: ["courses", includeArchived],
    queryFn: () => api.listCourses(includeArchived),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["courses"] });
    void queryClient.invalidateQueries({ queryKey: ["dashboard"] });
  };

  const createMutation = useMutation({
    mutationFn: (input: { name: string; code?: string; term?: string; instructor?: string }) =>
      api.createCourse(input),
    onSuccess: (course) => {
      toast("success", "课程已创建", course.name);
      setCreating(false);
      invalidate();
    },
    onError: (error) => toast("error", "创建课程失败", errorMessage(error)),
  });

  const archiveMutation = useMutation({
    mutationFn: ({ id, archived }: { id: string; archived: boolean }) =>
      api.updateCourse(id, { archived }),
    onSuccess: () => invalidate(),
    onError: (error) => toast("error", "更新失败", errorMessage(error)),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.deleteCourseData(id),
    onSuccess: () => {
      toast("success", "课程已删除");
      setPendingDelete(null);
      invalidate();
    },
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  const filtered = useMemo(() => {
    const list = courses.data ?? [];
    if (!query.trim()) return list;
    const needle = query.trim().toLowerCase();
    return list.filter(
      (course) =>
        course.name.toLowerCase().includes(needle) ||
        (course.code ?? "").toLowerCase().includes(needle) ||
        (course.term ?? "").toLowerCase().includes(needle),
    );
  }, [courses.data, query]);

  const grouped = useMemo(() => {
    const map = new Map<string, Course[]>();
    for (const course of filtered) {
      const term = course.term?.trim() || "未分学期";
      const bucket = map.get(term) ?? [];
      bucket.push(course);
      map.set(term, bucket);
    }
    return [...map.entries()].sort((a, b) => b[0].localeCompare(a[0]));
  }, [filtered]);

  return (
    <PageBody>
      <PageHeader
        title={t("courses.title")}
        subtitle={`${courses.data?.length ?? 0} 门课程`}
        actions={
          <>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setIncludeArchived((value) => !value)}
              icon={<Archive className="size-4" />}
            >
              {includeArchived ? "隐藏归档" : "显示归档"}
            </Button>
            <Button variant="primary" size="sm" onClick={() => setCreating(true)} icon={<Plus className="size-4" />} data-testid="new-course">
              {t("courses.new")}
            </Button>
          </>
        }
      />

      <div className="relative mb-4 max-w-md">
        <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" />
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={`${t("common.search")} ${t("courses.title")}`}
          className="pl-9"
        />
      </div>

      {courses.isLoading ? <LoadingBlock /> : null}
      {courses.isError ? (
        <ErrorState
          message="无法加载课程"
          detail={errorMessage(courses.error)}
          onRetry={() => void courses.refetch()}
          retryLabel={t("common.retry")}
        />
      ) : null}

      {!courses.isLoading && !courses.isError && filtered.length === 0 ? (
        <Card>
          <EmptyState
            icon={<GraduationCap className="size-5" />}
            title={t("courses.empty")}
            description={t("home.noCoursesHint")}
            action={
              <div className="flex gap-2">
                <Button variant="secondary" size="sm" onClick={() => setCreating(true)}>
                  {t("courses.new")}
                </Button>
                <LinkButton to="/canvas" variant="primary" size="sm">
                  {t("canvas.connect")}
                </LinkButton>
              </div>
            }
          />
        </Card>
      ) : null}

      {grouped.map(([term, items]) => (
        <section key={term} className="mb-5">
          <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">{term}</h2>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {items.map((course) => (
              <Card key={course.id} className={cn("overflow-hidden", course.archived && "opacity-70")}>
                <CardBody className="space-y-3">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <Link to={`/courses/${course.id}`} className="block truncate text-sm font-semibold hover:underline">
                        {course.name}
                      </Link>
                      <p className="mt-0.5 truncate text-[11px] text-muted">
                        {course.code ?? "无课程代码"}
                        {course.instructor ? ` · ${course.instructor}` : ""}
                      </p>
                    </div>
                    <div className="flex shrink-0 flex-col items-end gap-1">
                      <Badge tone={course.source === "canvas" ? "brand" : "neutral"}>
                        {course.source === "canvas" ? "Canvas" : "本地"}
                      </Badge>
                      {course.archived ? <Badge tone="warning">{t("courses.archived")}</Badge> : null}
                    </div>
                  </div>

                  <div className="flex items-center gap-3 text-[11px] text-muted">
                    <span className="inline-flex items-center gap-1">
                      <BookOpen className="size-3.5" />
                      {course.materialCount}
                    </span>
                    <span>{course.openTaskCount} 待办</span>
                    {course.dueSoonCount > 0 ? <Badge tone="warning">{course.dueSoonCount} 即将截止</Badge> : null}
                  </div>

                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-[color:var(--app-hover)]">
                    <div
                      className="h-full rounded-full bg-brand-600 dark:bg-brand-400"
                      style={{ width: `${percent(course.progress)}%` }}
                    />
                  </div>

                  <div className="flex items-center justify-between gap-2 pt-0.5">
                    <span className="truncate text-[10px] text-muted">
                      {course.lastSyncedAt ? `同步于 ${formatRelative(course.lastSyncedAt, locale)}` : "未同步"}
                    </span>
                    <div className="flex items-center gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => archiveMutation.mutate({ id: course.id, archived: !course.archived })}
                        title={course.archived ? t("courses.unarchive") : t("courses.archive")}
                      >
                        <Archive className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setPendingDelete(course)}
                        title={t("courses.delete")}
                      >
                        <Trash2 className="size-3.5" />
                      </Button>
                    </div>
                  </div>
                </CardBody>
              </Card>
            ))}
          </div>
        </section>
      ))}

      <CreateCourseDialog
        open={creating}
        busy={createMutation.isPending}
        onClose={() => setCreating(false)}
        onSubmit={(values) => createMutation.mutate(values)}
      />

      <Dialog
        open={pendingDelete !== null}
        onClose={() => setPendingDelete(null)}
        title={t("courses.delete")}
        description={t("courses.deleteWarning")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setPendingDelete(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              loading={deleteMutation.isPending}
              onClick={() => pendingDelete && deleteMutation.mutate(pendingDelete.id)}
            >
              {t("common.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm">
          课程：<span className="font-medium">{pendingDelete?.name}</span>
        </p>
      </Dialog>
    </PageBody>
  );
}

function CreateCourseDialog({
  open,
  busy,
  onClose,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  onClose: () => void;
  onSubmit: (values: { name: string; code?: string; term?: string; instructor?: string }) => void;
}) {
  const t = useT();
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [term, setTerm] = useState("2026 Fall");
  const [instructor, setInstructor] = useState("");

  const submit = () => {
    if (!name.trim()) {
      toast("error", "请填写课程名称");
      return;
    }
    onSubmit({
      name: name.trim(),
      code: code.trim() || undefined,
      term: term.trim() || undefined,
      instructor: instructor.trim() || undefined,
    });
    setName("");
    setCode("");
    setInstructor("");
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("courses.new")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" loading={busy} onClick={submit} data-testid="create-course-submit">
            {t("common.create")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label={t("courses.name")}>
          <Input
            value={name}
            onChange={(event) => setName(event.target.value)}
            autoFocus
            placeholder="例如：线性代数"
            data-testid="course-name"
          />
        </Field>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={`${t("courses.code")}（${t("common.optional")}）`}>
            <Input value={code} onChange={(event) => setCode(event.target.value)} placeholder="MATH 101" data-testid="course-code" />
          </Field>
          <Field label={t("courses.term")}>
            <Select value={term} onChange={(event) => setTerm(event.target.value)}>
              <option value="2026 Fall">2026 Fall</option>
              <option value="2026 Spring">2026 Spring</option>
              <option value="2027 Spring">2027 Spring</option>
              <option value="">未分学期</option>
            </Select>
          </Field>
        </div>
        <Field label={`${t("courses.instructor")}（${t("common.optional")}）`}>
          <Input value={instructor} onChange={(event) => setInstructor(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
