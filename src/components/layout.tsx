import { useEffect, useState, type ReactNode } from "react";
import { NavLink, useLocation } from "react-router-dom";
import {
  BookOpen,
  CalendarDays,
  GraduationCap,
  Home,
  LayoutGrid,
  MessageSquareText,
  Moon,
  MoreHorizontal,
  Settings,
  Sun,
  WifiOff,
  X,
} from "lucide-react";

import { Badge, Button } from "@/components/ui";
import { useT } from "@/i18n";
import { cn } from "@/lib/utils";
import { useSettings } from "@/stores/settings";

interface NavItem {
  to: string;
  labelKey: string;
  icon: ReactNode;
  primary: boolean;
}

const NAV: NavItem[] = [
  { to: "/", labelKey: "nav.home", icon: <Home className="size-[18px]" />, primary: true },
  { to: "/courses", labelKey: "nav.courses", icon: <GraduationCap className="size-[18px]" />, primary: true },
  { to: "/materials", labelKey: "nav.materials", icon: <BookOpen className="size-[18px]" />, primary: true },
  { to: "/assistant", labelKey: "nav.assistant", icon: <MessageSquareText className="size-[18px]" />, primary: true },
  { to: "/planner", labelKey: "nav.planner", icon: <LayoutGrid className="size-[18px]" />, primary: false },
  { to: "/calendar", labelKey: "nav.calendar", icon: <CalendarDays className="size-[18px]" />, primary: false },
  { to: "/settings", labelKey: "nav.settings", icon: <Settings className="size-[18px]" />, primary: true },
];

/** Desktop breakpoint: matches the Tailwind `md` scale used for the shell. */
export function useIsDesktop(): boolean {
  const [desktop, setDesktop] = useState(() =>
    typeof window === "undefined" ? true : window.innerWidth >= 900,
  );

  useEffect(() => {
    const query = window.matchMedia("(min-width: 900px)");
    const listener = () => setDesktop(query.matches);
    listener();
    query.addEventListener("change", listener);
    return () => query.removeEventListener("change", listener);
  }, []);

  return desktop;
}

export function useOnline(): boolean {
  const [online, setOnline] = useState(() =>
    typeof navigator === "undefined" ? true : navigator.onLine,
  );
  useEffect(() => {
    const up = () => setOnline(true);
    const down = () => setOnline(false);
    window.addEventListener("online", up);
    window.addEventListener("offline", down);
    return () => {
      window.removeEventListener("online", up);
      window.removeEventListener("offline", down);
    };
  }, []);
  return online;
}

function ThemeToggle() {
  const theme = useSettings((s) => s.theme);
  const setTheme = useSettings((s) => s.setTheme);
  const next = theme === "dark" ? "light" : "dark";
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => setTheme(next)}
      aria-label="toggle theme"
      title={theme === "dark" ? "浅色模式" : "深色模式"}
    >
      {theme === "dark" ? <Sun className="size-4" /> : <Moon className="size-4" />}
    </Button>
  );
}

function Sidebar() {
  const t = useT();
  return (
    <aside className="hidden w-60 shrink-0 flex-col border-r border-app bg-[color:var(--app-panel)] md:flex">
      <div className="flex items-center gap-2 px-4 py-4">
        <div className="grid size-8 place-items-center rounded-lg bg-brand-600 text-sm font-bold text-white dark:bg-brand-500 dark:text-slate-950">
          CA
        </div>
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold leading-tight">{t("app.name")}</p>
          <p className="truncate text-[11px] text-muted">{t("app.tagline")}</p>
        </div>
      </div>

      <nav className="flex-1 space-y-0.5 px-2 pb-4">
        {NAV.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === "/"}
            className={({ isActive }) =>
              cn(
                "flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition-colors",
                isActive
                  ? "bg-brand-500/12 font-medium text-brand-700 dark:text-brand-300"
                  : "text-[color:var(--app-muted)] hover:bg-[color:var(--app-hover)] hover:text-[color:var(--app-fg)]",
              )
            }
          >
            {item.icon}
            <span className="truncate">{t(item.labelKey)}</span>
          </NavLink>
        ))}
      </nav>

      <div className="border-t border-app px-3 py-2.5">
        <NavLink
          to="/canvas"
          className="flex items-center justify-between rounded-lg px-2 py-1.5 text-xs text-muted hover:bg-[color:var(--app-hover)]"
        >
          <span>Canvas</span>
          <Badge tone="neutral">设置</Badge>
        </NavLink>
      </div>
    </aside>
  );
}

function MobileTopBar({ title }: { title: string }) {
  return (
    <header className="sticky top-0 z-30 flex items-center justify-between gap-2 border-b border-app bg-[color:var(--app-panel)]/95 px-3 py-2 pt-safe backdrop-blur">
      <div className="flex min-w-0 items-center gap-2">
        <div className="grid size-7 place-items-center rounded-md bg-brand-600 text-[11px] font-bold text-white dark:bg-brand-500 dark:text-slate-950">
          CA
        </div>
        <span className="truncate text-sm font-semibold">{title}</span>
      </div>
      <div className="flex items-center gap-1">
        <ThemeToggle />
      </div>
    </header>
  );
}

function BottomNav() {
  const t = useT();
  const [moreOpen, setMoreOpen] = useState(false);
  const primary = NAV.filter((item) => item.primary);
  const secondary = NAV.filter((item) => !item.primary);

  return (
    <>
      {moreOpen ? (
        <div className="fixed inset-0 z-40 md:hidden" role="presentation">
          <div className="absolute inset-0 bg-black/40" onClick={() => setMoreOpen(false)} aria-hidden />
          <div className="absolute inset-x-0 bottom-0 rounded-t-2xl panel p-3 pb-safe animate-fade-in">
            <div className="mb-2 flex items-center justify-between px-1">
              <span className="text-sm font-semibold">更多</span>
              <Button variant="ghost" size="sm" onClick={() => setMoreOpen(false)} aria-label="close">
                <X className="size-4" />
              </Button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              {[...secondary, { to: "/canvas", labelKey: "canvas.title", icon: <Settings className="size-[18px]" />, primary: false }].map(
                (item) => (
                  <NavLink
                    key={item.to}
                    to={item.to}
                    onClick={() => setMoreOpen(false)}
                    className="flex items-center gap-2 rounded-lg border border-app px-3 py-2.5 text-sm"
                  >
                    {item.icon}
                    <span className="truncate">{t(item.labelKey)}</span>
                  </NavLink>
                ),
              )}
            </div>
          </div>
        </div>
      ) : null}

      <nav className="sticky bottom-0 z-30 flex items-stretch border-t border-app bg-[color:var(--app-panel)]/95 pb-safe backdrop-blur md:hidden">
        {primary.slice(0, 4).map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === "/"}
            className={({ isActive }) =>
              cn(
                "flex flex-1 flex-col items-center gap-0.5 py-2 text-[10px]",
                isActive ? "text-brand-600 dark:text-brand-300" : "text-muted",
              )
            }
          >
            {item.icon}
            <span>{t(item.labelKey)}</span>
          </NavLink>
        ))}
        <button
          type="button"
          onClick={() => setMoreOpen(true)}
          className="flex flex-1 flex-col items-center gap-0.5 py-2 text-[10px] text-muted"
        >
          <MoreHorizontal className="size-[18px]" />
          <span>更多</span>
        </button>
      </nav>
    </>
  );
}

export function AppShell({ title, children }: { title: string; children: ReactNode }) {
  const desktop = useIsDesktop();
  const online = useOnline();
  const t = useT();
  const location = useLocation();

  // Reset scroll on navigation (important on Android where the shell is fixed).
  useEffect(() => {
    window.scrollTo({ top: 0 });
  }, [location.pathname]);

  return (
    <div className="flex h-full min-h-0 bg-app">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        {desktop ? null : <MobileTopBar title={title} />}
        {!online ? (
          <div className="flex items-center gap-2 border-b border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-400">
            <WifiOff className="size-3.5" />
            <span>{t("error.offline")}</span>
          </div>
        ) : null}
        <main className="min-h-0 flex-1 overflow-y-auto">{children}</main>
        {desktop ? null : <BottomNav />}
      </div>
    </div>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-3 pb-4">
      <div className="min-w-0">
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {subtitle ? <p className="mt-0.5 text-sm text-muted">{subtitle}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}

export function PageBody({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("mx-auto w-full max-w-[1400px] px-4 py-4 md:px-6 md:py-6", className)}>{children}</div>;
}
