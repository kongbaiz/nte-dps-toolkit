import { getCurrentWindow } from "@tauri-apps/api/window";
import { lazy, Suspense } from "react";

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

function App() {
  const windowLabel = getCurrentWindow().label;
  const route = resolveWindowRoute(windowLabel);

  switch (route) {
    case "abyss-values":
      return (
        <Suspense fallback={null}>
          <AbyssValuesPage />
        </Suspense>
      );
    case "console":
      return <ConsolePage />;
    case "main-dps":
      return (
        <Suspense fallback={null}>
          <MainDpsPage />
        </Suspense>
      );
    case "technical-hud":
      return <TechnicalHudPage />;
    case "unsupported":
      return <UnsupportedWindow windowLabel={windowLabel} />;
  }
}

export default App;
