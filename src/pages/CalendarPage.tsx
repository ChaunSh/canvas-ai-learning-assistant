import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { CalendarPlus, Download, Upload } from "lucide-react";

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
  Tab,
  Tabs,
  Textarea,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { cn, errorMessage, formatDateTime } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { CalendarEvent } from "@/types/api";

const KIND_LABEL: Record<CalendarEvent["kind"], string> = {
  event: "日程",
  assignment_due: "作业截止",
  exam: "考试",
  study: "学习",
  personal: "个人",
};

export function CalendarPage() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const today = new Date();
  const [cursor, setCursor] = useState({ year: today.getFullYear(), month: today.getMonth() + 1 });
  const [tab, setTab] = useState("month");
  const [creating, setCreating] = useState(false);

  const monthEvents = useQuery({
    queryKey: ["calendar-month", cursor.year, cursor.month],
    queryFn: () => api.calendarMonth(cursor.year, cursor.month),
  });

  const agenda = useQuery({
    queryKey: ["calendar-agenda"],
    queryFn: () => api.listCalendarEvents(new Date(Date.now() - 7 * 86400_000).toISOString(), new Date(Date.now() + 60 * 86400_000).toISOString()),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteCalendarEvent(id),
    onSuccess: () => {
      toast("success", "日程已删除");
      void queryClient.invalidateQueries({ queryKey: ["calendar-month"] });
      void queryClient.invalidateQueries({ queryKey: ["calendar-agenda"] });
    },
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  const importIcs = async () => {
    const picked = await openDialog({ multiple: false, filters: [{ name: "iCalendar", extensions: ["ics"] }] });
    if (!picked || Array.isArray(picked)) return;
    try {
      const result = await api.importIcs(picked);
      toast(
        "success",
        `导入 ${result.inserted} 个新日程，更新 ${result.updated} 个`,
        result.recurring > 0 ? t("calendar.recurring", { n: result.recurring }) : undefined,
      );
      void queryClient.invalidateQueries({ queryKey: ["calendar-month"] });
      void queryClient.invalidateQueries({ queryKey: ["calendar-agenda"] });
    } catch (error) {
      toast("error", "导入失败", errorMessage(error));
    }
  };

  const exportIcs = async () => {
    const target = await saveDialog({ defaultPath: "canvas-calendar.ics", filters: [{ name: "iCalendar", extensions: ["ics"] }] });
    if (!target) return;
    try {
      const count = await api.exportIcs(target);
      toast("success", `已导出 ${count} 个日程`, target);
    } catch (error) {
      toast("error", "导出失败", errorMessage(error));
    }
  };

  const grid = useMemo(() => buildMonthGrid(cursor.year, cursor.month, monthEvents.data ?? []), [cursor, monthEvents.data]);

  const shift = (delta: number) => {
    setCursor((current) => {
      const month = current.month + delta;
      if (month < 1) return { year: current.year - 1, month: 12 };
      if (month > 12) return { year: current.year + 1, month: 1 };
      return { year: current.year, month };
    });
  };

  return (
    <PageBody>
      <PageHeader
        title={t("calendar.title")}
        subtitle={`${cursor.year} 年 ${cursor.month} 月`}
        actions={
          <>
            <Button variant="secondary" size="sm" onClick={() => void importIcs()} icon={<Upload className="size-4" />}>
              {t("calendar.importIcs")}
            </Button>
            <Button variant="secondary" size="sm" onClick={() => void exportIcs()} icon={<Download className="size-4" />}>
              {t("calendar.exportIcs")}
            </Button>
            <Button variant="primary" size="sm" onClick={() => setCreating(true)} icon={<CalendarPlus className="size-4" />}>
              {t("calendar.newEvent")}
            </Button>
          </>
        }
      />

      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={() => shift(-1)}>
            ‹
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setCursor({ year: today.getFullYear(), month: today.getMonth() + 1 })}>
            {t("common.today")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => shift(1)}>
            ›
          </Button>
        </div>
        <Tabs value={tab} onChange={setTab} className="border-0">
          <Tab value="month">{t("calendar.month")}</Tab>
          <Tab value="agenda">{t("calendar.list")}</Tab>
        </Tabs>
      </div>

      {monthEvents.isError ? (
        <ErrorState message="无法加载日历" detail={errorMessage(monthEvents.error)} onRetry={() => void monthEvents.refetch()} />
      ) : null}

      {tab === "month" ? (
        <Card className="overflow-hidden">
          <div className="grid grid-cols-7 border-b border-app bg-[color:var(--app-hover)]/40">
            {["一", "二", "三", "四", "五", "六", "日"].map((label) => (
              <div key={label} className="px-1 py-1.5 text-center text-[11px] font-medium text-muted">
                {label}
              </div>
            ))}
          </div>
          <div className="grid grid-cols-7">
            {grid.map((cell) => (
              <div
                key={cell.key}
                className={cn(
                  "min-h-20 border-b border-r border-app p-1 align-top last:border-r-0",
                  !cell.inMonth && "bg-[color:var(--app-bg)]/60",
                  cell.isToday && "bg-brand-500/8",
                )}
              >
                <div className="mb-0.5 flex items-center justify-between">
                  <span className={cn("text-[11px]", cell.isToday ? "font-semibold text-brand-700 dark:text-brand-300" : "text-muted")}>
                    {cell.day}
                  </span>
                </div>
                <div className="space-y-0.5">
                  {cell.events.slice(0, 3).map((event) => (
                    <div
                      key={event.id}
                      title={`${event.title} · ${formatDateTime(event.startsAt, locale)}`}
                      className={cn(
                        "truncate rounded px-1 py-0.5 text-[10px]",
                        event.kind === "exam"
                          ? "bg-red-500/15 text-red-700 dark:text-red-400"
                          : event.kind === "assignment_due"
                            ? "bg-amber-500/15 text-amber-700 dark:text-amber-400"
                            : "bg-brand-500/12 text-brand-700 dark:text-brand-300",
                      )}
                    >
                      {event.title}
                    </div>
                  ))}
                  {cell.events.length > 3 ? (
                    <p className="px-1 text-[10px] text-muted">+{cell.events.length - 3}</p>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </Card>
      ) : (
        <Card>
          <CardBody className="p-0">
            {agenda.isLoading ? <LoadingBlock /> : (agenda.data ?? []).length === 0 ? (
              <EmptyState title={t("calendar.empty")} description="导入 ICS 或手动添加日程。" />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {agenda.data!.map((event) => (
                  <li key={event.id} className="flex items-start justify-between gap-3 px-3 py-2.5">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{event.title}</p>
                      <p className="truncate text-[11px] text-muted">
                        {formatDateTime(event.startsAt, locale)}
                        {event.endsAt ? ` – ${formatDateTime(event.endsAt, locale)}` : ""}
                        {event.location ? ` · ${event.location}` : ""}
                        {event.courseName ? ` · ${event.courseName}` : ""}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <Badge tone={event.kind === "exam" ? "danger" : event.kind === "assignment_due" ? "warning" : "neutral"}>
                        {KIND_LABEL[event.kind]}
                      </Badge>
                      <Button variant="ghost" size="sm" onClick={() => remove.mutate(event.id)}>
                        ×
                      </Button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>
      )}

      <CreateEventDialog
        open={creating}
        onClose={() => setCreating(false)}
        onCreated={() => {
          setCreating(false);
          void queryClient.invalidateQueries({ queryKey: ["calendar-month"] });
          void queryClient.invalidateQueries({ queryKey: ["calendar-agenda"] });
        }}
      />
    </PageBody>
  );
}

interface GridCell {
  key: string;
  day: number;
  inMonth: boolean;
  isToday: boolean;
  events: CalendarEvent[];
}

function buildMonthGrid(year: number, month: number, events: CalendarEvent[]): GridCell[] {
  const first = new Date(year, month - 1, 1);
  const startOffset = (first.getDay() + 6) % 7; // Monday-first
  const daysInMonth = new Date(year, month, 0).getDate();
  const cells: GridCell[] = [];
  const today = new Date();

  const eventsByDay = new Map<string, CalendarEvent[]>();
  for (const event of events) {
    const date = new Date(event.startsAt);
    const key = `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
    const bucket = eventsByDay.get(key) ?? [];
    bucket.push(event);
    eventsByDay.set(key, bucket);
  }

  const prevMonthDays = new Date(year, month - 1, 0).getDate();
  for (let i = startOffset - 1; i >= 0; i -= 1) {
    const day = prevMonthDays - i;
    cells.push({
      key: `prev-${day}`,
      day,
      inMonth: false,
      isToday: false,
      events: eventsByDay.get(`${month === 1 ? year - 1 : year}-${month === 1 ? 12 : month - 1}-${day}`) ?? [],
    });
  }

  for (let day = 1; day <= daysInMonth; day += 1) {
    cells.push({
      key: `cur-${day}`,
      day,
      inMonth: true,
      isToday:
        today.getFullYear() === year && today.getMonth() + 1 === month && today.getDate() === day,
      events: eventsByDay.get(`${year}-${month}-${day}`) ?? [],
    });
  }

  while (cells.length % 7 !== 0) {
    const day = cells.length - (startOffset + daysInMonth) + 1;
    cells.push({ key: `next-${day}`, day, inMonth: false, isToday: false, events: [] });
  }

  return cells;
}

function CreateEventDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}) {
  const t = useT();
  const [title, setTitle] = useState("");
  const [start, setStart] = useState("");
  const [end, setEnd] = useState("");
  const [location, setLocation] = useState("");
  const [kind, setKind] = useState("personal");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!title.trim() || !start) {
      toast("error", "请填写标题与开始时间");
      return;
    }
    setBusy(true);
    try {
      await api.createCalendarEvent({
        title: title.trim(),
        startsAt: new Date(start).toISOString(),
        endsAt: end ? new Date(end).toISOString() : null,
        location: location.trim() || null,
        description: notes.trim() || null,
        kind,
      });
      toast("success", "日程已创建");
      setTitle("");
      setNotes("");
      setStart("");
      setEnd("");
      onCreated();
    } catch (error) {
      toast("error", "创建失败", errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("calendar.newEvent")}
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
          <Field label={t("calendar.startsAt")}>
            <Input type="datetime-local" value={start} onChange={(event) => setStart(event.target.value)} />
          </Field>
          <Field label={t("calendar.endsAt")}>
            <Input type="datetime-local" value={end} onChange={(event) => setEnd(event.target.value)} />
          </Field>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("calendar.kind")}>
            <Select value={kind} onChange={(event) => setKind(event.target.value)}>
              <option value="personal">{t("calendar.personal")}</option>
              <option value="study">{t("calendar.study")}</option>
              <option value="exam">{t("calendar.exam")}</option>
              <option value="event">日程</option>
            </Select>
          </Field>
          <Field label={t("calendar.location")}>
            <Input value={location} onChange={(event) => setLocation(event.target.value)} />
          </Field>
        </div>
        <Field label="备注">
          <Textarea value={notes} onChange={(event) => setNotes(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
