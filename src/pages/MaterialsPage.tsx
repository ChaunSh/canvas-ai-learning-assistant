/**
 * 文件：Canvas 课件的唯一入口。
 *
 * 这里不再有「上传」：课件只能从学校 Canvas 下载，因此页面只有两种状态——
 * 已下载（可打开、可删除）和待下载（点一下即取）。所有内容按课程分组。
 */
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, FileText, Folder, RefreshCw, Search, Trash2 } from "lucide-react";

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
  Input,
  LoadingBlock,
} from "@/components/ui";
import { api } from "@/services/api";
import { onCanvasDownload, onIndexProgress } from "@/services/events";
import { errorMessage, formatBytes } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { CanvasFile, CourseFileGroup, Material } from "@/types/api";

export function MaterialsPage() {
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");

  const groups = useQuery({ queryKey: ["course-files"], queryFn: () => api.listCourseFiles() });

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: ["course-files"] });
    void queryClient.invalidateQueries({ queryKey: ["materials"] });
    void queryClient.invalidateQueries({ queryKey: ["library-stats"] });
  };

  useEffect(() => {
    let unlistenDownload: (() => void) | undefined;
    let unlistenIndex: (() => void) | undefined;
    void onCanvasDownload((event) => {
      refresh();
      if (event.state === "batch-done" && event.downloaded !== null) {
        const failed = event.failed ?? 0;
        toast(
          failed > 0 ? "info" : "success",
          failed > 0 ? "部分课件下载失败" : "课件下载完成",
          `成功 ${event.downloaded} 个${failed > 0 ? `，失败 ${failed} 个` : ""}`,
        );
      }
    }).then((fn) => {
      unlistenDownload = fn;
    });
    void onIndexProgress((progress) => {
      if (progress.stage === "done" || progress.stage === "failed") refresh();
    }).then((fn) => {
      unlistenIndex = fn;
    });
    return () => {
      unlistenDownload?.();
      unlistenIndex?.();
    };
  }, []);

  const downloadOne = useMutation({
    mutationFn: (fileId: string) => api.downloadCanvasFile(fileId),
    onSuccess: (file) => {
      refresh();
      toast("success", "课件已下载", file.displayName);
    },
    onError: (error) => toast("error", "下载失败", errorMessage(error)),
  });

  const downloadCourse = useMutation({
    mutationFn: (courseId: string | null) => api.downloadCourseFiles(courseId),
    onSuccess: (report) => {
      refresh();
      toast(
        report.failed > 0 ? "info" : "success",
        "批量下载完成",
        `成功 ${report.downloaded} 个${report.failed > 0 ? `，失败 ${report.failed} 个` : ""}${
          report.errors.length > 0 ? `：${report.errors[0]}` : ""
        }`,
      );
    },
    onError: (error) => toast("error", "批量下载失败", errorMessage(error)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteMaterial(id),
    onSuccess: () => {
      refresh();
      toast("success", "已删除本地课件", "");
    },
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  const data = groups.data ?? [];
  const pendingTotal = data.reduce((sum, group) => sum + group.pending.length, 0);
  const downloadedTotal = data.reduce((sum, group) => sum + group.downloaded.length, 0);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return data;
    return data
      .map((group) => ({
        ...group,
        downloaded: group.downloaded.filter(
          (m) =>
            m.title.toLowerCase().includes(needle) ||
            m.filename.toLowerCase().includes(needle),
        ),
        pending: group.pending.filter((f) => f.displayName.toLowerCase().includes(needle)),
      }))
      .filter((group) => group.downloaded.length > 0 || group.pending.length > 0);
  }, [data, query]);

  return (
    <>
      <PageHeader
        title="课件文件"
        subtitle="课件全部来自学校 Canvas，按课程归类；未下载的课件可在此一键取回。"
        actions={
          <div className="flex items-center gap-2">
            <LinkButton to="/canvas" variant="secondary" size="sm">
              Canvas 同步
            </LinkButton>
            <Button
              variant="primary"
              size="sm"
              loading={downloadCourse.isPending}
              disabled={pendingTotal === 0}
              onClick={() => downloadCourse.mutate(null)}
              icon={<Download className="size-3.5" />}
            >
              下载全部（{pendingTotal}）
            </Button>
          </div>
        }
      />

      <PageBody className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-55 flex-1">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索已下载或待下载的课件"
              className="pl-8"
            />
          </div>
          <Badge tone="neutral">已下载 {downloadedTotal}</Badge>
          <Badge tone={pendingTotal > 0 ? "warning" : "neutral"}>待下载 {pendingTotal}</Badge>
          <Button
            variant="ghost"
            size="sm"
            onClick={refresh}
            icon={<RefreshCw className="size-3.5" />}
            title="刷新"
          />
        </div>

        {groups.isLoading ? <LoadingBlock label="正在读取课件列表…" /> : null}
        {groups.error ? (
          <ErrorState message="无法读取课件列表" detail={errorMessage(groups.error)} />
        ) : null}

        {!groups.isLoading && !groups.error && filtered.length === 0 ? (
          <EmptyState
            title={data.length === 0 ? "还没有任何课程课件" : "没有匹配的课件"}
            description={
              data.length === 0
                ? "先在「Canvas 同步」里连接学校 Canvas 并同步课程，课件会自动列在这里。"
                : "换个关键词试试，或清空搜索框。"
            }
            action={
              <LinkButton to="/canvas" variant="primary" size="sm">
                去同步 Canvas
              </LinkButton>
            }
          />
        ) : null}

        {filtered.map((group) => {
          const busyCourse =
            downloadCourse.isPending && downloadCourse.variables === group.courseId;
          return (
            <Card key={group.courseId}>
              <CardHeader
                title={group.courseName}
                subtitle={`已下载 ${group.downloaded.length} · 待下载 ${group.pending.length}`}
                actions={
                  group.pending.length > 0 ? (
                    <Button
                      variant="secondary"
                      size="sm"
                      loading={busyCourse}
                      onClick={() => downloadCourse.mutate(group.courseId)}
                      icon={<Download className="size-3.5" />}
                    >
                      下载本课程
                    </Button>
                  ) : null
                }
              />
              <CardBody className="divide-y divide-[color:var(--app-border)] p-0">
                {folderBuckets(group).map((bucket) => (
                  <div key={bucket.key}>
                    <div className="flex items-center gap-2 bg-[color:var(--app-hover)] px-3 py-1.5">
                      <Folder className="size-3.5 shrink-0 text-muted" />
                      <span
                        className="truncate text-[11px] font-medium"
                        style={{ paddingLeft: bucket.depth * 10 }}
                      >
                        {bucket.label}
                      </span>
                      <span className="ml-auto shrink-0 text-[10px] text-muted">
                        {bucket.downloaded.length + bucket.pending.length} 个文件
                      </span>
                    </div>
                    {bucket.pending.map((file) => (
                      <PendingRow
                        key={file.id}
                        file={file}
                        busy={downloadOne.isPending && downloadOne.variables === file.id}
                        disabled={downloadCourse.isPending}
                        onDownload={() => downloadOne.mutate(file.id)}
                      />
                    ))}
                    {bucket.downloaded.map((material) => (
                      <MaterialRow
                        key={material.id}
                        material={material}
                        onDelete={() => remove.mutate(material.id)}
                        deleting={remove.isPending && remove.variables === material.id}
                      />
                    ))}
                  </div>
                ))}
              </CardBody>
            </Card>
          );
        })}
      </PageBody>
    </>
  );
}

type FolderBucket = {
  key: string;
  label: string;
  depth: number;
  pending: CanvasFile[];
  downloaded: Material[];
};

/**
 * Group a course's files the way Canvas shows them: one section per folder,
 * nested by path depth, with the course root first.
 */
function folderBuckets(group: CourseFileGroup): FolderBucket[] {
  const buckets = new Map<string, FolderBucket>();
  const ensure = (path: string | null) => {
    const key = path ?? "";
    if (!buckets.has(key)) {
      const depth = key ? key.split("/").length - 1 : 0;
      buckets.set(key, {
        key,
        label: key || "根目录",
        depth,
        pending: [],
        downloaded: [],
      });
    }
    return buckets.get(key)!;
  };
  for (const file of group.pending) ensure(file.folderPath ?? null).pending.push(file);
  for (const material of group.downloaded) {
    ensure(group.folderByMaterial?.[material.id] ?? null).downloaded.push(material);
  }
  return [...buckets.values()].sort((a, b) => a.key.localeCompare(b.key, "zh-CN"));
}

function PendingRow({
  file,
  busy,
  disabled,
  onDownload,
}: {
  file: CanvasFile;
  busy: boolean;
  disabled: boolean;
  onDownload: () => void;
}) {
  const failed = file.state === "failed";
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[color:var(--app-hover)] text-muted">
        <Download className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{file.displayName}</p>
        <p className="truncate text-[11px] text-muted">
          {file.byteSize ? formatBytes(file.byteSize) : "大小未知"}
          {file.contentType ? ` · ${file.contentType}` : ""}
          {failed && file.error ? ` · ${file.error}` : ""}
        </p>
      </div>
      <Badge tone={failed ? "danger" : "warning"}>{failed ? "下载失败" : "待下载"}</Badge>
      <Button
        variant="secondary"
        size="sm"
        loading={busy}
        disabled={disabled}
        onClick={onDownload}
        icon={<Download className="size-3.5" />}
      >
        下载
      </Button>
    </div>
  );
}

function MaterialRow({
  material,
  onDelete,
  deleting,
}: {
  material: Material;
  onDelete: () => void;
  deleting: boolean;
}) {
  const pages =
    material.pageCount > 0
      ? `${material.pageCount} 页`
      : material.slideCount > 0
        ? `${material.slideCount} 张幻灯片`
        : null;
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-500/12 text-brand-700 dark:text-brand-300">
        <FileText className="size-4" />
      </span>
      <Link to={`/materials/${material.id}`} className="min-w-0 flex-1 hover:underline">
        <p className="truncate text-sm font-medium">{material.title}</p>
        <p className="truncate text-[11px] text-muted">
          {material.kind.toUpperCase()}
          {pages ? ` · ${pages}` : ""}
          {material.byteSize ? ` · ${formatBytes(material.byteSize)}` : ""}
          {material.indexStatus === "indexed" ? " · 已索引" : ` · ${material.indexStatus}`}
        </p>
      </Link>
      <Button
        variant="ghost"
        size="sm"
        loading={deleting}
        onClick={onDelete}
        title="删除本地副本"
        icon={<Trash2 className="size-3.5" />}
      />
    </div>
  );
}
