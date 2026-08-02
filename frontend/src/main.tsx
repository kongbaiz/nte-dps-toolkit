import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { TooltipProvider } from "@/components/ui/tooltip";
import { installBrowserContextMenuSuppression } from "@/lib/browser-context-menu";
import { bootstrapCharacterAvatarCatalog } from "@/lib/character-avatar";
import { bootstrapSettingsPresentation } from "@/lib/settings-presentation";
import { revealPrimaryWindowAfterFirstPaint } from "@/lib/tauri/window-ready";
import { resolveWindowRoute, type WindowRoute } from "@/routes/window-route";

import App from "./App";
import "./index.css";

const windowRoute = resolveWindowRoute(getCurrentWindow().label);
document.documentElement.dataset.windowRoute = windowRoute;
bootstrapSettingsPresentation();
const uninstallBrowserContextMenuSuppression =
  installBrowserContextMenuSuppression();
if (import.meta.hot) {
  import.meta.hot.dispose(uninstallBrowserContextMenuSuppression);
}

void bootstrapAndRender(windowRoute);

async function bootstrapAndRender(route: WindowRoute): Promise<void> {
  if (usesCharacterAvatars(route)) {
    try {
      await bootstrapCharacterAvatarCatalog();
    } catch (error: unknown) {
      console.error("load runtime character avatar catalog failed", error);
    }
  }

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <TooltipProvider>
        <App />
      </TooltipProvider>
    </StrictMode>,
  );

  void revealPrimaryWindowAfterFirstPaint().catch((error: unknown) => {
    console.error("show main DPS after the frontend first paint failed", error);
  });
}

function usesCharacterAvatars(route: WindowRoute): boolean {
  return route !== "notification-island" && route !== "unsupported";
}
