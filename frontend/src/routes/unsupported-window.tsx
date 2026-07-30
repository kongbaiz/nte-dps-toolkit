import { ShieldCheck } from "lucide-react";

import { t, tf } from "@/lib/i18n";

interface UnsupportedWindowProps {
  windowLabel: string;
}

export function UnsupportedWindow({ windowLabel }: UnsupportedWindowProps) {
  return (
    <main className="hud-surface grid min-h-screen place-items-center p-6 text-center">
      <div className="flex max-w-sm flex-col items-center gap-3">
        <ShieldCheck className="text-amber-300" aria-hidden="true" />
        <p className="font-medium">{t("This window is not registered.")}</p>
        <p className="font-mono text-xs text-slate-500">
          {tf("Window label: {0}", [windowLabel])}
        </p>
      </div>
    </main>
  );
}
