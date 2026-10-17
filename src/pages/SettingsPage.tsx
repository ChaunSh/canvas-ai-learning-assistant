import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Database, Download, HardDrive, KeyRound, Plus, RefreshCw, ShieldCheck, Trash2, Upload } from "lucide-react";

import { PageBody, PageHeader } from "@/components/layout";
import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Dialog,
  EmptyState,
  Field,
  Input,
  LoadingBlock,
  Select,
  Switch,
  Tab,
  Tabs,
} from "@/components/ui";
import { useLocale, useT } from "@/i18n";
import { api } from "@/services/api";
import { cn, errorMessage, formatBytes, formatDateTime } from "@/lib/utils";
import { useSettings, type ThemeMode } from "@/stores/settings";
import { toast } from "@/stores/toasts";
import type { AiProvider } from "@/types/api";

const TABS = ["general", "ai", "data", "privacy", "about"] as const;
type TabKey = (typeof TABS)[number];

export function SettingsPage() {
  const t = useT();
  const [tab, setTab] = useState<TabKey>("general");

  return (
    <PageBody>
      <PageHeader title={t("settings.title")} />
      <Tabs value={tab} onChange={(value) => setTab(value as TabKey)}>
        <Tab value="general">{t("settings.general")}</Tab>
        <Tab value="ai">{t("settings.ai")}</Tab>
        <Tab value="data">{t("settings.data")}</Tab>
        <Tab value="privacy">{t("settings.privacy")}</Tab>
        <Tab value="about">{t("settings.about")}</Tab>
      </Tabs>
      <div className="pt-4">
        {tab === "general" ? <GeneralTab /> : null}
        {tab === "ai" ? <AiTab /> : null}
        {tab === "data" ? <DataTab /> : null}
        {tab === "privacy" ? <PrivacyTab /> : null}
        {tab === "about" ? <AboutTab /> : null}
      </div>
    </PageBody>
  );
}

function GeneralTab() {
  const t = useT();
  const theme = useSettings((s) => s.theme);
  const setTheme = useSettings((s) => s.setTheme);
  const locale = useSettings((s) => s.locale);
  const setLocale = useSettings((s) => s.setLocale);

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader title={t("settings.appearance")} />
        <CardBody className="space-y-3">
          <Field label={t("settings.theme")}>
            <Select value={theme} onChange={(event) => setTheme(event.target.value as ThemeMode)}>
              <option value="system">{t("settings.themeSystem")}</option>
              <option value="light">{t("settings.themeLight")}</option>
              <option value="dark">{t("settings.themeDark")}</option>
            </Select>
          </Field>
          <Field label={t("settings.language")}>
            <Select value={locale} onChange={(event) => setLocale(event.target.value as "zh" | "en")}>
              <option value="zh">简体中文</option>
              <option value="en">English</option>
            </Select>
          </Field>
        </CardBody>
      </Card>

      <Card>
        <CardHeader title={t("settings.canvas")} />
        <CardBody className="space-y-2 text-sm">
          <p className="text-xs text-muted">
            登录、能力诊断与同步都在独立的 Canvas 窗口中完成，与本地数据库隔离。
          </p>
          <Link to="/canvas" className="inline-flex text-sm text-brand-600 hover:underline dark:text-brand-300">
            打开 Canvas 连接页面 →
          </Link>
        </CardBody>
      </Card>
    </div>
  );
}

function AiTab() {
  const t = useT();
  const locale = useLocale();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<AiProvider | "new" | null>(null);

  const providers = useQuery({ queryKey: ["providers"], queryFn: () => api.listProviders() });
  const usage = useQuery({ queryKey: ["ai-usage"], queryFn: () => api.aiUsage() });

  const test = useMutation({
    mutationFn: (id: string) => api.testProvider(id),
    onSuccess: (result) => {
      toast(
        result.ok ? "success" : "error",
        result.ok ? `连接正常 · ${result.latencyMs} ms` : "连接失败",
        result.error ?? `模型：${result.models.slice(0, 3).map((model) => model.id).join(", ") || "未返回列表"}`,
      );
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
    onError: (error) => toast("error", "测试失败", errorMessage(error)),
  });

  const setDefault = useMutation({
    mutationFn: (id: string) => api.setDefaultProvider(id),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["providers"] }),
    onError: (error) => toast("error", "设置失败", errorMessage(error)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => {
      toast("success", "已删除服务配置");
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
    onError: (error) => toast("error", "删除失败", errorMessage(error)),
  });

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader
          title={t("settings.ai")}
          subtitle={providers.data ? `${providers.data.providers.length} 个服务` : undefined}
          actions={
            <Button variant="primary" size="sm" onClick={() => setEditing("new")} icon={<Plus className="size-4" />}>
              {t("settings.addProvider")}
            </Button>
          }
        />
        <CardBody className="p-0">
          {providers.isLoading ? <LoadingBlock /> : null}
          {providers.isSuccess && providers.data.providers.length === 0 ? (
            <EmptyState
              icon={<KeyRound className="size-5" />}
              title={t("settings.noProviders")}
              description={t("settings.noProvidersHint")}
              action={
                <Button variant="primary" size="sm" onClick={() => setEditing("new")}>
                  {t("settings.addProvider")}
                </Button>
              }
            />
          ) : null}
          <ul className="divide-y divide-[color:var(--app-border)]">
            {(providers.data?.providers ?? []).map((provider) => (
              <li key={provider.id} className="px-3 py-2.5">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <p className="truncate text-sm font-medium">{provider.name}</p>
                      {provider.isDefault ? <Badge tone="brand">{t("settings.default")}</Badge> : null}
                      {provider.hasApiKey ? <Badge tone="success">Key 已保存</Badge> : <Badge tone="warning">缺少 Key</Badge>}
                    </div>
                    <p className="truncate text-[11px] text-muted">
                      {provider.baseUrl} · {provider.chatModel}
                      {provider.embeddingModel ? ` · 向量 ${provider.embeddingModel}` : ""}
                    </p>
                    {provider.lastTestedAt ? (
                      <p className="text-[10px] text-muted">
                        最近测试 {formatDateTime(provider.lastTestedAt, locale)} ·{" "}
                        {provider.lastTestOk ? "成功" : provider.lastTestError ?? "失败"}
                      </p>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <Button variant="secondary" size="sm" loading={test.isPending} onClick={() => test.mutate(provider.id)} icon={<RefreshCw className="size-3.5" />}>
                      {t("settings.test")}
                    </Button>
                    {!provider.isDefault ? (
                      <Button variant="ghost" size="sm" onClick={() => setDefault.mutate(provider.id)}>
                        {t("settings.setDefault")}
                      </Button>
                    ) : null}
                    <Button variant="ghost" size="sm" onClick={() => setEditing(provider)}>
                      {t("common.edit")}
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => remove.mutate(provider.id)}>
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </CardBody>
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader title={t("settings.secretBacking")} />
          <CardBody className="space-y-2 text-xs">
            <div className="flex items-center gap-2">
              <ShieldCheck className={cn("size-4", providers.data?.secretBackingSecure ? "text-emerald-500" : "text-amber-500")} />
              <span className="font-medium">{providers.data?.secretBackingLabel ?? "—"}</span>
            </div>
            <p className="text-muted">
              API Key 不写入数据库，而是保存在平台凭据存储中；界面只显示是否已保存，不回显明文。
            </p>
            {providers.data && !providers.data.secretBackingSecure ? (
              <p className="rounded bg-amber-500/10 px-2 py-1 text-amber-700 dark:text-amber-400">{t("settings.secretWarn")}</p>
            ) : null}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title={t("settings.usage")} />
          <CardBody className="space-y-1 text-xs">
            <Row label="请求数" value={String(usage.data?.requests ?? 0)} />
            <Row label="提示 tokens" value={String(usage.data?.promptTokens ?? 0)} />
            <Row label="生成 tokens" value={String(usage.data?.completionTokens ?? 0)} />
            <Row label="失败次数" value={String(usage.data?.failures ?? 0)} />
          </CardBody>
        </Card>
      </div>

      <ProviderDialog
        provider={editing}
        onClose={() => setEditing(null)}
        onSaved={() => {
          setEditing(null);
          void queryClient.invalidateQueries({ queryKey: ["providers"] });
        }}
      />
    </div>
  );
}

function ProviderDialog({
  provider,
  onClose,
  onSaved,
}: {
  provider: AiProvider | "new" | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const t = useT();
  const editing = provider !== "new" && provider !== null ? provider : null;
  const [form, setForm] = useState({
    name: "",
    baseUrl: "https://api.deepseek.com/v1",
    chatModel: "deepseek-chat",
    visionModel: "",
    embeddingModel: "",
    apiKey: "",
    temperature: "0.3",
    maxTokens: "4096",
    timeout: "120",
    isDefault: false,
  });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (provider === "new" || provider === null) {
      setForm((current) => ({ ...current, apiKey: "" }));
      return;
    }
    setForm({
      name: provider.name,
      baseUrl: provider.baseUrl,
      chatModel: provider.chatModel,
      visionModel: provider.visionModel ?? "",
      embeddingModel: provider.embeddingModel ?? "",
      apiKey: "",
      temperature: provider.temperature != null ? String(provider.temperature) : "0.3",
      maxTokens: provider.maxOutputTokens != null ? String(provider.maxOutputTokens) : "",
      timeout: String(provider.timeoutSeconds),
      isDefault: provider.isDefault,
    });
  }, [provider]);

  const submit = async () => {
    setBusy(true);
    try {
      await api.saveProvider({
        id: editing?.id ?? null,
        name: form.name.trim(),
        baseUrl: form.baseUrl.trim(),
        chatModel: form.chatModel.trim(),
        visionModel: form.visionModel.trim() || null,
        embeddingModel: form.embeddingModel.trim() || null,
        apiKey: form.apiKey.trim() || null,
        temperature: form.temperature ? Number(form.temperature) : null,
        maxOutputTokens: form.maxTokens ? Number(form.maxTokens) : null,
        timeoutSeconds: form.timeout ? Number(form.timeout) : 120,
        isDefault: form.isDefault,
      });
      toast("success", "已保存 AI 服务配置");
      onSaved();
    } catch (error) {
      toast("error", "保存失败", errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={provider !== null}
      onClose={onClose}
      title={editing ? t("common.edit") : t("settings.addProvider")}
      width="max-w-xl"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" loading={busy} onClick={() => void submit()}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("settings.providerName")}>
            <Input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="DeepSeek" />
          </Field>
          <Field label={t("settings.baseUrl")} hint="形如 https://api.openai.com/v1">
            <Input value={form.baseUrl} onChange={(event) => setForm({ ...form, baseUrl: event.target.value })} />
          </Field>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("settings.chatModel")}>
            <Input value={form.chatModel} onChange={(event) => setForm({ ...form, chatModel: event.target.value })} />
          </Field>
          <Field label={`${t("settings.visionModel")}（${t("common.optional")}）`}>
            <Input value={form.visionModel} onChange={(event) => setForm({ ...form, visionModel: event.target.value })} />
          </Field>
        </div>
        <Field label={`${t("settings.embeddingModel")}（${t("common.optional")}）`} hint="配置后启用语义检索（混合检索）">
          <Input value={form.embeddingModel} onChange={(event) => setForm({ ...form, embeddingModel: event.target.value })} />
        </Field>
        <Field label={t("settings.apiKey")} hint={editing ? t("settings.apiKeyKeep") : "仅保存在系统凭据存储中"}>
          <Input
            type="password"
            value={form.apiKey}
            onChange={(event) => setForm({ ...form, apiKey: event.target.value })}
            placeholder={editing?.hasApiKey ? "••••••••" : ""}
            autoComplete="off"
          />
        </Field>
        <div className="grid gap-3 sm:grid-cols-3">
          <Field label={t("settings.temperature")}>
            <Input value={form.temperature} onChange={(event) => setForm({ ...form, temperature: event.target.value })} />
          </Field>
          <Field label={t("settings.maxTokens")}>
            <Input value={form.maxTokens} onChange={(event) => setForm({ ...form, maxTokens: event.target.value })} />
          </Field>
          <Field label={t("settings.timeout")}>
            <Input value={form.timeout} onChange={(event) => setForm({ ...form, timeout: event.target.value })} />
          </Field>
        </div>
        <Switch
          checked={form.isDefault}
          onChange={(checked) => setForm({ ...form, isDefault: checked })}
          label={t("settings.setDefault")}
        />
      </div>
    </Dialog>
  );
}

function DataTab() {
  const t = useT();
  const queryClient = useQueryClient();
  const [resetText, setResetText] = useState("");

  const storage = useQuery({ queryKey: ["storage"], queryFn: () => api.storageOverview() });
  const cache = useQuery({ queryKey: ["cache-info"], queryFn: () => api.cacheInfo() });

  const exportData = async () => {
    const target = await saveDialog({ defaultPath: "canvas-ai-export.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!target) return;
    try {
      const bytes = await api.exportUserData(target);
      toast("success", "导出完成", `${formatBytes(bytes)} → ${target}`);
    } catch (error) {
      toast("error", "导出失败", errorMessage(error));
    }
  };

  const importData = async () => {
    const picked = await openDialog({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!picked || Array.isArray(picked)) return;
    try {
      const report = await api.importUserData(picked);
      toast("success", "导入完成", `课程 ${report.coursesAdded} · 计划 ${report.plansAdded} · 任务 ${report.tasksAdded} · 跳过 ${report.skipped}`);
      void queryClient.invalidateQueries();
    } catch (error) {
      toast("error", "导入失败", errorMessage(error));
    }
  };

  const backup = async () => {
    try {
      const result = await api.backupDatabase();
      toast("success", "备份完成", `${formatBytes(result.bytes)} → ${result.path}`);
    } catch (error) {
      toast("error", "备份失败", errorMessage(error));
    }
  };

  const clearCache = async () => {
    try {
      const freed = await api.clearCachedFiles();
      toast("success", "缓存已清理", formatBytes(freed));
      void queryClient.invalidateQueries({ queryKey: ["cache-info"] });
    } catch (error) {
      toast("error", "清理失败", errorMessage(error));
    }
  };

  const reset = async () => {
    if (resetText !== "RESET") {
      toast("error", "请输入 RESET 确认");
      return;
    }
    try {
      await api.resetApplicationData("RESET");
      toast("success", "已重置全部本地数据");
      setResetText("");
      void queryClient.invalidateQueries();
    } catch (error) {
      toast("error", "重置失败", errorMessage(error));
    }
  };

  return (
    <div className="space-y-4">
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader title="存储位置" />
          <CardBody className="space-y-1 text-xs">
            {storage.isLoading ? <LoadingBlock /> : storage.data ? (
              <>
                <Row label="数据目录" value={storage.data.dataDir} />
                <Row label="课件目录" value={storage.data.materialsDir} />
                <Row label="备份目录" value={storage.data.backupsDir} />
                <Row label="密钥存储" value={storage.data.secretsBacking} />
              </>
            ) : null}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="缓存占用" />
          <CardBody className="space-y-1 text-xs">
            {cache.data ? (
              <>
                <Row label="数据库" value={formatBytes(cache.data.databaseBytes)} />
                <Row label="课件文件" value={`${formatBytes(cache.data.materialBytes)}（${cache.data.materialCount} 个）`} />
                <Row label="缓存目录" value={formatBytes(cache.data.cacheDirBytes)} />
              </>
            ) : (
              <LoadingBlock />
            )}
          </CardBody>
        </Card>
      </div>

      <Card>
        <CardHeader title="数据管理" />
        <CardBody className="flex flex-wrap gap-2">
          <Button variant="secondary" size="sm" onClick={() => void exportData()} icon={<Download className="size-4" />}>
            {t("settings.exportData")}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void importData()} icon={<Upload className="size-4" />}>
            {t("settings.importData")}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void backup()} icon={<Database className="size-4" />}>
            {t("settings.backup")}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void clearCache()} icon={<HardDrive className="size-4" />}>
            {t("settings.clearCache")}
          </Button>
        </CardBody>
      </Card>

      <Card className="border-red-500/40">
        <CardHeader title={t("settings.reset")} subtitle={t("settings.resetHint")} />
        <CardBody className="space-y-2">
          <div className="flex flex-wrap items-end gap-2">
            <Field label="输入 RESET" className="w-48">
              <Input value={resetText} onChange={(event) => setResetText(event.target.value)} placeholder="RESET" />
            </Field>
            <Button variant="danger" onClick={() => void reset()} disabled={resetText !== "RESET"} icon={<Trash2 className="size-4" />}>
              {t("settings.reset")}
            </Button>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

function PrivacyTab() {
  const t = useT();
  const privacy = useSettings((s) => s.privacy);
  const setPrivacy = useSettings((s) => s.setPrivacy);
  const providers = useQuery({ queryKey: ["providers"], queryFn: () => api.listProviders() });

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader title={t("settings.privacy")} />
        <CardBody className="divide-y divide-[color:var(--app-border)]">
          <Switch
            checked={privacy.allowImages}
            onChange={(checked) => setPrivacy({ allowImages: checked })}
            label={t("settings.allowImages")}
            description="扫描页、图表需要发送给视觉模型才能理解；关闭后仅使用本地文本层。"
          />
          <Switch
            checked={privacy.allowGrades}
            onChange={(checked) => setPrivacy({ allowGrades: checked })}
            label={t("settings.allowGrades")}
            description="成绩属于敏感数据，默认不会进入 AI 上下文。"
          />
          <Switch
            checked={privacy.keepHistory}
            onChange={(checked) => setPrivacy({ keepHistory: checked })}
            label={t("settings.keepHistory")}
            description="关闭后不再持久化新的聊天记录（已有记录需手动删除）。"
          />
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="上传范围" />
        <CardBody className="space-y-2 text-xs text-muted">
          <p>· 只有被检索到的课件片段会进入提示词，且每个课件的「允许发送到 AI」开关独立生效。</p>
          <p>· 课件正文被显式包裹为「证据」，模型指令无法被课件内容覆盖（Prompt Injection 防护）。</p>
          <p>· API Key 仅由本机 Rust 进程读取，Canvas 页面无法访问。</p>
          <p className="pt-1">
            密钥存储：<span className="font-medium text-[color:var(--app-fg)]">{providers.data?.secretBackingLabel ?? "—"}</span>
          </p>
        </CardBody>
      </Card>
    </div>
  );
}

function AboutTab() {
  const t = useT();
  const info = useQuery({ queryKey: ["app-info"], queryFn: () => api.appInfo() });
  const usage = useQuery({ queryKey: ["ai-usage"], queryFn: () => api.aiUsage() });

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader title={t("settings.about")} />
        <CardBody className="space-y-1 text-xs">
          {info.data ? (
            <>
              <Row label="应用" value={`${info.data.name} ${info.data.version}`} />
              <Row label="Tauri" value={info.data.tauriVersion} />
              <Row label="平台" value={`${info.data.platform} / ${info.data.arch}`} />
              <Row label="数据目录" value={info.data.dataDir} />
              <Row label="数据库版本" value={String(info.data.databaseVersion)} />
              <Row label="数据库大小" value={formatBytes(info.data.databaseSize)} />
            </>
          ) : (
            <LoadingBlock />
          )}
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="设计原则" />
        <CardBody className="space-y-2 text-xs text-muted">
          <p>· 本地优先：课程、课件、对话与计划全部存放在本机 SQLite。</p>
          <p>· Canvas 只使用官方网页登录，不使用 PAT / OAuth，不提取会话机密。</p>
          <p>· 能力状态来自真实请求结果；未验证的能力保持「未测试」。</p>
          <p>· 引用只来自真实检索片段，页码由解析结果决定，不为 DOCX 等无分页格式编造页码。</p>
          <p className="pt-1">AI 请求累计：{usage.data?.requests ?? 0} 次 · 失败 {usage.data?.failures ?? 0} 次</p>
        </CardBody>
      </Card>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="shrink-0 text-muted">{label}</span>
      <span className="min-w-0 break-all text-right">{value}</span>
    </div>
  );
}
