import { useState } from "react";
import {
  RefreshCw,
  Search,
  UserRound,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import {
  INITIAL_CHARACTER_REQUEST,
  type UserCharacters,
} from "@/lib/tauri/user-characters-client";
import { useUserCharacters } from "./use-user-characters";
import { CharacterSnapshotDetail } from "./character-build-sheet";
import { panelImage } from "./character-build-model";
export { CharacterSnapshotDetail } from "./character-build-sheet";

export function CharacterDataPage() {
  const model = useUserCharacters();
  const [search, setSearch] = useState("");
  const [appliedSearch, setAppliedSearch] = useState("");
  const page = model.snapshot?.page;
  const read = (
    offset = 0,
    selectedUid: string | null = null,
    query = appliedSearch,
  ) =>
    model.load({
      ...INITIAL_CHARACTER_REQUEST,
      offset,
      selectedUid,
      query,
      expectedIdentity: model.snapshot?.connectionIdentity ?? null,
      expectedSnapshotId: page?.snapshotId ?? null,
    });
  return (
    <section className="flex size-full min-h-0 min-w-0 flex-col bg-background">
      <header className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b px-4 py-2">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-base font-semibold">{t("Character Data")}</h1>
            <Badge variant="outline">{t("Read-only snapshot")}</Badge>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "Character progression and equipment observed by the User plugin.",
            )}
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            disabled={model.pending}
            onClick={() => {
              setAppliedSearch("");
              setSearch("");
              model.load(INITIAL_CHARACTER_REQUEST);
            }}
          >
            {t("Check snapshot")}
          </Button>
          <Button
            disabled={model.pending || !model.snapshot?.sdkCompatible}
            onClick={() => {
              setAppliedSearch("");
              setSearch("");
              model.load({
                ...INITIAL_CHARACTER_REQUEST,
                refresh: true,
                expectedIdentity: model.snapshot?.connectionIdentity ?? null,
              });
            }}
          >
            <RefreshCw
              className={cn("size-4", model.pending && "animate-spin")}
              aria-hidden="true"
            />
            {t("Refresh from game")}
          </Button>
        </div>
      </header>
      {model.error ? (
        <Alert variant="destructive" className="m-4 w-auto">
          <AlertTitle>{t("Character data could not be loaded.")}</AlertTitle>
          <AlertDescription>{t(model.error)}</AlertDescription>
        </Alert>
      ) : null}
      <div
        className="border-b px-4 py-1.5 text-xs text-muted-foreground"
        role="status"
        aria-live="polite"
      >
        {model.pending ? (
          t("Reading account snapshot…")
        ) : (
          <SnapshotStatus snapshot={model.snapshot} />
        )}
        {page ? (
          <p className="mt-1">
            {t("Snapshot time")}: {snapshotTime(page.observedUnixUs)} ·{" "}
            {t(
              "Client observation; collected across game-thread batches, not an atomic snapshot.",
            )}
          </p>
        ) : null}
      </div>
      {page ? (
        <div
          className="m-3 grid min-h-0 flex-1 grid-cols-[192px_minmax(0,1fr)] gap-3 max-[800px]:flex max-[800px]:flex-col max-[800px]:overflow-y-auto"
          aria-busy={model.pending}
        >
          <aside
            aria-label={t("Search characters")}
            className="flex min-h-0 flex-col overflow-hidden rounded-xl border bg-card max-[800px]:h-52 max-[800px]:shrink-0"
          >
            <form
              className="flex gap-2 border-b p-3"
              onSubmit={(e) => {
                e.preventDefault();
                setAppliedSearch(search);
                read(0, null, search);
              }}
            >
              <input
                className="h-9 min-w-0 flex-1 rounded-lg border bg-background px-3 text-sm"
                aria-label={t("Search characters")}
                placeholder={t("ID / name")}
                maxLength={128}
                value={search}
                disabled={model.pending}
                onChange={(e) => setSearch(e.target.value)}
              />
              <Button
                type="submit"
                variant="outline"
                size="icon"
                disabled={model.pending}
                aria-label={t("Search")}
              >
                <Search className="size-4" />
              </Button>
            </form>
            <div className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
              {page.records.length === 0 ? (
                <p className="p-4 text-sm text-muted-foreground">
                  {t("No characters match the current search.")}
                </p>
              ) : (
                page.records.map((r) => (
                  <button
                    type="button"
                    key={r.uid}
                    disabled={model.pending}
                    aria-pressed={page.detail?.summary.uid === r.uid}
                    className={cn(
                      "flex w-full items-center gap-2.5 rounded-lg border border-transparent px-2 py-2 text-left transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-60",
                      page.detail?.summary.uid === r.uid &&
                        "border-primary/20 bg-primary/10 text-foreground hover:bg-primary/15",
                    )}
                    onClick={() => read(page.offset, r.uid)}
                  >
                    {panelImage("avatar", r.itemId) ? (
                      <img
                        src={panelImage("avatar", r.itemId)!}
                        alt=""
                        className="size-9 shrink-0 rounded-full bg-muted object-cover"
                        draggable={false}
                      />
                    ) : (
                      <UserRound
                        className="size-6 shrink-0"
                        aria-hidden="true"
                      />
                    )}
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium">
                        {r.name || r.itemId || t("Unknown character")}
                      </span>
                      <span className="block text-[11px] opacity-70">
                        {r.itemId} · {t("Level")}{" "}
                        {r.level ?? t("Unknown value")}
                      </span>
                    </span>
                  </button>
                ))
              )}
            </div>
            <footer className="flex items-center justify-between gap-1 border-t px-2 py-2">
              <Button
                size="sm"
                variant="ghost"
                aria-label={t("Previous")}
                title={t("Previous")}
                disabled={model.pending || page.offset === 0}
                onClick={() => read(page.offset - 16)}
              >
                <ChevronLeft aria-hidden="true" />
              </Button>
              <span className="whitespace-nowrap text-xs tabular-nums text-muted-foreground">
                {tf("Total {} entries", [String(page.total)])}
              </span>
              <Button
                size="sm"
                variant="ghost"
                aria-label={t("Next")}
                title={t("Next")}
                disabled={model.pending || page.offset + 16 >= page.total}
                onClick={() => read(page.offset + 16)}
              >
                <ChevronRight aria-hidden="true" />
              </Button>
            </footer>
          </aside>
          <main className="min-h-0 min-w-0 overflow-y-auto rounded-xl border bg-card p-3 max-[800px]:shrink-0 max-[800px]:overflow-visible">
            {!page.complete ? (
              <Alert className="mb-4">
                <AlertTitle>{t("Partial snapshot")}</AlertTitle>
                <AlertDescription>
                  {t(
                    "Some account data could not be observed. Missing values are not zero or unequipped.",
                  )}
                </AlertDescription>
              </Alert>
            ) : null}
            {page.detail ? (
              <CharacterSnapshotDetail detail={page.detail} />
            ) : (
              <p className="p-6 text-sm text-muted-foreground">
                {t("No owned characters in this snapshot.")}
              </p>
            )}
          </main>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center p-8 text-center text-sm text-muted-foreground">
          {t(
            "Load NTE_PluginUser in Mod Workshop, then refresh from the game. Resource files are not used as account data.",
          )}
        </div>
      )}
    </section>
  );
}
function SnapshotStatus({ snapshot }: { snapshot: UserCharacters | null }) {
  if (!snapshot) return t("No account snapshot loaded.");
  if (!snapshot.sdkCompatible) return t("The User plugin SDK is not ready.");
  if (snapshot.state === "failed")
    return t("Account refresh failed. Check the User plugin before retrying.");
  if (snapshot.state === "canceled" || snapshot.state === "stopped")
    return t("Account refresh stopped.");
  if (snapshot.dirty)
    return t(
      "The account snapshot is missing or outdated. Refresh from the game.",
    );
  return t(
    "Showing the last observed account snapshot. Check or refresh to detect changes.",
  );
}
function snapshotTime(value: string) {
  const ms = Number(BigInt(value) / 1000n);
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
