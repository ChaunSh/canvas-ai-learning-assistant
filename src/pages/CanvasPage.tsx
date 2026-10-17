import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, ExternalLink, Info, LogIn, RefreshCw, ShieldOff, Wifi } from "lucide-react";

import { LinkButton } from "@/components/LinkButton";
import { PageBody, PageHeader } from "@/components/layout";
import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  EmptyState,
  ErrorState,
  LoadingBlock,
  ProgressBar,
  Spinner,
  Switch,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { onCanvasConnection, onSyncProgress } from "@/services/events";
import { cn, errorMessage, formatDateTime } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { CapabilityStatus, SyncProgressEvent } from "@/types/api";

const STATE_TONE: Record<string, "success" | "warning" | "danger" | "neutral" | "info"> = {
  "not-connected": "neutral",
  "login-required": "warning",
  "logging-in": "info",
  "web-authenticated": "info",
  "data-access-verified": "success",
  "session-expired": "warning",
  unsupported: "danger",
  error: "danger",
};

const CAP_TONE: Record<CapabilityStatus, "success" | "warning" | "danger" | "neutral" | "info"> = {
  supported: "success",
  unsupported: "neutral",
  "permission-denied": "warning",
  "login-required": "info",
  "not-tested": "neutral",
  "temporarily-unavailable": "warning",
};

export function CanvasPage() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<SyncProgressEvent | null>(null);

  const connection = useQuery({
    queryKey: ["canvas-connection"],
    queryFn: () => api.getCanvasConnection(),
    // The backend watcher normally pushes an event, but polling is a cheap
    // safety net while we are waiting for the user to finish logging in.
    refetchInterval: (query) => {
      const state = query.state.data?.state;
      return state === "logging-in" || state === "login-required" || state === "session-expired"
        ? 5000
        : false;
    },
  });
  const history = useQuery({ queryKey: ["sync-history"], queryFn: () => api.syncHistoryDetailed(15) });
  const info = useQuery({ queryKey: ["canvas-info"], queryFn: () => api.canvasIntegrationInfo() });
  const diagnostics = useQuery({ queryKey: ["sync-diagnostics"], queryFn: () => api.getSyncDiagnostics() });
  const autoDownload = useQuery({
    queryKey: ["setting", "canvas.auto_download"],
    queryFn: () => api.settingsGet("canvas.auto_download"),
  });
  const setAutoDownload = useMutation({
    mutationFn: (value: boolean) =>
      api.settingsSet("canvas.auto_download", value ? "true" : "false"),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["setting", "canvas.auto_download"] });
      toast("success", "设置已保存", "只影响之后发起的同步");
    },
    onError: (error) => toast("error", "保存失败", errorMessage(error)),
  });
  const downloadAll = useMutation({
    mutationFn: () => api.downloadCourseFiles(null),
    onSuccess: (report) => {
      void queryClient.invalidateQueries({ queryKey: ["course-files"] });
      toast(
        report.failed > 0 ? "info" : "success",
        "课件下载完成",
        `成功 ${report.downloaded} 个${report.failed > 0 ? `，失败 ${report.failed} 个` : ""}${
          report.errors.length > 0 ? `：${report.errors[0]}` : ""
        }`,
      );
    },
    onError: (error) => toast("error", "下载失败", errorMessage(error)),
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void onSyncProgress((event) => {
      setProgress(event);
      if (event.stage === "courses" && event.processed === event.total && event.total > 0) {
        window.setTimeout(() => setProgress(null), 2000);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  // The login happens in another window; the backend watcher verifies the
  // session and pushes the result here so this page updates by itself.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void onCanvasConnection((connection) => {
      void queryClient.invalidateQueries({ queryKey: ["canvas-connection"] });
      void queryClient.invalidateQueries({ queryKey: ["sync-history"] });
      void queryClient.invalidateQueries({ queryKey: ["sync-diagnostics"] });
      toast(
        connection.state === "data-access-verified" ? "success" : "info",
        connection.state === "data-access-verified"
          ? "已检测到 Canvas 登录，数据可读"
          : "已检测到 Canvas 登录",
        connection.displayName ?? undefined,
      );
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [queryClient]);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: ["canvas-connection"] });
    void queryClient.invalidateQueries({ queryKey: ["sync-history"] });
    void queryClient.invalidateQueries({ queryKey: ["sync-diagnostics"] });
  };

  const openLogin = useMutation({
    mutationFn: () => api.openCanvasLogin(),
    onSuccess: (result) => toast("info", "已打开 Canvas 登录窗口", result.message),
    onError: (error) => toast("error", "无法打开登录窗口", errorMessage(error)),
  });

  const check = useMutation({
    mutationFn: () => api.checkCanvasConnection(),
    onSuccess: (result) => {
      toast(
        result.state === "data-access-verified" ? "success" : "info",
        `连接状态：${t(`canvas.state.${result.state}`)}`,
        result.diagnostics.at(-1)?.message,
      );
      refresh();
    },
    onError: (error) => toast("error", "检查失败", errorMessage(error)),
  });

  const sync = useMutation({
    mutationFn: () => api.syncAll(),
    onSuccess: (run) => {
      toast(
        run.status === "completed" ? "success" : "error",
        run.status === "completed" ? "同步完成" : "同步失败",
        run.error ?? `课程 ${run.stats.courses} · 作业 ${run.stats.assignments} · 公告 ${run.stats.announcements}`,
      );
      setProgress(null);
      refresh();
      void queryClient.invalidateQueries();
    },
    onError: (error) => toast("error", "同步失败", errorMessage(error)),
  });

  const disconnect = useMutation({
    mutationFn: () => api.disconnectCanvas(),
    onSuccess: () => {
      toast("success", "已断开 Canvas 并清除登录数据");
      refresh();
    },
    onError: (error) => toast("error", "断开失败", errorMessage(error)),
  });

  if (connection.isLoading) {
    return (
      <PageBody>
        <LoadingBlock />
      </PageBody>
    );
  }

  if (connection.isError || !connection.data) {
    return (
      <PageBody>
        <ErrorState message="无法读取 Canvas 状态" detail={errorMessage(connection.error)} onRetry={() => void connection.refetch()} />
      </PageBody>
    );
  }

  const state = connection.data;
  const stateLabel = t(`canvas.state.${state.state}`);

  return (
    <PageBody>
      <PageHeader
        title={t("canvas.title")}
        subtitle={state.baseUrl}
        actions={
          <>
            <Button variant="secondary" size="sm" loading={openLogin.isPending} onClick={() => openLogin.mutate()} icon={<LogIn className="size-4" />}>
              {t("canvas.openLogin")}
            </Button>
            <Button variant="secondary" size="sm" loading={check.isPending} onClick={() => check.mutate()} icon={<Wifi className="size-4" />}>
              {t("canvas.check")}
            </Button>
            <Button
              variant="primary"
              size="sm"
              loading={sync.isPending}
              disabled={state.state !== "data-access-verified" && state.state !== "web-authenticated"}
              onClick={() => sync.mutate()}
              icon={<RefreshCw className="size-4" />}
            >
              {t("canvas.syncAll")}
            </Button>
          </>
        }
      />

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader
            title={t("canvas.state")}
            actions={<Badge tone={STATE_TONE[state.state] ?? "neutral"}>{stateLabel}</Badge>}
          />
          <CardBody className="space-y-2 text-xs">
            <Row label="已登录用户" value={state.displayName ?? "—"} />
            <Row label="最近检查" value={state.lastCheckedAt ? formatDateTime(state.lastCheckedAt, locale) : "—"} />
            <Row label="最近验证" value={state.lastVerifiedAt ? formatDateTime(state.lastVerifiedAt, locale) : "—"} />
            <Row label="认证方式" value="学校 Canvas 网页登录（无 PAT / 无 OAuth）" />
            <p className="rounded-lg bg-[color:var(--app-hover)] px-2 py-1.5 text-[11px] text-muted">
              {t("canvas.loginHint")}
            </p>
            {state.state === "logging-in" ? (
              <div className="flex items-center gap-2 rounded-lg bg-sky-500/10 px-2 py-1.5 text-[11px] text-sky-700 dark:text-sky-300">
                <Spinner className="size-3.5" />
                <span>已打开登录窗口。完成登录后会自动检测（约 5 秒内），无需手动点击。</span>
              </div>
            ) : null}
            <div className="flex flex-wrap gap-2 pt-1">
              <Button variant="danger" size="sm" loading={disconnect.isPending} onClick={() => disconnect.mutate()} icon={<ShieldOff className="size-3.5" />}>
                {t("canvas.disconnect")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void api.openCanvasPage(`${state.baseUrl}/courses`).catch((error) => toast("error", "打开失败", errorMessage(error)))}
                icon={<ExternalLink className="size-3.5" />}
              >
                {t("common.open")} Canvas
              </Button>
            </div>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("canvas.capabilities")} subtitle="仅对真实请求成功的数据标记为可用" />
          <CardBody className="p-0">
            <ul className="divide-y divide-[color:var(--app-border)]">
              {state.capabilities.map((capability) => (
                <li key={capability.key} className="flex items-center justify-between gap-2 px-3 py-2">
                  <div className="min-w-0">
                    <p className="truncate text-xs font-medium">{t(`canvas.cap.${capability.key}`)}</p>
                    <p className="truncate text-[10px] text-muted">{capability.detail ?? "—"}</p>
                  </div>
                  <Badge tone={CAP_TONE[capability.status]}>{t(`canvas.cap.${capability.status}`)}</Badge>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>
      </div>

      {progress ? (
        <Card className="mt-4">
          <CardBody className="space-y-1.5">
            <p className="text-xs font-medium">
              {progress.message ?? progress.stage} · {progress.resource}
            </p>
            <ProgressBar value={progress.total > 0 ? (progress.processed / progress.total) * 100 : 10} />
          </CardBody>
        </Card>
      ) : null}

      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader title="课件下载" subtitle="同步只拉取清单，文件按需或自动下载" />
          <CardBody className="space-y-3">
            <Switch
              checked={autoDownload.isLoading ? true : (autoDownload.data ?? "true") !== "false"}
              disabled={setAutoDownload.isPending || autoDownload.isLoading}
              onChange={(value) => setAutoDownload.mutate(value)}
              label="同步后自动下载课件"
              description="同步完成时自动取回尚未下载的课件文件；失败的文件可在「课件文件」里重试。"
            />
            <div className="flex flex-wrap gap-2">
              <Button
                variant="secondary"
                size="sm"
                loading={downloadAll.isPending}
                onClick={() => downloadAll.mutate()}
                icon={<Download className="size-3.5" />}
              >
                下载全部待下载课件
              </Button>
              <LinkButton to="/materials" variant="ghost" size="sm">
                查看课件文件
              </LinkButton>
            </div>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("canvas.history")} />
          <CardBody className="p-0">
            {history.isLoading ? <LoadingBlock /> : (history.data ?? []).length === 0 ? (
              <EmptyState title="还没有同步记录" description="登录成功后点击「同步全部」。" />
            ) : (
              <ul className="divide-y divide-[color:var(--app-border)]">
                {history.data!.map((entry) => (
                  <li key={entry.run.id} className="px-3 py-2">
                    <div className="flex items-center justify-between gap-2">
                      <p className="truncate text-xs font-medium">{entry.summary}</p>
                      <Badge tone={entry.run.status === "completed" ? "success" : entry.run.status === "failed" ? "danger" : "neutral"}>
                        {entry.run.status}
                      </Badge>
                    </div>
                    <p className="mt-0.5 text-[10px] text-muted">
                      {formatDateTime(entry.run.createdAt, locale)}
                      {entry.run.error ? ` · ${entry.run.error}` : ""}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("canvas.integration")} />
          <CardBody className="space-y-2 text-xs">
            {info.data ? (
              <>
                <Row label="地址" value={info.data.baseUrl} />
                <Row label="认证模型" value={info.data.authModel} />
                <Row label="自动下载文件" value={info.data.fileDownloadSupported ? "支持（按课件下载）" : "不支持"} />
                <ul className="mt-2 list-disc space-y-1 pl-4 text-[11px] text-muted">
                  {info.data.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
              </>
            ) : (
              <LoadingBlock />
            )}
          </CardBody>
        </Card>
      </div>

      <Card className="mt-4">
        <CardHeader
          title={t("canvas.diagnostics")}
          subtitle={`${diagnostics.data?.errors.length ?? 0} 条同步错误`}
        />
        <CardBody className="space-y-2">
          {state.diagnostics.length > 0 ? (
            <ul className="space-y-1">
              {state.diagnostics.slice(-8).map((entry) => (
                <li key={`${entry.at}-${entry.message}`} className={cn("rounded px-2 py-1 text-[11px]", entry.level === "warn" ? "bg-amber-500/10 text-amber-700 dark:text-amber-400" : "bg-[color:var(--app-hover)] text-muted")}>
                  {formatDateTime(entry.at, locale)} · {entry.message}
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-xs text-muted">暂无诊断信息。</p>
          )}
          {(diagnostics.data?.errors.length ?? 0) > 0 ? (
            <ul className="space-y-1">
              {diagnostics.data!.errors.slice(0, 8).map((error) => (
                <li key={`${error.runId}-${error.message}`} className="rounded bg-red-500/10 px-2 py-1 text-[11px] text-red-700 dark:text-red-400">
                  {error.resource ?? "全局"} · {error.message}
                  {error.retryable ? "（可重试）" : ""}
                </li>
              ))}
            </ul>
          ) : null}
          <div className="flex items-start gap-2 rounded-lg border border-app p-2 text-[11px] text-muted">
            <Info className="mt-0.5 size-3.5 shrink-0" />
            <span>
              应用不会读取、复制或保存 Cookie / SSO / CSRF 等会话材料；所有请求都在 Canvas 页面内同源执行。
              课件会按需从 Canvas 下载并保存到本地，用于离线阅读与检索；下载在后台进行，可在「课件文件」查看进度。
            </span>
          </div>
        </CardBody>
      </Card>
    </PageBody>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="shrink-0 text-muted">{label}</span>
      <span className="min-w-0 break-all text-right font-medium">{value}</span>
    </div>
  );
}
