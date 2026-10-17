import { lazy, Suspense } from "react";
import { createHashRouter, Navigate, Outlet, useLocation } from "react-router-dom";

import { AppShell } from "@/components/layout";
import { LoadingBlock } from "@/components/ui";
import { useT } from "@/i18n";
import { useSettings } from "@/stores/settings";

import { AssistantPage } from "@/pages/AssistantPage";
import { CalendarPage } from "@/pages/CalendarPage";
import { CanvasPage } from "@/pages/CanvasPage";
import { CourseDetailPage } from "@/pages/CourseDetailPage";
import { CoursesPage } from "@/pages/CoursesPage";
import { DashboardPage } from "@/pages/DashboardPage";
import { MaterialsPage } from "@/pages/MaterialsPage";
import { PlannerPage } from "@/pages/PlannerPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { WelcomePage } from "@/pages/WelcomePage";

// The PDF reader pulls in pdf.js (~1.2 MB); keep it out of the startup bundle.
const MaterialViewerPage = lazy(() =>
  import("@/pages/MaterialViewerPage").then((module) => ({ default: module.MaterialViewerPage })),
);

/**
 * Routes are hash-based: the packaged app is served from a custom protocol, and
 * hash routing keeps deep links working after a WebView reload on both Windows
 * and Android without server-side rewrites.
 */
function ShellLayout() {
  const t = useT();
  const location = useLocation();
  const onboardingDone = useSettings((s) => s.onboardingDone);

  if (!onboardingDone && location.pathname !== "/welcome") {
    return <Navigate to="/welcome" replace />;
  }

  const title = titleFor(location.pathname, t);
  return (
    <AppShell title={title}>
      <Suspense fallback={<LoadingBlock />}>
        <Outlet />
      </Suspense>
    </AppShell>
  );
}

function titleFor(pathname: string, t: (key: string) => string): string {
  if (pathname === "/") return t("nav.home");
  if (pathname.startsWith("/courses")) return t("nav.courses");
  if (pathname.startsWith("/materials")) return t("nav.materials");
  if (pathname.startsWith("/assistant")) return t("nav.assistant");
  if (pathname.startsWith("/planner")) return t("nav.planner");
  if (pathname.startsWith("/calendar")) return t("nav.calendar");
  if (pathname.startsWith("/settings")) return t("nav.settings");
  if (pathname.startsWith("/canvas")) return t("canvas.title");
  return t("app.name");
}

export const router = createHashRouter([
  { path: "/welcome", element: <WelcomePage /> },
  {
    path: "/",
    element: <ShellLayout />,
    children: [
      { index: true, element: <DashboardPage /> },
      { path: "courses", element: <CoursesPage /> },
      { path: "courses/:courseId", element: <CourseDetailPage /> },
      { path: "materials", element: <MaterialsPage /> },
      { path: "materials/:materialId", element: <MaterialViewerPage /> },
      { path: "assistant", element: <AssistantPage /> },
      { path: "planner", element: <PlannerPage /> },
      { path: "calendar", element: <CalendarPage /> },
      { path: "canvas", element: <CanvasPage /> },
      { path: "settings", element: <SettingsPage /> },
      { path: "*", element: <Navigate to="/" replace /> },
    ],
  },
]);
