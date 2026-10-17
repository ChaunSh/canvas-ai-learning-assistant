import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  BookOpen,
  CalendarClock,
  GraduationCap,
  LayoutGrid,
  MessageSquareText,
  Sparkles,
} from "lucide-react";

import { LinkButton } from "@/components/LinkButton";
import { PageBody, PageHeader } from "@/components/layout";
import { Badge, Card, CardBody, CardHeader, EmptyState, ErrorState, LoadingBlock } from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { errorMessage, formatDateTime, formatRelative, percent } from "@/lib/utils";

export function DashboardPage() {
  const t = useT();
  const locale = useLocale();
  const dashboard = useQuery({ queryKey: ["dashboard"], queryFn: () => api.getDashboard() });

  if (dashboard.isLoading) {
    return (
      <PageBody>
        <LoadingBlock />
      </PageBody>
    );
  }

  if (dashboard.isError) {
    return (
      <PageBody>
        <ErrorState
          message="无法加载首页数据"
          detail={errorMessage(dashboard.error)}
          onRetry={() => void dashboard.refetch()}
          retryLabel={t("common.retry")}
        />
      </PageBody>
    );
  }

  const data = dashboard.data!;
  const stats = data.stats;

  return (
    <PageBody>
      <PageHeader
        title={t("home.title")}
        subtitle={t("home.subtitle")}
        actions={
          <>
            <LinkButton to="/assistant" variant="secondary" size="sm" icon={<MessageSquareText className="size-4" />}>
              {t("nav.assistant")}
            </LinkButton>
            <LinkButton to="/materials" variant="primary" size="sm" icon={<Sparkles className="size-4" />}>
              {t("materials.import")}
            </LinkButton>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard icon={<GraduationCap className="size-4" />} label={t("nav.courses")} value={stats.courseCount} to="/courses" />
        <StatCard icon={<BookOpen className="size-4" />} label={t("home.materials")} value={stats.materialCount} to="/materials" />
        <StatCard icon={<MessageSquareText className="size-4" />} label={t("home.chats")} value={stats.chatCount} to="/assistant" />
        <StatCard icon={<LayoutGrid className="size-4" />} label={t("home.openTasks")} value={stats.openTasks} to="/planner" />
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader
            title={t("home.dueSoon")}
            subtitle={`${t("home.pages")}: ${stats.pageCount} · ${t("materials.indexed")}: ${stats.indexedCount}`}
            actions={
              <Badge tone={stats.dueThisWeek > 0 ? "warning" : "neutral"}>
                {stats.dueThisWeek} 项本周截止
              </Badge>
            }
          />
          <CardBody className="p-0">
            {data.upcoming.length === 0 ? (
              <EmptyState
                icon={<CalendarClock className="size-5" />}
                title="没有临近的作业"
                description="同步 Canvas 或手动添加作业后，这里会显示截止时间。"
              />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {data.upcoming.map((assignment) => (
                  <li key={assignment.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{assignment.title}</p>
                      <p className="truncate text-xs text-muted">
                        {assignment.courseName ?? "—"}
                        {assignment.pointsPossible != null ? ` · ${assignment.pointsPossible} 分` : ""}
                      </p>
                    </div>
                    <div className="shrink-0 text-right">
                      <p className="text-xs font-medium">{formatDateTime(assignment.dueAt, locale)}</p>
                      <p className="text-[11px] text-muted">{formatRelative(assignment.dueAt, locale)}</p>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("home.todayTasks")} actions={<Link to="/planner" className="text-xs text-brand-600 dark:text-brand-300">查看</Link>} />
          <CardBody className="p-0">
            {data.todayTasks.length === 0 ? (
              <EmptyState title="今天没有安排" description="生成学习计划后，今天的任务会显示在这里。" />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {data.todayTasks.map((task) => (
                  <li key={task.id} className="px-4 py-2.5">
                    <div className="flex items-center justify-between gap-2">
                      <p className="truncate text-sm">{task.title}</p>
                      <Badge tone={task.status === "done" ? "success" : "neutral"}>
                        {task.estimatedMinutes}{t("common.minutes")}
                      </Badge>
                    </div>
                    <p className="mt-0.5 text-[11px] text-muted">
                      {task.scheduledStart ? formatDateTime(task.scheduledStart, locale) : "未排期"}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader title={t("home.recentMaterials")} actions={<Link to="/materials" className="text-xs text-brand-600 dark:text-brand-300">全部</Link>} />
          <CardBody className="p-0">
            {data.recentMaterials.length === 0 ? (
              <EmptyState
                icon={<BookOpen className="size-5" />}
                title={t("materials.empty")}
                description={t("materials.emptyHint")}
                action={
                  <LinkButton to="/materials" variant="primary" size="sm">
                    {t("materials.import")}
                  </LinkButton>
                }
              />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {data.recentMaterials.map((material) => (
                  <li key={material.id}>
                    <Link to={`/materials/${material.id}`} className="flex items-center justify-between gap-3 px-4 py-2.5 hover:bg-[color:var(--app-hover)]">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium">{material.title}</p>
                        <p className="truncate text-[11px] text-muted">
                          {material.courseName ?? "未归类"} · {material.kind.toUpperCase()}
                        </p>
                      </div>
                      <Badge tone={material.indexStatus === "indexed" ? "success" : material.indexStatus === "failed" ? "danger" : "neutral"}>
                        {material.indexStatus === "indexed" ? t("materials.indexed") : material.indexStatus === "failed" ? t("materials.failed") : t("materials.pending")}
                      </Badge>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("home.announcements")} />
          <CardBody className="p-0">
            {data.announcements.length === 0 ? (
              <EmptyState title="暂无公告" description="连接 Canvas 后可同步课程公告。" />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {data.announcements.map((announcement) => (
                  <li key={announcement.id} className="px-4 py-2.5">
                    <p className="truncate text-sm">{announcement.title}</p>
                    <p className="mt-0.5 truncate text-[11px] text-muted">
                      {announcement.courseName ?? ""} · {formatRelative(announcement.postedAt, locale)}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>
      </div>

      {data.courses.length === 0 ? (
        <Card className="mt-4">
          <CardBody className="flex flex-col items-start gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="text-sm font-medium">{t("home.noCourses")}</p>
              <p className="text-xs text-muted">{t("home.noCoursesHint")}</p>
            </div>
            <div className="flex gap-2">
              <LinkButton to="/courses" variant="secondary" size="sm">
                {t("courses.new")}
              </LinkButton>
              <LinkButton to="/canvas" variant="primary" size="sm">
                {t("canvas.connect")}
              </LinkButton>
            </div>
          </CardBody>
        </Card>
      ) : null}

      <CourseProgress courses={data.courses} />
    </PageBody>
  );
}

function StatCard({
  icon,
  label,
  value,
  to,
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
  to: string;
}) {
  return (
    <Link to={to} className="block">
      <Card className="transition-colors hover:bg-[color:var(--app-hover)]">
        <CardBody className="flex items-center gap-3 p-3.5">
          <div className="grid size-9 shrink-0 place-items-center rounded-lg bg-brand-500/10 text-brand-600 dark:text-brand-300">
            {icon}
          </div>
          <div className="min-w-0">
            <p className="text-lg font-semibold leading-tight">{value}</p>
            <p className="truncate text-[11px] text-muted">{label}</p>
          </div>
        </CardBody>
      </Card>
    </Link>
  );
}

function CourseProgress({ courses }: { courses: { id: string; name: string; progress: number; materialCount: number; openTaskCount: number }[] }) {
  if (courses.length === 0) return null;
  return (
    <Card className="mt-4">
      <CardHeader title="课程进度" subtitle="基于已完成任务与学习记录的粗略估计" />
      <CardBody className="space-y-3">
        {courses.slice(0, 6).map((course) => (
          <div key={course.id}>
            <div className="mb-1 flex items-center justify-between text-xs">
              <Link to={`/courses/${course.id}`} className="truncate font-medium hover:underline">
                {course.name}
              </Link>
              <span className="text-muted">
                {course.materialCount} 份课件 · {course.openTaskCount} 项待办
              </span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-[color:var(--app-hover)]">
              <div className="h-full rounded-full bg-brand-600 dark:bg-brand-400" style={{ width: `${percent(course.progress)}%` }} />
            </div>
          </div>
        ))}
      </CardBody>
    </Card>
  );
}
