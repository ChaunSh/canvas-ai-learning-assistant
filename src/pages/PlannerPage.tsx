import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarClock, Check, Lock, LockOpen, Plus, RefreshCw, Sparkles, Trash2 } from "lucide-react";

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
  Select,
  Switch,
  Textarea,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { cn, errorMessage, formatDate, formatDateTime, toDateInput } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { StudyTask } from "@/types/api";

const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"];

export function PlannerPage() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [generating, setGenerating] = useState(false);
  const [kind, setKind] = useState("weekly");
  const [startDate, setStartDate] = useState(toDateInput(new Date()));
  const [endDate, setEndDate] = useState(toDateInput(new Date(Date.now() + 13 * 86400_000)));
  const [creatingTask, setCreatingTask] = useState(false);
  const [prefsOpen, setPrefsOpen] = useState(false);

  const plans = useQuery({ queryKey: ["plans"], queryFn: () => api.listStudyPlans() });
  const preferences = useQuery({ queryKey: ["preferences"], queryFn: () => api.getStudyPreferences() });
  const deadlines = useQuery({ queryKey: ["deadlines"], queryFn: () => api.upcomingDeadlines(30, 20) });

  const activePlan = useMemo(
    () => (plans.data ?? []).find((plan) => plan.status === "active") ?? (plans.data ?? [])[0],
    [plans.data],
  );

  const tasks = useQuery({
    queryKey: ["tasks", activePlan?.id],
    queryFn: () => api.listStudyTasks({ planId: activePlan?.id ?? null, includeDone: true }),
    enabled: plans.isSuccess,
  });

  const generate = useMutation({
    mutationFn: () =>
      api.generateStudyPlan({
        kind,
        startDate,
        endDate,
        includeAssignments: true,
        includeMaterials: true,
      }),
    onSuccess: (result) => {
      toast(
        "success",
        `已生成 ${result.scheduled.length} 个学习块`,
        result.unscheduled.length > 0 ? `${result.unscheduled.length} 项未能安排` : undefined,
      );
      setGenerating(false);
      void queryClient.invalidateQueries({ queryKey: ["plans"] });
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard"] });
    },
    onError: (error) => toast("error", "生成计划失败", errorMessage(error)),
  });

  const replan = useMutation({
    mutationFn: (planId: string) => api.replanSchedule(planId),
    onSuccess: (result) => {
      toast("success", "已重新规划", `保留锁定与手动任务；安排 ${result.scheduled.length} 个学习块`);
      void queryClient.invalidateQueries({ queryKey: ["plans"] });
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
    },
    onError: (error) => toast("error", "重新规划失败", errorMessage(error)),
  });

  const toggleTask = useMutation({
    mutationFn: ({ id, done }: { id: string; done: boolean }) => api.completeStudyTask(id, done),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["plans"] });
      void queryClient.invalidateQueries({ queryKey: ["dashboard"] });
    },
    onError: (error) => toast("error", "更新任务失败", errorMessage(error)),
  });

  const toggleLock = useMutation({
    mutationFn: ({ id, locked }: { id: string; locked: boolean }) =>
      api.updateStudyTask(id, { locked }),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["tasks"] }),
    onError: (error) => toast("error", "更新失败", errorMessage(error)),
  });

  const removeTask = useMutation({
    mutationFn: (id: string) => api.deleteStudyTask(id),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["tasks"] }),
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  const grouped = useMemo(() => {
    const map = new Map<string, StudyTask[]>();
    for (const task of tasks.data ?? []) {
      const key = task.scheduledStart ? task.scheduledStart.slice(0, 10) : "unscheduled";
      const bucket = map.get(key) ?? [];
      bucket.push(task);
      map.set(key, bucket);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [tasks.data]);

  const openCount = (tasks.data ?? []).filter((task) => task.status === "todo" || task.status === "in_progress").length;
  const doneCount = (tasks.data ?? []).filter((task) => task.status === "done").length;

  return (
    <PageBody>
      <PageHeader
        title={t("planner.title")}
        subtitle={activePlan ? `${activePlan.name} · ${activePlan.startDate} → ${activePlan.endDate}` : t("planner.emptyHint")}
        actions={
          <>
            <Button variant="secondary" size="sm" onClick={() => setPrefsOpen(true)}>
              {t("planner.preferences")}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={!activePlan}
              loading={replan.isPending}
              onClick={() => activePlan && replan.mutate(activePlan.id)}
              icon={<RefreshCw className="size-4" />}
            >
              {t("planner.replan")}
            </Button>
            <Button variant="primary" size="sm" onClick={() => setGenerating(true)} icon={<Sparkles className="size-4" />}>
              {t("planner.generate")}
            </Button>
          </>
        }
      />

      <div className="mb-4 grid grid-cols-3 gap-3">
        <Card>
          <CardBody className="p-3">
            <p className="text-lg font-semibold">{openCount}</p>
            <p className="text-[11px] text-muted">{t("planner.openTasks")}</p>
          </CardBody>
        </Card>
        <Card>
          <CardBody className="p-3">
            <p className="text-lg font-semibold">{doneCount}</p>
            <p className="text-[11px] text-muted">{t("planner.doneTasks")}</p>
          </CardBody>
        </Card>
        <Card>
          <CardBody className="p-3">
            <p className="text-lg font-semibold">{preferences.data?.dailyMinutes ?? "—"}</p>
            <p className="text-[11px] text-muted">{t("planner.dailyMinutes")}</p>
          </CardBody>
        </Card>
      </div>

      <div className="grid gap-4 lg:grid-cols-[1fr_320px]">
        <div className="space-y-3">
          {tasks.isLoading ? <LoadingBlock /> : null}
          {tasks.isError ? (
            <ErrorState message="无法加载任务" detail={errorMessage(tasks.error)} onRetry={() => void tasks.refetch()} />
          ) : null}

          {!tasks.isLoading && (tasks.data ?? []).length === 0 ? (
            <Card>
              <EmptyState
                icon={<CalendarClock className="size-5" />}
                title={t("planner.empty")}
                description={t("planner.needsWork")}
                action={
                  <Button variant="primary" size="sm" onClick={() => setGenerating(true)}>
                    {t("planner.generate")}
                  </Button>
                }
              />
            </Card>
          ) : null}

          {grouped.map(([day, items]) => (
            <Card key={day}>
              <CardHeader
                title={day === "unscheduled" ? "未排期" : formatDate(`${day}T00:00:00Z`, locale)}
                subtitle={`${items.length} 项 · ${items.reduce((sum, task) => sum + task.estimatedMinutes, 0)} ${t("common.minutes")}`}
              />
              <CardBody className="p-0">
                <ul className="divide-y divide-[color:var(--app-border)]">
                  {items.map((task) => (
                    <li key={task.id} className="flex items-start gap-2 px-3 py-2.5">
                      <button
                        onClick={() => toggleTask.mutate({ id: task.id, done: task.status !== "done" })}
                        className={cn(
                          "mt-0.5 grid size-5 shrink-0 place-items-center rounded border transition-colors",
                          task.status === "done"
                            ? "border-emerald-500 bg-emerald-500 text-white"
                            : "border-[color:var(--app-border)] hover:border-brand-500",
                        )}
                        aria-label={t("planner.complete")}
                      >
                        {task.status === "done" ? <Check className="size-3" /> : null}
                      </button>
                      <div className="min-w-0 flex-1">
                        <p className={cn("truncate text-sm", task.status === "done" && "text-muted line-through")}>
                          {task.title}
                        </p>
                        <p className="truncate text-[11px] text-muted">
                          {task.scheduledStart ? `${formatDateTime(task.scheduledStart, locale)} – ${task.scheduledEnd ? formatDateTime(task.scheduledEnd, locale).split(" ").pop() : ""}` : "未排期"}
                          {task.courseName ? ` · ${task.courseName}` : ""}
                          {task.deadline ? ` · 截止 ${formatDateTime(task.deadline, locale)}` : ""}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-0.5">
                        <Badge tone={task.generatedBy === "user" ? "info" : "neutral"}>{task.estimatedMinutes}{t("common.minutes")}</Badge>
                        <Button
                          variant="ghost"
                          size="sm"
                          title={task.locked ? t("planner.unlock") : t("planner.lock")}
                          onClick={() => toggleLock.mutate({ id: task.id, locked: !task.locked })}
                        >
                          {task.locked ? <Lock className="size-3.5 text-amber-500" /> : <LockOpen className="size-3.5" />}
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => removeTask.mutate(task.id)} title={t("common.delete")}>
                          <Trash2 className="size-3.5" />
                        </Button>
                      </div>
                    </li>
                  ))}
                </ul>
              </CardBody>
            </Card>
          ))}

          <Button variant="secondary" size="sm" onClick={() => setCreatingTask(true)} icon={<Plus className="size-4" />}>
            手动添加任务
          </Button>
        </div>

        <div className="space-y-3">
          <Card>
            <CardHeader title={t("planner.deadlines")} />
            <CardBody className="p-0">
              {deadlines.isLoading ? <LoadingBlock /> : (deadlines.data ?? []).length === 0 ? (
                <EmptyState title="没有临近截止" />
              ) : (
                <ul className="divide-y divide-[color:var(--app-border)]">
                  {deadlines.data!.map((deadline) => (
                    <li key={`${deadline.kind}-${deadline.id}`} className="px-3 py-2">
                      <p className="truncate text-xs font-medium">{deadline.title}</p>
                      <p className="text-[10px] text-muted">
                        {deadline.courseName ?? ""} · {formatDateTime(deadline.dueAt, locale)}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </CardBody>
          </Card>

          {preferences.data ? (
            <Card>
              <CardHeader title={t("planner.preferences")} />
              <CardBody className="space-y-1 text-xs">
                <Row label={t("planner.dailyMinutes")} value={String(preferences.data.dailyMinutes)} />
                <Row label={t("planner.sessionMinutes")} value={String(preferences.data.sessionMinutes)} />
                <Row label={t("planner.window")} value={`${preferences.data.preferredStart} – ${preferences.data.preferredEnd}`} />
                <Row label={t("planner.studyDays")} value={preferences.data.studyDays.map((day) => WEEKDAYS[day - 1]).join(" ")} />
                <Row label={t("planner.timezone")} value={preferences.data.timezone} />
                <Row label={t("planner.avoidWeekends")} value={preferences.data.avoidWeekends ? t("common.yes") : t("common.no")} />
              </CardBody>
            </Card>
          ) : null}
        </div>
      </div>

      <Dialog
        open={generating}
        onClose={() => setGenerating(false)}
        title={t("planner.generate")}
        description="根据作业截止时间、课件数量与可用时间生成确定性时间表；AI 不直接决定时间槽。"
        footer={
          <>
            <Button variant="secondary" onClick={() => setGenerating(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" loading={generate.isPending} onClick={() => generate.mutate()}>
              {t("planner.generate")}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <Field label={t("planner.kind")}>
            <Select value={kind} onChange={(event) => setKind(event.target.value)}>
              <option value="daily">{t("planner.kindDaily")}</option>
              <option value="weekly">{t("planner.kindWeekly")}</option>
              <option value="exam">{t("planner.kindExam")}</option>
              <option value="assignment">{t("planner.kindAssignment")}</option>
              <option value="custom">{t("planner.kindCustom")}</option>
            </Select>
          </Field>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label={t("planner.start")}>
              <Input type="date" value={startDate} onChange={(event) => setStartDate(event.target.value)} />
            </Field>
            <Field label={t("planner.end")}>
              <Input type="date" value={endDate} onChange={(event) => setEndDate(event.target.value)} />
            </Field>
          </div>
        </div>
      </Dialog>

      <PreferencesDialog open={prefsOpen} onClose={() => setPrefsOpen(false)} />
      <CreateTaskDialog open={creatingTask} planId={activePlan?.id ?? null} onClose={() => setCreatingTask(false)} />
    </PageBody>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-muted">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}

function PreferencesDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const t = useT();
  const queryClient = useQueryClient();
  const preferences = useQuery({ queryKey: ["preferences"], queryFn: () => api.getStudyPreferences(), enabled: open });
  const [draft, setDraft] = useState<Record<string, unknown>>({});

  const save = useMutation({
    mutationFn: () => api.updateStudyPreferences({ ...(preferences.data ?? {}), ...draft }),
    onSuccess: () => {
      toast("success", "偏好已保存");
      void queryClient.invalidateQueries({ queryKey: ["preferences"] });
      onClose();
    },
    onError: (error) => toast("error", "保存失败", errorMessage(error)),
  });

  const value = { ...(preferences.data ?? {}), ...draft } as Record<string, unknown>;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("planner.preferences")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" loading={save.isPending} onClick={() => save.mutate()}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      {preferences.isLoading ? (
        <LoadingBlock />
      ) : (
        <div className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-3">
            <Field label={t("planner.dailyMinutes")}>
              <Input
                type="number"
                min={15}
                max={960}
                value={String(value.dailyMinutes ?? 120)}
                onChange={(event) => setDraft((current) => ({ ...current, dailyMinutes: Number(event.target.value) }))}
              />
            </Field>
            <Field label={t("planner.sessionMinutes")}>
              <Input
                type="number"
                min={10}
                max={240}
                value={String(value.sessionMinutes ?? 50)}
                onChange={(event) => setDraft((current) => ({ ...current, sessionMinutes: Number(event.target.value) }))}
              />
            </Field>
            <Field label={t("planner.breakMinutes")}>
              <Input
                type="number"
                min={0}
                max={120}
                value={String(value.breakMinutes ?? 10)}
                onChange={(event) => setDraft((current) => ({ ...current, breakMinutes: Number(event.target.value) }))}
              />
            </Field>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="开始时间">
              <Input
                type="time"
                value={String(value.preferredStart ?? "19:00")}
                onChange={(event) => setDraft((current) => ({ ...current, preferredStart: event.target.value }))}
              />
            </Field>
            <Field label="结束时间">
              <Input
                type="time"
                value={String(value.preferredEnd ?? "22:30")}
                onChange={(event) => setDraft((current) => ({ ...current, preferredEnd: event.target.value }))}
              />
            </Field>
          </div>
          <Field label={t("planner.studyDays")}>
            <div className="flex flex-wrap gap-1">
              {WEEKDAYS.map((label, index) => {
                const day = index + 1;
                const days = (value.studyDays as number[]) ?? [1, 2, 3, 4, 5, 6, 7];
                const active = days.includes(day);
                return (
                  <button
                    key={label}
                    type="button"
                    onClick={() => {
                      const next = active ? days.filter((d) => d !== day) : [...days, day].sort();
                      setDraft((current) => ({ ...current, studyDays: next }));
                    }}
                    className={cn(
                      "size-9 rounded-lg border text-xs",
                      active ? "border-brand-500 bg-brand-500/12 text-brand-700 dark:text-brand-300" : "border-app text-muted",
                    )}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </Field>
          <Field label={t("planner.timezone")} hint="IANA 时区，例如 Asia/Shanghai">
            <Input
              value={String(value.timezone ?? "Asia/Shanghai")}
              onChange={(event) => setDraft((current) => ({ ...current, timezone: event.target.value }))}
            />
          </Field>
          <Switch
            checked={Boolean(value.avoidWeekends ?? false)}
            onChange={(checked) => setDraft((current) => ({ ...current, avoidWeekends: checked }))}
            label={t("planner.avoidWeekends")}
          />
        </div>
      )}
    </Dialog>
  );
}

function CreateTaskDialog({
  open,
  planId,
  onClose,
}: {
  open: boolean;
  planId: string | null;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const [title, setTitle] = useState("");
  const [minutes, setMinutes] = useState("45");
  const [start, setStart] = useState("");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!title.trim()) {
      toast("error", "请填写任务标题");
      return;
    }
    setBusy(true);
    try {
      const startIso = start ? new Date(start).toISOString() : null;
      const endIso = start ? new Date(new Date(start).getTime() + Number(minutes) * 60_000).toISOString() : null;
      await api.createStudyTask({
        planId,
        title: title.trim(),
        description: notes.trim() || null,
        estimatedMinutes: Number(minutes) || 30,
        scheduledStart: startIso,
        scheduledEnd: endIso,
      });
      toast("success", "任务已添加");
      setTitle("");
      setNotes("");
      onClose();
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
    } catch (error) {
      toast("error", "添加任务失败", errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="手动添加任务"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" loading={busy} onClick={() => void submit()}>
            添加
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label="标题">
          <Input value={title} onChange={(event) => setTitle(event.target.value)} autoFocus />
        </Field>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="时长（分钟）">
            <Input type="number" min={5} value={minutes} onChange={(event) => setMinutes(event.target.value)} />
          </Field>
          <Field label="开始时间（可选）">
            <Input type="datetime-local" value={start} onChange={(event) => setStart(event.target.value)} />
          </Field>
        </div>
        <Field label="备注">
          <Textarea value={notes} onChange={(event) => setNotes(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
