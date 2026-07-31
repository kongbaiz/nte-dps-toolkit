import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { TooltipProvider } from "@/components/ui/tooltip";
import { bootstrapSettingsPresentation } from "@/lib/settings-presentation";
import { revealConsoleAfterFirstPaint } from "@/lib/tauri/window-ready";

import App from "./App";
import "./index.css";

bootstrapSettingsPresentation();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <TooltipProvider>
      <App />
    </TooltipProvider>
  </StrictMode>,
);

void revealConsoleAfterFirstPaint().catch((error: unknown) => {
  console.error("show Console after the frontend first paint failed", error);
});
