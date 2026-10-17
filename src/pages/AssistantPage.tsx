import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Copy,
  CornerDownLeft,
  MessageSquarePlus,
  Send,
  Settings2,
  Square,
  Trash2,
} from "lucide-react";

import { Markdown } from "@/components/markdown";
import { LinkButton } from "@/components/LinkButton";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Input,
  LoadingBlock,
  Select,
  Spinner,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { onChatStream } from "@/services/events";
import { cn, errorMessage, formatRelative } from "@/lib/utils";
import { toast } from "@/stores/toasts";
import type { ChatMessage, Citation, ChatScope } from "@/types/api";

interface LiveGeneration {
  requestId: string;
  sessionId: string;
  messageId: string;
  text: string;
  citations: Citation[];
  error: string | null;
  status: "streaming" | "error";
}

export function AssistantPage() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [searchParams] = useSearchParams();
  const initialMaterial = searchParams.get("material");
  const initialCourse = searchParams.get("course");

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [question, setQuestion] = useState("");
  const [scopeKind, setScopeKind] = useState<ChatScope["kind"]>(
    initialMaterial ? "document" : initialCourse ? "course" : "all",
  );
  const [courseId, setCourseId] = useState(initialCourse ?? "");
  const [materialId, setMaterialId] = useState(initialMaterial ?? "");
  const [live, setLive] = useState<LiveGeneration | null>(null);
  // Retrieval outcome for the latest question. Without it the user only sees
  // prose, which is how "no chunks matched" turns into "the AI refuses to cite".
  const [retrieval, setRetrieval] = useState<{ chunks: number; strategy: string } | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  const providers = useQuery({ queryKey: ["providers"], queryFn: () => api.listProviders() });
  const courses = useQuery({ queryKey: ["course-options"], queryFn: () => api.courseOptions() });
  const sessions = useQuery({ queryKey: ["chat-sessions"], queryFn: () => api.listChatSessions(50) });

  const detail = useQuery({
    queryKey: ["chat-session", sessionId],
    queryFn: () => api.getChatSession(sessionId!),
    enabled: Boolean(sessionId),
  });

  const materials = useQuery({
    queryKey: ["materials", "for-scope", courseId],
    queryFn: () => api.listMaterials({ courseId: courseId || null, limit: 200 }),
    enabled: scopeKind === "document",
  });

  const messages = useMemo<ChatMessage[]>(() => detail.data?.messages ?? [], [detail.data]);

  // Stream subscription lives for the whole page so background completions land.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void onChatStream((event) => {
      setLive((current) => {
        if (!current || current.requestId !== event.requestId) {
          return current;
        }
        switch (event.type) {
          case "delta":
            return { ...current, text: current.text + event.text };
          case "citations":
            return { ...current, citations: event.citations };
          case "completed":
            return current;
          case "failed":
            return { ...current, status: "error", error: event.message };
          default:
            return current;
        }
      });

      if (event.type === "completed") {
        setLive(null);
        void queryClient.invalidateQueries({ queryKey: ["chat-session", event.message.sessionId] });
        void queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      }
      if (event.type === "failed") {
        toast("error", t("assistant.failed"), event.message);
        setLive(null);
        void queryClient.invalidateQueries({ queryKey: ["chat-session"] });
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [queryClient, t]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages.length, live?.text]);

  const buildScope = (): ChatScope => {
    if (scopeKind === "document") {
      return materialId
        ? { kind: "document", courseIds: [], materialIds: [materialId] }
        : { kind: "all", courseIds: [], materialIds: [] };
    }
    if (scopeKind === "course") {
      return courseId
        ? { kind: "course", courseIds: [courseId], materialIds: [] }
        : { kind: "all", courseIds: [], materialIds: [] };
    }
    if (scopeKind === "none") {
      return { kind: "none", courseIds: [], materialIds: [] };
    }
    return { kind: "all", courseIds: [], materialIds: [] };
  };

  const ask = useMutation({
    mutationFn: (text: string) =>
      api.askQuestion({
        sessionId,
        question: text,
        scope: buildScope(),
        noRetrieval: scopeKind === "none",
        topK: 6,
      }),
    onSuccess: (accepted) => {
      setSessionId(accepted.sessionId);
      setQuestion("");
      setRetrieval({ chunks: accepted.contextChunks, strategy: accepted.strategy });
      setLive({
        requestId: accepted.requestId,
        sessionId: accepted.sessionId,
        messageId: accepted.assistantMessage.id,
        text: "",
        citations: accepted.citations,
        error: null,
        status: "streaming",
      });
      void queryClient.invalidateQueries({ queryKey: ["chat-session", accepted.sessionId] });
      void queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
    onError: (error) => toast("error", "提问失败", errorMessage(error)),
  });

  const stop = async () => {
    if (!live) return;
    try {
      await api.cancelGeneration(live.requestId);
      toast("info", "已请求停止生成");
    } catch (error) {
      toast("error", "停止失败", errorMessage(error));
    }
  };

  const removeSession = useMutation({
    mutationFn: (id: string) => api.deleteChatSession(id),
    onSuccess: () => {
      setSessionId(null);
      setLive(null);
      void queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      toast("success", "对话已删除");
    },
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  const hasProvider = (providers.data?.providers.length ?? 0) > 0;
  const streaming = live?.status === "streaming";

  // Human-readable scope, used by the "no chunks matched" hint so the user can
  // see *why* a question produced no citations.
  const scopeDescription = (() => {
    if (scopeKind === "none") return t("assistant.scopeNone");
    if (scopeKind === "all") return t("assistant.scopeAll");
    if (scopeKind === "course") {
      const course = (courses.data ?? []).find((item) => item.id === courseId);
      return course ? `课程：${course.name}` : "未选择课程（等同全部资料）";
    }
    const material = (materials.data ?? []).find((item) => item.id === materialId);
    return material ? `课件：${material.title}` : "未选择课件（等同全部资料）";
  })();

  return (
    <div className="flex h-full min-h-0">
      <aside className="hidden w-60 shrink-0 flex-col border-r border-app bg-[color:var(--app-panel)] lg:flex">
        <div className="flex items-center justify-between px-3 py-2.5">
          <span className="text-xs font-semibold uppercase tracking-wide text-muted">{t("assistant.sessions")}</span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setSessionId(null);
              setLive(null);
            }}
            title={t("assistant.newChat")}
          >
            <MessageSquarePlus className="size-4" />
          </Button>
        </div>
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-1.5 pb-3">
          {(sessions.data ?? []).map((session) => (
            <div
              key={session.id}
              className={cn(
                "group flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs",
                sessionId === session.id ? "bg-brand-500/12" : "hover:bg-[color:var(--app-hover)]",
              )}
            >
              <button className="min-w-0 flex-1 text-left" onClick={() => setSessionId(session.id)}>
                <p className="truncate font-medium">{session.title}</p>
                <p className="truncate text-[10px] text-muted">
                  {session.messageCount} 条 · {formatRelative(session.updatedAt, locale)}
                </p>
              </button>
              <button
                className="opacity-0 transition-opacity group-hover:opacity-100"
                onClick={() => removeSession.mutate(session.id)}
                aria-label={t("assistant.deleteSession")}
              >
                <Trash2 className="size-3.5 text-muted hover:text-red-500" />
              </button>
            </div>
          ))}
          {(sessions.data ?? []).length === 0 ? (
            <p className="px-2 py-6 text-center text-xs text-muted">还没有历史对话</p>
          ) : null}
        </div>
        <div className="border-t border-app p-2">
          <LinkButton to="/settings" variant="ghost" size="sm" icon={<Settings2 className="size-3.5" />} className="w-full justify-start">
            {t("settings.ai")}
          </LinkButton>
        </div>
      </aside>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="flex flex-wrap items-center gap-2 border-b border-app bg-[color:var(--app-panel)] px-3 py-2">
          <span className="text-xs text-muted">{t("assistant.scope")}</span>
          <Select
            value={scopeKind}
            onChange={(event) => setScopeKind(event.target.value as ChatScope["kind"])}
            className="h-8 w-32 text-xs"
          >
            <option value="all">{t("assistant.scopeAll")}</option>
            <option value="course">{t("assistant.scopeCourse")}</option>
            <option value="document">{t("assistant.scopeDoc")}</option>
            <option value="none">{t("assistant.scopeNone")}</option>
          </Select>
          {scopeKind === "course" ? (
            <Select value={courseId} onChange={(event) => setCourseId(event.target.value)} className="h-8 w-40 text-xs">
              <option value="">选择课程</option>
              {(courses.data ?? []).map((course) => (
                <option key={course.id} value={course.id}>
                  {course.name}
                </option>
              ))}
            </Select>
          ) : null}
          {scopeKind === "document" ? (
            <Select value={materialId} onChange={(event) => setMaterialId(event.target.value)} className="h-8 w-52 text-xs">
              <option value="">选择课件</option>
              {(materials.data ?? []).map((material) => (
                <option key={material.id} value={material.id}>
                  {material.title}
                </option>
              ))}
            </Select>
          ) : null}
          <div className="ml-auto flex items-center gap-2">
            {providers.data?.providers[0] ? (
              <Badge tone="neutral">{providers.data.providers.find((p) => p.isDefault)?.name ?? providers.data.providers[0].name}</Badge>
            ) : null}
          </div>
        </div>

        {!hasProvider && providers.isSuccess ? (
          <div className="m-4">
            <Card>
              <EmptyState
                title={t("assistant.noProvider")}
                description={t("settings.noProvidersHint")}
                action={
                  <LinkButton to="/settings" variant="primary" size="sm">
                    {t("assistant.configure")}
                  </LinkButton>
                }
              />
            </Card>
          </div>
        ) : (
          <>
            <div className="min-h-0 flex-1 overflow-y-auto px-3 py-4">
              {detail.isLoading ? <LoadingBlock /> : null}

              {messages.length === 0 && !live ? (
                <div className="mx-auto max-w-2xl">
                  <EmptyState
                    icon={<Send className="size-5" />}
                    title={t("assistant.empty")}
                    description={t("assistant.emptyHint")}
                  />
                  <div className="mt-2 grid gap-2 sm:grid-cols-2">
                    {[
                      "为什么可导一定连续？请分步证明",
                      "总结这份课件的核心公式",
                      "用例子解释链式法则",
                      "列出本章最容易出错的三个点",
                    ].map((example) => (
                      <button
                        key={example}
                        onClick={() => setQuestion(example)}
                        className="rounded-lg border border-app px-3 py-2 text-left text-xs text-muted hover:bg-[color:var(--app-hover)]"
                      >
                        {example}
                      </button>
                    ))}
                  </div>
                </div>
              ) : null}

              <div className="mx-auto flex max-w-3xl flex-col gap-4">
                {messages.map((message) => (
                  <MessageBubble key={message.id} message={message} />
                ))}

                {live ? (
                  <div className="flex gap-2">
                    <div className="mt-0.5 grid size-7 shrink-0 place-items-center rounded-full bg-brand-500/15 text-[10px] font-semibold text-brand-700 dark:text-brand-300">
                      AI
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="rounded-xl border border-app bg-[color:var(--app-panel)] px-3 py-2">
                        {live.text ? (
                          <Markdown content={live.text} citations={live.citations} />
                        ) : (
                          <p className="flex items-center gap-2 text-sm text-muted">
                            <Spinner /> {t("assistant.thinking")}
                          </p>
                        )}
                        {streaming ? <span className="ml-0.5 inline-block h-4 w-1 animate-pulse-dot bg-brand-500 align-middle" /> : null}
                      </div>
                      {live.error ? <p className="mt-1 text-xs text-red-600 dark:text-red-400">{live.error}</p> : null}
                      {live.citations.length > 0 ? <CitationList citations={live.citations} /> : null}
                    </div>
                  </div>
                ) : null}

                <div ref={bottomRef} />
              </div>
            </div>

            <div className="border-t border-app bg-[color:var(--app-panel)] px-3 py-2.5 pb-safe">
              <div className="mx-auto max-w-3xl">
                {retrieval ? (
                  <div className="mb-2">
                    {retrieval.chunks === 0 ? (
                      <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-800 dark:text-amber-300">
                        <p className="font-medium">
                          这次没有检索到任何课件片段，所以回答里不会出现引用和页码。
                        </p>
                        <ul className="mt-1 list-disc space-y-0.5 pl-4">
                          <li>确认检索范围包含要问的课件（当前：{scopeDescription}）</li>
                          <li>换用更接近课件原文的关键词；课件是英文时用英文提问命中率更高</li>
                          <li>课件若显示「等待索引」，请先到课件资料页重建索引</li>
                        </ul>
                      </div>
                    ) : (
                      <p className="text-[11px] text-muted">
                        检索到 {retrieval.chunks} 个片段（{retrieval.strategy}）· 点击引用卡片可跳转到对应页
                      </p>
                    )}
                  </div>
                ) : null}
                <div className="flex items-end gap-2">
                  <Input
                    value={question}
                    onChange={(event) => setQuestion(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" && !event.shiftKey) {
                        event.preventDefault();
                        if (!streaming && question.trim()) ask.mutate(question.trim());
                      }
                    }}
                    placeholder={t("assistant.placeholder")}
                    disabled={!hasProvider || streaming}
                    className="h-11"
                  />
                  {streaming ? (
                    <Button variant="danger" onClick={() => void stop()} icon={<Square className="size-3.5" />}>
                      {t("assistant.stop")}
                    </Button>
                  ) : (
                    <Button
                      variant="primary"
                      loading={ask.isPending}
                      disabled={!hasProvider || !question.trim()}
                      onClick={() => ask.mutate(question.trim())}
                      icon={<Send className="size-4" />}
                    >
                      {t("assistant.send")}
                    </Button>
                  )}
                </div>
                <p className="mt-1.5 flex items-center gap-1 text-[10px] text-muted">
                  <CornerDownLeft className="size-3" />
                  Enter 发送 · 回答基于本地课件检索，并标注真实页码来源
                </p>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const isUser = message.role === "user";

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast("error", "复制失败");
    }
  };

  return (
    <div className={cn("flex gap-2", isUser && "flex-row-reverse")}>
      <div
        className={cn(
          "mt-0.5 grid size-7 shrink-0 place-items-center rounded-full text-[10px] font-semibold",
          isUser ? "bg-[color:var(--app-hover)] text-muted" : "bg-brand-500/15 text-brand-700 dark:text-brand-300",
        )}
      >
        {isUser ? "你" : "AI"}
      </div>
      <div className={cn("min-w-0 flex-1", isUser && "flex flex-col items-end")}>
        <div
          className={cn(
            "inline-block max-w-full rounded-xl px-3 py-2 text-left",
            isUser
              ? "bg-brand-600 text-white dark:bg-brand-500 dark:text-slate-950"
              : "border border-app bg-[color:var(--app-panel)]",
          )}
        >
          {isUser ? (
            <p className="whitespace-pre-wrap text-sm">{message.content}</p>
          ) : (
            <Markdown content={message.content} citations={message.citations} />
          )}
        </div>

        {message.status === "error" ? (
          <p className="mt-1 text-xs text-red-600 dark:text-red-400">{message.error}</p>
        ) : null}
        {message.status === "stopped" ? (
          <p className="mt-1 text-[11px] text-muted">{t("assistant.stopped")}</p>
        ) : null}

        {!isUser && message.content ? (
          <div className="mt-1 flex items-center gap-2">
            <button onClick={() => void copy()} className="inline-flex items-center gap-1 text-[11px] text-muted hover:underline">
              <Copy className="size-3" />
              {copied ? t("common.copied") : t("common.copy")}
            </button>
            {message.promptTokens != null ? (
              <span className="text-[10px] text-muted">
                {message.promptTokens} + {message.completionTokens ?? 0} tokens
              </span>
            ) : null}
          </div>
        ) : null}

        {message.citations.length > 0 ? <CitationList citations={message.citations} /> : null}
      </div>
    </div>
  );
}

function CitationList({ citations }: { citations: Citation[] }) {
  const t = useT();
  return (
    <div className="mt-2 rounded-lg border border-app bg-[color:var(--app-panel)] p-2">
      <p className="mb-1 text-[11px] font-medium text-muted">
        {t("assistant.citations")} · {citations.length} 处 · 点击跳转到页码
      </p>
      <ul className="space-y-1">
        {citations.map((citation) => {
          // Every citation now carries a page, so the link always jumps; the
          // label is what distinguishes a real page from an app-laid-out one.
          const page = citation.page ?? citation.slide ?? null;
          const params = new URLSearchParams();
          if (page) params.set("page", String(page));
          const suffix = params.toString() ? `?${params.toString()}` : "";
          const locationLabel =
            citation.pageKind === "slide"
              ? `第 ${page} 张幻灯片`
              : citation.pageKind === "generated"
                ? `第 ${page} 页（应用分页）`
                : page
                  ? `第 ${page} 页`
                  : citation.section ?? "全文";
          return (
            <li key={citation.id || `${citation.chunkId}-${citation.ordinal}`}>
              <Link
                to={`/materials/${citation.documentId}${suffix}`}
                className="flex items-start gap-2 rounded px-1 py-0.5 hover:bg-[color:var(--app-hover)]"
                title={
                  citation.pageKind === "generated"
                    ? "该文档本身没有页码，页号由应用按内容分页生成，点击仍会跳到对应位置"
                    : undefined
                }
              >
                <span className="mt-0.5 shrink-0 rounded bg-brand-500/15 px-1 text-[10px] font-semibold text-brand-700 dark:text-brand-300">
                  S{citation.ordinal}
                </span>
                <span className="min-w-0">
                  <span className="block truncate text-[11px] font-medium">{citation.filename}</span>
                  <span className="block truncate text-[10px] text-muted">
                    {locationLabel}
                    {citation.section && page ? ` · ${citation.section}` : ""}
                  </span>
                  <span className="line-clamp-2 text-[10px] text-muted">{citation.excerpt}</span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
