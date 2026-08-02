import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { TooltipProvider } from "@/components/ui/tooltip";
import { bootstrapSettingsPresentation } from "@/lib/settings-presentation";
import { revealPrimaryWindowAfterFirstPaint } from "@/lib/tauri/window-ready";
import { resolveWindowRoute } from "@/routes/window-route";

import App from "./App";
import "./index.css";

document.documentElement.dataset.windowRoute = resolveWindowRoute(
  getCurrentWindow().label,
);
bootstrapSettingsPresentation();

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
