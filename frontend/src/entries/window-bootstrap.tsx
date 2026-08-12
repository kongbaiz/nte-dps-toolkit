import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";

import { WindowMotionBoundary } from "@/components/nte/window-motion-boundary";
import { TooltipProvider } from "@/components/ui/tooltip";
import { installBrowserContextMenuSuppression } from "@/lib/browser-context-menu";
import { bootstrapCharacterAvatarCatalog } from "@/lib/character-avatar";
import { bootstrapSettingsPresentation } from "@/lib/settings-presentation";
import { revealPrimaryWindowAfterFirstPaint } from "@/lib/tauri/window-ready";

import "@/index.css";

export interface WindowBootstrapOptions {
  windowRoute: string;
  characterAvatars?: boolean;
}

export function renderWindow(
  content: ReactNode,
  options: WindowBootstrapOptions,
): void {
  document.documentElement.dataset.windowRoute = options.windowRoute;
  bootstrapSettingsPresentation();
  const uninstallBrowserContextMenuSuppression =
    installBrowserContextMenuSuppression();
  if (import.meta.hot) {
    import.meta.hot.dispose(uninstallBrowserContextMenuSuppression);
  }

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <TooltipProvider>
        <WindowMotionBoundary>{content}</WindowMotionBoundary>
      </TooltipProvider>
    </StrictMode>,
  );
  void revealPrimaryWindowAfterFirstPaint().catch((error: unknown) => {
    console.error("show main DPS after the frontend first paint failed", error);
  });

  if (options.characterAvatars !== false) {
    void bootstrapCharacterAvatarCatalog().catch((error: unknown) => {
      console.error("load runtime character avatar catalog failed", error);
    });
  }
}
