import { getCurrentWindow } from "@tauri-apps/api/window";

import { ModStudioPage } from "@/features/mod-studio/mod-studio-page";
import { TechnicalHudPage } from "@/features/technical-hud/technical-hud-page";
import { UnsupportedWindow } from "@/routes/unsupported-window";
import { resolveWindowRoute } from "@/routes/window-route";

function App() {
  const windowLabel = getCurrentWindow().label;
  const route = resolveWindowRoute(windowLabel);

  switch (route) {
    case "mod-studio":
      return <ModStudioPage />;
    case "technical-hud":
      return <TechnicalHudPage />;
    case "unsupported":
      return <UnsupportedWindow windowLabel={windowLabel} />;
  }
}

export default App;
