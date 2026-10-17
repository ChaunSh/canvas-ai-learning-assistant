import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ArrowRight, BookOpen, GraduationCap, MessageSquareText, ShieldCheck, Sparkles } from "lucide-react";

import { Button, Card } from "@/components/ui";
import { useT } from "@/i18n";
import { useSettings } from "@/stores/settings";

export function WelcomePage() {
  const t = useT();
  const navigate = useNavigate();
  const completeOnboarding = useSettings((s) => s.completeOnboarding);
  const [busy, setBusy] = useState(false);

  const start = (target: string) => {
    setBusy(true);
    completeOnboarding();
    navigate(target);
  };

  const steps = [
    { icon: <GraduationCap className="size-4" />, key: "onboarding.step1" },
    { icon: <BookOpen className="size-4" />, key: "onboarding.step2" },
    { icon: <Sparkles className="size-4" />, key: "onboarding.step3" },
    { icon: <MessageSquareText className="size-4" />, key: "onboarding.step4" },
  ];

  return (
    <div className="flex min-h-full items-center justify-center bg-app px-4 py-10">
      <Card className="w-full max-w-2xl overflow-hidden">
        <div className="bg-gradient-to-br from-brand-700 via-brand-600 to-brand-500 px-6 py-8 text-white dark:from-brand-800 dark:via-brand-700 dark:to-brand-600">
          <div className="mb-3 inline-flex size-11 items-center justify-center rounded-xl bg-white/15 text-lg font-bold">
            CA
          </div>
          <h1 className="text-2xl font-semibold tracking-tight">{t("onboarding.title")}</h1>
          <p className="mt-1.5 max-w-lg text-sm text-white/85">{t("onboarding.subtitle")}</p>
        </div>

        <div className="space-y-3 p-6">
          {steps.map((step, index) => (
            <div key={step.key} className="flex items-start gap-3">
              <div className="mt-0.5 grid size-7 shrink-0 place-items-center rounded-full bg-[color:var(--app-hover)] text-[color:var(--app-muted)]">
                {step.icon}
              </div>
              <div className="min-w-0">
                <p className="text-sm">
                  <span className="mr-1.5 font-mono text-xs text-muted">{index + 1}.</span>
                  {t(step.key)}
                </p>
              </div>
            </div>
          ))}

          <div className="mt-4 flex items-start gap-2 rounded-lg border border-app p-3 text-xs text-muted">
            <ShieldCheck className="mt-0.5 size-4 shrink-0 text-emerald-500" />
            <p>
              Canvas 登录只使用学校官方网页会话，应用不会读取或保存你的密码、Cookie 或 SSO 令牌。
              AI API Key 保存在系统凭据存储中。
            </p>
          </div>

          <div className="flex flex-col gap-2 pt-2 sm:flex-row sm:justify-end">
            <Button variant="secondary" onClick={() => start("/canvas")} disabled={busy}>
              {t("onboarding.connectCanvas")}
            </Button>
            <Button variant="primary" onClick={() => start("/")} disabled={busy} icon={<ArrowRight className="size-4" />}>
              {t("onboarding.start")}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}
