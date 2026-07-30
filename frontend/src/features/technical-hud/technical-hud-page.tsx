import type { ReactNode } from "react";
import {
  Activity,
  Cpu,
  MousePointer2,
  Pin,
  Radio,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { t, tf } from "@/lib/i18n";

import {
  bridgeTone,
  localeSummary,
  type TechnicalPageState,
} from "./technical-view-model";
import { useTechnicalState } from "./use-technical-state";

export function TechnicalHudPage() {
  const { state, refresh, setPassthrough, setAlwaysOnTop } =
    useTechnicalState();

  return (
    <main className="hud-surface relative min-h-screen overflow-hidden p-3">
      <div className="pointer-events-none absolute inset-0 opacity-45">
        <div className="absolute -top-20 left-1/3 size-52 rounded-full bg-cyan-400/15 blur-3xl" />
        <div className="absolute -right-16 bottom-0 size-44 rounded-full bg-violet-500/15 blur-3xl" />
      </div>

      <Card className="relative h-[calc(100vh-1.5rem)] gap-0 border border-white/10 bg-slate-950/88 py-0 shadow-2xl shadow-black/50 ring-1 ring-cyan-300/10 backdrop-blur-xl">
        <CardHeader
          data-tauri-drag-region
          className="border-b border-white/8 px-4 py-3"
        >
          <div
            data-tauri-drag-region
            className="flex items-center gap-3 select-none"
          >
            <div
              data-tauri-drag-region
              className="grid size-9 place-items-center rounded-lg border border-cyan-300/20 bg-cyan-300/10 text-cyan-200"
            >
              <Activity data-tauri-drag-region aria-hidden="true" />
            </div>
            <div data-tauri-drag-region className="min-w-0">
              <CardTitle
                data-tauri-drag-region
                className="truncate text-[15px] font-semibold tracking-tight"
              >
                {t("Tauri HUD migration spike")}
              </CardTitle>
              <CardDescription
                data-tauri-drag-region
                className="mt-0.5 text-[11px] tracking-[0.16em] text-cyan-100/45 uppercase"
              >
                {t("Phase 1 · Desktop bridge")}
              </CardDescription>
            </div>
          </div>
          <CardAction>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("Refresh technical state")}
                    onClick={() => void refresh()}
                  />
                }
              >
                <RefreshCw data-icon="inline-start" aria-hidden="true" />
              </TooltipTrigger>
              <TooltipContent>{t("Refresh technical state")}</TooltipContent>
            </Tooltip>
          </CardAction>
        </CardHeader>

        <TechnicalContent
          state={state}
          onRetry={refresh}
          onPassthroughChange={setPassthrough}
          onAlwaysOnTopChange={setAlwaysOnTop}
        />
      </Card>
    </main>
  );
}

interface TechnicalContentProps {
  state: TechnicalPageState;
  onRetry: () => Promise<void>;
  onPassthroughChange: (enabled: boolean) => Promise<void>;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
}

function TechnicalContent({
  state,
  onRetry,
  onPassthroughChange,
  onAlwaysOnTopChange,
}: TechnicalContentProps) {
  if (state.status === "loading") {
    return (
      <CardContent className="grid flex-1 place-items-center">
        <div className="flex items-center gap-3 text-sm text-slate-400">
          <Radio className="animate-pulse text-cyan-300" aria-hidden="true" />
          {t("Waiting for Rust bridge...")}
        </div>
      </CardContent>
    );
  }

  if (state.status === "error") {
    return (
      <CardContent className="grid flex-1 place-items-center">
        <div className="flex max-w-sm flex-col items-center gap-3 text-center">
          <ShieldCheck className="text-amber-300" aria-hidden="true" />
          <div>
            <p className="font-medium">{t("Rust bridge unavailable")}</p>
            <p className="mt-1 text-xs text-slate-400">
              {tf(state.error.messageKey, state.error.messageArguments)}
            </p>
          </div>
          <Button variant="outline" size="sm" onClick={() => void onRetry()}>
            <RefreshCw data-icon="inline-start" aria-hidden="true" />
            {t("Retry")}
          </Button>
        </div>
      </CardContent>
    );
  }

  const { snapshot } = state;
  const tone = bridgeTone(snapshot.bridgeStatus);
  const locales = localeSummary(snapshot.supportedLocales);

  return (
    <>
      <CardContent className="flex flex-1 flex-col gap-3 px-4 py-3">
        <section className="grid grid-cols-3 gap-2">
          <Metric
            icon={<Cpu aria-hidden="true" />}
            label={t("Rust core bridge")}
            value={
              tone === "unknown"
                ? tf("Unknown status: {0}", [snapshot.bridgeStatus])
                : t(snapshot.bridgeStatus === "ready" ? "Ready" : "Degraded")
            }
            accent={tone === "ready"}
          />
          <Metric
            icon={<Radio aria-hidden="true" />}
            label={t("Ordered Channel")}
            value={`#${snapshot.sequence}`}
            accent
          />
          <Metric
            icon={<Activity aria-hidden="true" />}
            label={t("Uptime")}
            value={formatUptime(snapshot.uptimeMs)}
          />
        </section>

        <Separator className="bg-white/8" />

        <section className="grid grid-cols-2 gap-2">
          <Control
            icon={<MousePointer2 aria-hidden="true" />}
            title={t("Mouse passthrough")}
            description={t("Manage passthrough from the control console.")}
          >
            <Switch
              aria-label={t("Mouse passthrough")}
              checked={snapshot.window.passthrough}
              onCheckedChange={(enabled) => void onPassthroughChange(enabled)}
            />
          </Control>

          <Control
            icon={<Pin aria-hidden="true" />}
            title={t("Always on top")}
            description={t("Keep the HUD above other windows.")}
          >
            <Switch
              aria-label={t("Always on top")}
              checked={snapshot.window.alwaysOnTop}
              onCheckedChange={(enabled) => void onAlwaysOnTopChange(enabled)}
            />
          </Control>
        </section>
      </CardContent>

      <CardFooter className="grid grid-cols-[1fr_auto] gap-3 border-white/8 bg-white/3 px-4 py-2.5">
        <div className="min-w-0">
          <p className="truncate text-[11px] text-slate-400">
            {t("Locales")}: {locales ?? t("No locales reported")}
          </p>
          <p className="mt-0.5 truncate font-mono text-[10px] text-slate-500">
            {snapshot.windowLabel} ·{" "}
            {tf("Adapter {0}", [snapshot.adapterVersion])}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Badge
            variant="outline"
            className="border-white/10 bg-white/4 font-mono text-[10px] text-slate-300"
          >
            {tf("Contract v{0}", [String(snapshot.contractVersion)])}
          </Badge>
          <Badge className="bg-cyan-300/15 text-cyan-100">
            <span className="size-1.5 rounded-full bg-cyan-300 shadow-[0_0_8px_rgba(103,232,249,0.9)]" />
            {t("Live")}
          </Badge>
        </div>
      </CardFooter>
    </>
  );
}

interface MetricProps {
  icon: ReactNode;
  label: string;
  value: string;
  accent?: boolean;
}

function Metric({ icon, label, value, accent = false }: MetricProps) {
  return (
    <div className="rounded-lg border border-white/8 bg-white/3 p-2.5">
      <div className="flex items-center gap-1.5 text-[10px] tracking-wide text-slate-500 uppercase [&_svg]:size-3">
        {icon}
        <span className="truncate">{label}</span>
      </div>
      <p
        className={
          accent
            ? "mt-1.5 truncate font-mono text-sm font-semibold text-cyan-200"
            : "mt-1.5 truncate font-mono text-sm font-semibold text-slate-200"
        }
      >
        {value}
      </p>
    </div>
  );
}

interface ControlProps {
  icon: ReactNode;
  title: string;
  description: string;
  children: ReactNode;
}

function Control({ icon, title, description, children }: ControlProps) {
  return (
    <div className="flex min-w-0 items-start gap-2.5 rounded-lg border border-white/8 bg-white/3 p-2.5">
      <div className="mt-0.5 text-cyan-200/70 [&_svg]:size-3.5">{icon}</div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-medium text-slate-200">{title}</p>
        <p className="mt-0.5 line-clamp-2 text-[10px] leading-4 text-slate-500">
          {description}
        </p>
      </div>
      <div className="pt-0.5">{children}</div>
    </div>
  );
}

function formatUptime(uptimeMs: string): string {
  const seconds = BigInt(uptimeMs) / 1000n;
  const minutes = seconds / 60n;
  const remainingSeconds = seconds % 60n;
  return `${minutes.toString().padStart(2, "0")}:${remainingSeconds
    .toString()
    .padStart(2, "0")}`;
}
