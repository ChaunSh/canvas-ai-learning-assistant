import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import {
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  FileQuestion,
  Maximize2,
  MessageSquareText,
  Minus,
  Plus,
  Sparkles,
} from "lucide-react";

import { Markdown } from "@/components/markdown";
import { PageBody } from "@/components/layout";
import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  EmptyState,
  ErrorState,
  LoadingBlock,
  Select,
  Spinner,
  Tab,
  Tabs,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { cn, errorMessage, formatDateTime } from "@/lib/utils";
import { toast } from "@/stores/toasts";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

const SUMMARY_KINDS = [
  { value: "full", label: "完整总结" },
  { value: "concepts", label: "核心概念" },
  { value: "definitions", label: "定义整理" },
  { value: "formulas", label: "公式整理" },
  { value: "theorems", label: "定理整理" },
  { value: "pitfalls", label: "易错点" },
  { value: "review", label: "复习建议" },
  { value: "self_test", label: "自测问题" },
];

export function MaterialViewerPage() {
  const { materialId = "" } = useParams();
  const [searchParams, setSearchParams] = useSearchParams();
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState("summary");
  const [summaryKind, setSummaryKind] = useState("full");
  const initialPage = Number(searchParams.get("page") ?? "1");
  const [page, setPage] = useState(Number.isFinite(initialPage) && initialPage > 0 ? initialPage : 1);

  const material = useQuery({
    queryKey: ["material", materialId],
    queryFn: () => api.getMaterial(materialId),
    enabled: Boolean(materialId),
  });

  const summaries = useQuery({
    queryKey: ["summaries", materialId],
    queryFn: () => api.listSummaries(materialId),
    enabled: Boolean(materialId),
  });

  const generateSummary = useMutation({
    mutationFn: () =>
      api.generateSummary({
        materialId,
        kind: summaryKind,
        language: locale,
        force: true,
      }),
    onSuccess: () => {
      toast("success", "总结已生成");
      void queryClient.invalidateQueries({ queryKey: ["summaries", materialId] });
    },
    onError: (error) => toast("error", "生成总结失败", errorMessage(error)),
  });

  const goToPage = (next: number) => {
    setPage(next);
    const params = new URLSearchParams(searchParams);
    params.set("page", String(next));
    setSearchParams(params, { replace: true });
  };

  if (material.isLoading) {
    return (
      <PageBody>
        <LoadingBlock />
      </PageBody>
    );
  }

  if (material.isError || !material.data) {
    return (
      <PageBody>
        <ErrorState message="无法加载课件" detail={errorMessage(material.error)} onRetry={() => void material.refetch()} />
      </PageBody>
    );
  }

  const data = material.data;
  const isPdf = data.kind === "pdf";
  const isImage = data.kind === "image";

  const currentSummary = (summaries.data ?? []).find(
    (summary) => summary.kind === summaryKind && summary.scope == null,
  );

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-app bg-[color:var(--app-panel)] px-4 py-2.5">
        <Link to="/materials" className="mb-1 inline-flex items-center gap-1 text-xs text-muted hover:underline">
          <ArrowLeft className="size-3.5" />
          {t("nav.materials")}
        </Link>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="min-w-0">
            <h1 className="truncate text-base font-semibold">{data.title}</h1>
            <p className="truncate text-[11px] text-muted">
              {data.courseName ?? "未归类"} · {data.kind.toUpperCase()} · {data.pageCount || data.slideCount} 页 ·{" "}
              {data.chunkCount} 片段
            </p>
          </div>
          <div className="flex items-center gap-1.5">
            <Badge tone={data.indexStatus === "indexed" ? "success" : data.indexStatus === "failed" ? "danger" : "neutral"}>
              {data.indexStatus}
            </Badge>
            <Link
              to={`/assistant?material=${data.id}`}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-brand-600 px-2.5 text-[13px] font-medium text-white hover:bg-brand-700 dark:bg-brand-500 dark:text-slate-950"
            >
              <MessageSquareText className="size-3.5" />
              {t("nav.assistant")}
            </Link>
          </div>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        <div className="min-h-0 flex-1 overflow-auto bg-[color:var(--app-bg)] p-3">
          {isPdf ? (
            <PdfViewer materialId={data.id} page={page} onPageChange={goToPage} onNumPages={() => undefined} />
          ) : isImage ? (
            <ImageViewer materialId={data.id} />
          ) : (
            <TextViewer materialId={data.id} page={page} onPageChange={goToPage} />
          )}
        </div>

        <aside className="flex w-full shrink-0 flex-col border-t border-app bg-[color:var(--app-panel)] lg:w-[420px] lg:border-l lg:border-t-0">
          <div className="px-3 pt-2">
            <Tabs value={tab} onChange={setTab}>
              <Tab value="summary">{t("materials.summary")}</Tab>
              <Tab value="info">信息</Tab>
            </Tabs>
          </div>

          {tab === "summary" ? (
            <div className="flex min-h-0 flex-1 flex-col">
              <div className="flex items-center gap-2 px-3 py-2">
                <Select value={summaryKind} onChange={(event) => setSummaryKind(event.target.value)} className="h-9 flex-1">
                  {SUMMARY_KINDS.map((kind) => (
                    <option key={kind.value} value={kind.value}>
                      {kind.label}
                    </option>
                  ))}
                </Select>
                <Button
                  variant="primary"
                  size="sm"
                  loading={generateSummary.isPending}
                  onClick={() => generateSummary.mutate()}
                  icon={<Sparkles className="size-3.5" />}
                >
                  {t("materials.summarize")}
                </Button>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-4">
                {currentSummary ? (
                  <Card>
                    <CardBody>
                      {currentSummary.stale ? (
                        <p className="mb-2 rounded bg-amber-500/10 px-2 py-1 text-[11px] text-amber-700 dark:text-amber-400">
                          课件已更新，此总结可能已过期。
                        </p>
                      ) : null}
                      <Markdown content={currentSummary.contentMd} />
                      <p className="mt-3 border-t border-app pt-2 text-[10px] text-muted">
                        {currentSummary.model ?? "模型未知"} · {formatDateTime(currentSummary.updatedAt, locale)}
                      </p>
                    </CardBody>
                  </Card>
                ) : (
                  <EmptyState
                    icon={<Sparkles className="size-5" />}
                    title="还没有总结"
                    description="选择总结类型并生成；结果会缓存，避免重复调用模型。"
                  />
                )}
              </div>
            </div>
          ) : (
            <div className="min-h-0 flex-1 overflow-y-auto p-3">
              <Card>
                <CardHeader title="课件信息" />
                <CardBody className="space-y-1.5 text-xs">
                  <InfoRow label="文件名" value={data.filename} />
                  <InfoRow label="类型" value={data.kind} />
                  <InfoRow label="大小" value={`${(data.byteSize / 1024 / 1024).toFixed(2)} MB`} />
                  <InfoRow label="页 / 幻灯片" value={`${data.pageCount} / ${data.slideCount}`} />
                  <InfoRow
                    label="页码来源"
                    value={
                      data.kind === "pdf"
                        ? "文档自身分页"
                        : data.kind === "pptx"
                          ? "幻灯片序号"
                          : "应用按内容分页（原文无页码）"
                    }
                  />
                  <InfoRow label="字符数" value={String(data.charCount)} />
                  <InfoRow label="索引状态" value={data.indexStatus} />
                  <InfoRow label="来源" value={data.source} />
                  <InfoRow label="SHA-256" value={`${data.sha256.slice(0, 16)}…`} />
                  {data.indexError ? <InfoRow label="索引信息" value={data.indexError} /> : null}
                </CardBody>
              </Card>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="shrink-0 text-muted">{label}</span>
      <span className="min-w-0 break-all text-right">{value}</span>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* PDF                                                                 */
/* ------------------------------------------------------------------ */

function PdfViewer({
  materialId,
  page,
  onPageChange,
  onNumPages,
}: {
  materialId: string;
  page: number;
  onPageChange: (page: number) => void;
  onNumPages: (count: number) => void;
}) {
  const t = useT();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const frameRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  // Serialises pdf.js renders on the single canvas element.
  const renderQueueRef = useRef<Promise<void>>(Promise.resolve());
  const dragRef = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const [document, setDocument] = useState<pdfjs.PDFDocumentProxy | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 1.0 means "fit the available width"; larger values zoom in and let the page
  // overflow the panel, where it can be panned or scrolled.
  const [zoom, setZoom] = useState(1);
  const [rendering, setRendering] = useState(false);
  const [numPages, setNumPages] = useState(0);
  // Width available for the page. Zoom is relative to this, so resizing the
  // window keeps the same visible portion instead of a fixed pixel scale.
  const [frameWidth, setFrameWidth] = useState(0);

  useEffect(() => {
    if (!document) return;
    const element = frameRef.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? 0;
      setFrameWidth(Math.floor(width));
    });
    observer.observe(element);
    setFrameWidth(Math.floor(element.getBoundingClientRect().width));
    return () => observer.disconnect();
  }, [document]);

  useEffect(() => {
    let cancelled = false;
    setDocument(null);
    setError(null);
    (async () => {
      try {
        const buffer = await api.materialBytes(materialId);
        const loaded = await pdfjs.getDocument({ data: new Uint8Array(buffer) }).promise;
        if (cancelled) {
          void loaded.cleanup();
          return;
        }
        setDocument(loaded);
        setNumPages(loaded.numPages);
        onNumPages(loaded.numPages);
      } catch (cause) {
        if (!cancelled) setError(errorMessage(cause));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [materialId, onNumPages]);

  useEffect(() => {
    if (!document || !canvasRef.current) return;
    // Wait for the first measurement; rendering at the unscaled width first
    // would flash a page that is wider than the panel.
    if (frameWidth <= 0) return;

    let cancelled = false;
    let task: pdfjs.RenderTask | null = null;
    setRendering(true);

    const render = async () => {
      if (cancelled) return;
      try {
        const safePage = Math.min(Math.max(1, page), document.numPages);
        const pdfPage = await document.getPage(safePage);
        // Re-check after every await: this run may have been superseded while
        // the page was being fetched.
        if (cancelled) return;

        const base = pdfPage.getViewport({ scale: 1 });
        const canvas = canvasRef.current;
        if (!canvas || cancelled) return;

        // Zoom is relative to the panel width, so 100% is "fit width" and
        // anything above that overflows and becomes scrollable/pannable.
        // The old code clamped the width back to the panel, which made zooming
        // past the window size do nothing at all.
        const cssWidth = Math.max(1, Math.round(frameWidth * zoom));
        const cssHeight = Math.max(1, Math.round(cssWidth * (base.height / base.width)));

        // Render at the display size times DPR, but cap the total pixel count so
        // a 400% zoom on an A4 page cannot allocate a canvas the GPU rejects.
        const dpr = window.devicePixelRatio || 1;
        const maxPixels = 16_000_000;
        const wanted = cssWidth * dpr * cssHeight * dpr;
        const density = wanted > maxPixels ? dpr * Math.sqrt(maxPixels / wanted) : dpr;

        const viewport = pdfPage.getViewport({ scale: (cssWidth * density) / base.width });
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        canvas.style.width = `${cssWidth}px`;
        canvas.style.height = `${cssHeight}px`;

        const context = canvas.getContext("2d");
        if (!context) return;
        context.setTransform(1, 0, 0, 1, 0, 0);
        task = pdfPage.render({ canvasContext: context, viewport, canvas });
        await task.promise;
      } catch (cause) {
        // A cancelled render is expected whenever the page/zoom/size changes.
        const cancelledRender =
          cause instanceof Error && cause.name === "RenderingCancelledException";
        if (!cancelled && !cancelledRender) setError(errorMessage(cause));
      } finally {
        if (!cancelled) setRendering(false);
      }
    };

    // pdf.js refuses two concurrent renders on one canvas. Runs are chained so
    // a resize or page change can never overlap with the previous render — the
    // cause of the "Cannot use the same canvas during multiple render()
    // operations" error when the window was resized while a page was drawing.
    renderQueueRef.current = renderQueueRef.current.then(render, render);

    return () => {
      cancelled = true;
      // Cancelling lets the queued run start immediately instead of waiting for
      // a page that is no longer wanted.
      task?.cancel();
    };
  }, [document, page, zoom, frameWidth]);

  if (error) {
    return <ErrorState message="无法渲染 PDF" detail={error} />;
  }

  return (
    <div className="flex flex-col items-center gap-3">
      <div className="sticky top-0 z-10 flex w-full max-w-3xl items-center justify-between gap-2 rounded-lg panel px-2 py-1.5">
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={() => onPageChange(Math.max(1, page - 1))} disabled={page <= 1} title={t("reader.prev")}>
            <ChevronLeft className="size-4" />
          </Button>
          <span className="min-w-16 text-center text-xs">
            {page} / {numPages || "?"}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onPageChange(Math.min(numPages || page, page + 1))}
            disabled={numPages > 0 && page >= numPages}
            title={t("reader.next")}
          >
            <ChevronRight className="size-4" />
          </Button>
          {/* Lives in the left cluster on purpose: the right cluster is pinned to
              the toolbar's right edge, so putting it there pushed the −/%/+
              controls sideways whenever zoom left 100%. */}
          {zoom !== 1 ? (
            <Button variant="ghost" size="sm" onClick={() => setZoom(1)} title="适应宽度">
              <Maximize2 className="size-3.5" />
              <span className="ml-1 text-[11px]">适应宽度</span>
            </Button>
          ) : null}
        </div>
        <div className="flex items-center gap-1">
          {rendering ? <Spinner className="mr-1" /> : null}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setZoom((value) => Math.max(0.5, Math.round((value - 0.15) * 100) / 100))}
            title={t("reader.zoomOut")}
          >
            <Minus className="size-3.5" />
          </Button>
          <span className="w-10 text-center text-[11px] text-muted">{Math.round(zoom * 100)}%</span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setZoom((value) => Math.min(4, Math.round((value + 0.15) * 100) / 100))}
            title={t("reader.zoomIn")}
          >
            <Plus className="size-3.5" />
          </Button>
        </div>
      </div>

      {!document ? (
        <div className="flex h-64 items-center justify-center">
          <Spinner className="size-5" />
        </div>
      ) : (
        <div ref={frameRef} className="w-full">
          {/* The page overflows this box when zoomed past fit, so it scrolls
              and can also be dragged with the mouse. */}
          <div
            ref={scrollRef}
            className="w-full max-h-[76vh] select-none overflow-auto rounded-lg p-1"
            onPointerDown={(event) => {
              const element = scrollRef.current;
              if (!element || event.button !== 0) return;
              // Capture so a fast drag keeps panning even when the pointer
              // leaves the panel.
              element.setPointerCapture(event.pointerId);
              dragRef.current = {
                x: event.clientX,
                y: event.clientY,
                left: element.scrollLeft,
                top: element.scrollTop,
              };
            }}
            onPointerMove={(event) => {
              const drag = dragRef.current;
              const element = scrollRef.current;
              if (!drag || !element) return;
              element.scrollLeft = drag.left - (event.clientX - drag.x);
              element.scrollTop = drag.top - (event.clientY - drag.y);
            }}
            onPointerUp={(event) => {
              dragRef.current = null;
              scrollRef.current?.releasePointerCapture(event.pointerId);
            }}
            onPointerCancel={(event) => {
              dragRef.current = null;
              scrollRef.current?.releasePointerCapture(event.pointerId);
            }}
          >
            <canvas
              ref={canvasRef}
              className="mx-auto block touch-none select-none rounded-lg bg-white shadow-sm"
              style={{ cursor: zoom > 1 ? "grab" : "default" }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Image                                                               */
/* ------------------------------------------------------------------ */

function ImageViewer({ materialId }: { materialId: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let objectUrl: string | null = null;
    (async () => {
      try {
        const buffer = await api.materialBytes(materialId);
        objectUrl = URL.createObjectURL(new Blob([buffer]));
        setUrl(objectUrl);
      } catch (cause) {
        setError(errorMessage(cause));
      }
    })();
    return () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [materialId]);

  if (error) return <ErrorState message="无法加载图片" detail={error} />;
  if (!url) return <LoadingBlock />;
  return <img src={url} alt="" className="mx-auto max-w-full rounded-lg" />;
}

/* ------------------------------------------------------------------ */
/* Text-like documents                                                 */
/* ------------------------------------------------------------------ */

function TextViewer({
  materialId,
  page,
  onPageChange,
}: {
  materialId: string;
  page: number;
  onPageChange: (page: number) => void;
}) {
  const t = useT();
  const pages = useQuery({
    queryKey: ["material-pages", materialId],
    queryFn: () => api.materialPages(materialId),
  });

  const list = useMemo(() => pages.data ?? [], [pages.data]);
  const index = Math.min(Math.max(0, page - 1), Math.max(0, list.length - 1));
  const current = list[index];

  if (pages.isLoading) return <LoadingBlock />;
  if (pages.isError) return <ErrorState message="无法加载文本" detail={errorMessage(pages.error)} />;
  if (list.length === 0) {
    return (
      <EmptyState
        icon={<FileQuestion className="size-5" />}
        title="没有可显示的文本"
        description={t("reader.notPdf")}
      />
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-3">
      <div className="flex items-center justify-between rounded-lg panel px-2 py-1.5">
        <Button variant="ghost" size="sm" onClick={() => onPageChange(Math.max(1, page - 1))} disabled={page <= 1}>
          <ChevronLeft className="size-4" />
        </Button>
        <span className="text-xs">
          {page} / {list.length}
        </span>
        <Button variant="ghost" size="sm" onClick={() => onPageChange(Math.min(list.length, page + 1))} disabled={page >= list.length}>
          <ChevronRight className="size-4" />
        </Button>
      </div>
      <Card>
        <CardBody>
          <div className={cn("whitespace-pre-wrap text-sm leading-relaxed", !current?.text && "text-muted")}>
            {current?.text || t("reader.noText")}
          </div>
        </CardBody>
      </Card>
    </div>
  );
}
