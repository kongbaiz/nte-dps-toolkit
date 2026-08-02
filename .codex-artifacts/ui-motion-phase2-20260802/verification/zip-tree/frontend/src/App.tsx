import { getCurrentWindow } from "@tauri-apps/api/window";
import { lazy, Suspense, type ReactNode } from "react";

import { MotionRouteLoading } from "@/components/nte/motion-route-loading";
import { WindowMotionBoundary } from "@/components/nte/window-motion-boundary";
import { ConsolePage } from "@/features/console/console-page";
import { TechnicalHudPage } from "@/features/technical-hud/technical-hud-page";
import { UnsupportedWindow } from "@/routes/unsupported-window";
import { resolveWindowRoute } from "@/routes/window-route";

const AbyssValuesPage = lazy(() =>
  import("@/features/abyss-values/abyss-values-page").then((module) => ({
    default: module.AbyssValuesPage,
  })),
);
const MainDpsPage = lazy(() =>
  import("@/features/main-dps/main-dps-page").then((module) => ({
    default: module.MainDpsPage,
  })),
);
const MainDpsDetailPage = lazy(() =>
  import("@/features/main-dps/main-dps-detail-page").then((module) => ({
    default: module.MainDpsDetailPage,
  })),
);
const IslandPage = lazy(() =>
  import("@/features/island/island-page").then((module) => ({
    default: module.IslandPage,
  })),
);

function App() {
  const windowLabel = getCurrentWindow().label;
  const route = resolveWindowRoute(windowLabel);
  let content: ReactNode;

  switch (route) {
    case "abyss-values":
      content = (
        <Suspense fallback={<MotionRouteLoading />}>
          <AbyssValuesPage />
        </Suspense>
      );
      break;
    case "console":
      content = <ConsolePage />;
      break;
    case "combat-details":
      content = (
        <Suspense fallback={<MotionRouteLoading />}>
          <MainDpsDetailPage />
        </Suspense>
      );
      break;
    case "main-dps":
      content = (
        <Suspense fallback={<MotionRouteLoading />}>
          <MainDpsPage />
        </Suspense>
      );
      break;
    case "notification-island":
      content = (
        <Suspense fallback={<MotionRouteLoading compact />}>
          <IslandPage />
        </Suspense>
      );
      break;
    case "technical-hud":
      content = <TechnicalHudPage />;
      break;
    case "unsupported":
      content = <UnsupportedWindow windowLabel={windowLabel} />;
      break;
  }

  return <WindowMotionBoundary>{content}</WindowMotionBoundary>;
}

export default App;
