import { useMemo, useState } from "react";
import {
  Check,
  Download,
  RefreshCw,
  Server,
  Puzzle,
  Search,
  ShieldCheck,
  Store,
  TriangleAlert,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import {
  currentFrontendLanguage,
  t,
  tf,
  useTranslationRevision,
} from "@/lib/i18n";
import type { ModMarketItem } from "@/lib/tauri/mod-studio-contract";

import {
  localizedModMarketText,
  modMarketLocalStatus,
  filterModMarketItems,
  type ModMarketCategory,
} from "./mod-market-model";
import { useModMarket, type ModMarketInstallState } from "./use-mod-market";

export function ModMarketPanel({
  onInstalled,
}: {
  onInstalled: () => void | Promise<void>;
}) {
  const { state, installStates, refresh, install } = useModMarket(onInstalled);
  useTranslationRevision();
  const language = currentFrontendLanguage();
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<ModMarketCategory>("plugins");
  const visibleMods = useMemo(() => {
    if (state.status !== "ready") {
      return [];
    }
    return filterModMarketItems(state.catalog.mods, category, query, language);
  }, [category, language, query, state]);
  const foundation = category === "foundation";
  const searchLabel = foundation
    ? "Search foundation components"
    : "Search feature plugins";

  return (
    <section className="mt-3 flex min-h-0 flex-1 flex-col overflow-hidden border bg-card">
      <div
        className="flex flex-wrap gap-2 border-b px-4 py-3"
        role="group"
        aria-label={t("Market categories")}
      >
        {(["plugins", "foundation"] as const).map((value) => (
          <Button
            key={value}
            size="sm"
            variant={category === value ? "default" : "outline"}
            aria-pressed={category === value}
            onClick={() => {
              setCategory(value);
              setQuery("");
            }}
          >
            {category === value ? (
              <Check aria-hidden="true" />
            ) : value === "plugins" ? (
              <Puzzle aria-hidden="true" />
            ) : (
              <Server aria-hidden="true" />
            )}
            {t(
              value === "plugins" ? "Feature plugins" : "Foundation components",
            )}
          </Button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-3">
        <div className="relative min-w-52 flex-1">
          <Search
            className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <input
            className="h-9 w-full rounded-md border bg-background pl-9 pr-3 text-sm"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t(searchLabel)}
            aria-label={t(searchLabel)}
          />
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={state.status === "loading"}
          onClick={() => void refresh()}
        >
          <RefreshCw className="size-4" aria-hidden="true" />
          {t("Refresh")}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        <h2 className="text-base font-semibold">
          {t(foundation ? "Foundation components" : "Feature plugins")}
        </h2>
        <p className="mb-4 mt-1 text-sm text-muted-foreground">
          {t(
            foundation
              ? "Host, Loader and driver are shared runtime components, not feature plugins."
              : "Optional combat, account, network and performance features. Runtime components are managed separately.",
          )}
        </p>
        <Alert className="mb-4 border-[var(--console-success)]/40 bg-[var(--console-success)]/5">
          <ShieldCheck aria-hidden="true" />
          <AlertTitle>{t("Privacy-protected downloads")}</AlertTitle>
          <AlertDescription>
            {t(
              "The market uses anonymous read-only requests. It does not send an account, device identifier, local path, Mod list, or telemetry.",
            )}
          </AlertDescription>
        </Alert>

        {state.status === "loading" ? <MarketLoading /> : null}
        {state.status === "error" ? (
          <Alert variant="destructive">
            <TriangleAlert aria-hidden="true" />
            <AlertTitle>{t("Failed to load the Mod Market")}</AlertTitle>
            <AlertDescription>
              {tf(state.error.messageKey, state.error.messageArguments)}
            </AlertDescription>
            <Button
              variant="outline"
              size="sm"
              className="mt-3"
              onClick={() => void refresh()}
            >
              {t("Retry")}
            </Button>
          </Alert>
        ) : null}
        {state.status === "ready" && visibleMods.length === 0 ? (
          <Empty className="min-h-64 border bg-background">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Store aria-hidden="true" />
              </EmptyMedia>
              <EmptyTitle>
                {t(
                  foundation
                    ? "No matching foundation components"
                    : "No matching feature plugins",
                )}
              </EmptyTitle>
              <EmptyDescription>
                {t("Try another name or component ID.")}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : null}
        {state.status === "ready" && visibleMods.length > 0 ? (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(17rem,1fr))] gap-3">
            {visibleMods.map((item) => (
              <MarketItem
                key={item.id}
                item={item}
                language={language}
                installState={installStates[item.id] ?? { status: "idle" }}
                onInstall={() => void install(item.id)}
              />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}

function MarketItem({
  item,
  language,
  installState,
  onInstall,
}: {
  item: ModMarketItem;
  language: ReturnType<typeof currentFrontendLanguage>;
  installState: ModMarketInstallState;
  onInstall: () => void;
}) {
  const local = modMarketLocalStatus(item);
  const installed = local.installed || installState.status === "installed";
  const active = installed && local.current;
  const unreadable = local.unreadable;
  const working = installState.status === "installing";
  const localized = localizedModMarketText(item, language);
  const installError =
    installState.status === "error"
      ? tf(installState.error.messageKey, installState.error.messageArguments)
      : null;
  return (
    <article className="flex min-h-64 flex-col rounded-lg border bg-background p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-heading text-base font-semibold">
            {localized.name}
          </h3>
          <p className="mt-0.5 font-mono text-xs text-muted-foreground">
            {item.id} · v{item.version}
          </p>
        </div>
        <span className="rounded-full border px-2 py-0.5 text-[11px] text-muted-foreground">
          {(item.packageSize / 1024).toFixed(1)} KiB
        </span>
      </div>
      <p className="mt-2 text-xs text-muted-foreground">
        {t(
          {
            plugin: "Plugin component",
            host: "Host component",
            loader: "Loader component",
            driver: "Driver component",
          }[item.component],
        )}
      </p>
      <p className="mt-3 flex-1 text-sm leading-6 text-muted-foreground">
        {localized.summary}
      </p>
      <p className="mt-3 text-xs text-muted-foreground">
        {tf("By {}", [item.author])}
      </p>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {item.capabilities.map((capability) => (
          <span
            key={capability}
            className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
          >
            {capability}
          </span>
        ))}
      </div>
      <Button
        className="mt-4 w-full"
        variant={active ? "outline" : "default"}
        disabled={unreadable !== null || active || working}
        onClick={onInstall}
      >
        {unreadable !== null ? (
          <TriangleAlert aria-hidden="true" />
        ) : active ? (
          <Check aria-hidden="true" />
        ) : (
          <Download aria-hidden="true" />
        )}
        {t(
          unreadable !== null
            ? "Local Mod state unavailable"
            : active
              ? "Installed"
              : installState.status === "installing"
                ? "Downloading and verifying..."
                : installed
                  ? item.component === "plugin"
                    ? "Update plugin"
                    : "Update component"
                  : item.component === "plugin"
                    ? "Download plugin"
                    : "Download component",
        )}
      </Button>
      {unreadable !== null ? (
        <p className="mt-2 text-xs text-destructive" role="alert">
          {t(unreadable.messageKey)}{" "}
          <span className="font-mono">({unreadable.code})</span>
        </p>
      ) : null}
      {installError !== null ? (
        <p className="mt-2 text-xs text-destructive" role="alert">
          {installError}
        </p>
      ) : null}
    </article>
  );
}

function MarketLoading() {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(17rem,1fr))] gap-3">
      {Array.from({ length: 6 }, (_, index) => (
        <div key={index} className="rounded-lg border bg-background p-4">
          <Skeleton className="h-5 w-2/3" />
          <Skeleton className="mt-2 h-3 w-1/2" />
          <Skeleton className="mt-5 h-16 w-full" />
          <Skeleton className="mt-5 h-9 w-full" />
        </div>
      ))}
    </div>
  );
}
