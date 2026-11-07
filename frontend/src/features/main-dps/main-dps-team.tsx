import { useState } from "react";
import { Check, Copy, Eye, EyeOff } from "lucide-react";

import { AnimatedNumber } from "@/components/nte/animated-number";
import { MaxHpCompressionEffect } from "@/components/nte/max-hp-compression-effect";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuPopup,
  ContextMenuPortal,
  ContextMenuPositioner,
  ContextMenuTrigger,
  Menu,
  MenuItem,
  MenuPopup,
  MenuPortal,
  MenuPositioner,
  MenuTrigger,
} from "@/components/ui/menu";
import { useCharacterAvatar } from "@/hooks/use-character-avatar";
import { useCloseOnWindowBlur } from "@/hooks/use-close-on-window-blur";
import { characterAttributeUrl } from "@/lib/character-attribute";
import { t, tf } from "@/lib/i18n";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import type { MainDpsCharacterSnapshot } from "@/lib/tauri/generated/MainDpsCharacterSnapshot";
import type { MainDpsDetailFilter } from "@/lib/tauri/generated/MainDpsDetailFilter";
import type { MainDpsSnapshot } from "@/lib/tauri/generated/MainDpsSnapshot";
import { cn } from "@/lib/utils";

import {
  characterAccent,
  damagePercent,
  formatDuration,
  formatMainMetric,
  mainCharacterListState,
} from "./main-dps-model";
import type { useMainDps } from "./use-main-dps";

type MainDpsOpenDetailFilter = Exclude<MainDpsDetailFilter, "qteType">;
type MainDpsControls = ReturnType<typeof useMainDps>;

export function MainDpsTeamSection({
  snapshot,
  hiddenCharacters,
  onHideCharacter,
  onShowAll,
  pending,
  apply,
  effect,
  startCapture,
  importReplay,
}: {
  snapshot: MainDpsSnapshot;
  hiddenCharacters: ReadonlySet<number>;
  onHideCharacter(id: number): void;
  onShowAll(): void;
  pending: string | null;
  apply: MainDpsControls["apply"];
  effect: MainDpsControls["effect"];
  startCapture(): void;
  importReplay(kind: "json" | "pcapng"): Promise<void>;
}) {
  const [context, setContext] = useState<{
    row: MainDpsCharacterSnapshot;
  } | null>(null);
  useCloseOnWindowBlur(context !== null, () => setContext(null));
  const characters = snapshot.readout.characters.filter(
    (row) => !hiddenCharacters.has(row.characterId),
  );
  const characterListState = mainCharacterListState(
    snapshot.readout.damageAttribution.totalDamage,
    snapshot.readout.characters.length,
    characters.length,
  );
  const captureRunning = snapshot.actions.canStopCapture;

  return (
    <section className="main-dps-team-section flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex min-w-0 items-center gap-3">
        <h1 className="text-sm font-medium">{t("Team")}</h1>
        <span className="main-dps-team-hint min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {t(
            "Click a character to view details; right-click to copy data or hide the character",
          )}
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={!snapshot.actions.teamDetailsAvailable}
          onClick={() =>
            void effect("team-details", () =>
              mainDpsClient.openTeamDetails("all"),
            )
          }
        >
          {t("Team Combat Details")}
        </Button>
      </div>
      {(snapshot.readout.damageAttribution.totalDamage > 0 ||
        snapshot.readout.damageAttribution.maxHpReduction > 0) && (
        <DamageAttributionStrip
          attribution={snapshot.readout.damageAttribution}
          visible={snapshot.display.attributions}
          onOpen={(filter) =>
            void effect(`team-details-${filter}`, () =>
              mainDpsClient.openTeamDetails(filter),
            )
          }
        />
      )}
      {characterListState === "combat-empty" ? (
        <EmptyCombat
          gameDetected={snapshot.gameDetected}
          gameDetectionStatus={snapshot.gameDetectionStatus}
          captureRunning={captureRunning}
          importReplay={importReplay}
          pending={pending}
          start={startCapture}
          redetect={() =>
            void effect("redetect", async () => {
              apply(await mainDpsClient.getSnapshot());
            })
          }
        />
      ) : characterListState === "unattributed" ? (
        <div className="m-auto text-sm text-muted-foreground">
          {t(
            "No character-attributed damage under the current accounting mode",
          )}
        </div>
      ) : characterListState === "hidden" ? (
        <div className="m-auto flex flex-col items-center gap-3 text-muted-foreground">
          <span>{t("All ranking rows are hidden")}</span>
          <Button variant="outline" onClick={onShowAll}>
            {t("Show all characters")}
          </Button>
        </div>
      ) : (
        <ContextMenu
          open={context !== null}
          onOpenChange={(open) => {
            if (!open) setContext(null);
          }}
        >
          <ContextMenuTrigger
            render={
              <div
                className="main-dps-team-list"
                style={
                  {
                    "--main-dps-team-row-count": characters.length,
                  } as React.CSSProperties
                }
              >
                {characters.map((row, index) => (
                  <CharacterRow
                    key={row.characterId}
                    row={row}
                    index={index}
                    onClick={() =>
                      void effect(`character-details-${row.characterId}`, () =>
                        mainDpsClient.openCharacterDetails(row.characterId),
                      )
                    }
                    onContext={() => setContext({ row })}
                  />
                ))}
              </div>
            }
          />
          {context !== null && (
            <ContextMenuPortal>
              <ContextMenuPositioner>
                <ContextMenuPopup
                  aria-label={t("Character actions")}
                  className="motion-popover min-w-52"
                >
                  <ContextMenuItem
                    className="gap-2 px-3 py-2"
                    onClick={() =>
                      void effect(
                        `character-details-${context.row.characterId}`,
                        () =>
                          mainDpsClient.openCharacterDetails(
                            context.row.characterId,
                          ),
                      )
                    }
                  >
                    <Eye className="size-4" aria-hidden="true" />
                    {t("View combat details")}
                  </ContextMenuItem>
                  <ContextMenuItem
                    className="gap-2 px-3 py-2"
                    onClick={() =>
                      void navigator.clipboard.writeText(
                        [
                          tf("Character: {}", [context.row.name]),
                          tf("DPS: {}", [formatMainMetric(context.row.dps)]),
                          tf("Damage: {}", [
                            formatMainMetric(context.row.damage),
                          ]),
                          tf("Share: {}%", [
                            context.row.damageSharePercent.toFixed(1),
                          ]),
                          tf("Taken: {}", [
                            formatMainMetric(context.row.damageTaken),
                          ]),
                        ].join("\n"),
                      )
                    }
                  >
                    <Copy className="size-4" aria-hidden="true" />
                    {t("Copy values")}
                  </ContextMenuItem>
                  <ContextMenuItem
                    className="gap-2 px-3 py-2"
                    onClick={() => onHideCharacter(context.row.characterId)}
                  >
                    <EyeOff className="size-4" aria-hidden="true" />
                    {t("Hide from ranking")}
                  </ContextMenuItem>
                </ContextMenuPopup>
              </ContextMenuPositioner>
            </ContextMenuPortal>
          )}
        </ContextMenu>
      )}
    </section>
  );
}

function CharacterRow({
  row,
  index,
  onClick,
  onContext,
}: {
  row: MainDpsCharacterSnapshot;
  index: number;
  onClick(): void;
  onContext(event: React.MouseEvent): void;
}) {
  const avatar = useCharacterAvatar(row.characterId);
  const attribute = characterAttributeUrl(row.attribute);
  const accent = characterAccent(row.characterId, row.color);
  return (
    <button
      type="button"
      className="main-dps-character-row motion-list-item grid items-center gap-2 overflow-hidden rounded-lg border bg-card text-left transition-colors hover:bg-muted/60"
      style={
        {
          "--main-dps-character-accent": accent,
          "--motion-index": index,
        } as React.CSSProperties
      }
      onClick={onClick}
      onContextMenu={onContext}
    >
      <span className="main-dps-character-attribute grid size-5 place-items-center text-xs font-medium text-[var(--main-dps-character-accent)]">
        {attribute === null ? (
          `#${index + 1}`
        ) : (
          <img src={attribute} alt="" className="size-5 object-contain" />
        )}
      </span>
      <span className="main-dps-character-avatar grid size-10 place-items-center overflow-hidden rounded-lg bg-muted">
        {avatar === null ? (
          row.name.slice(0, 1)
        ) : (
          <img src={avatar} alt="" className="size-full object-cover" />
        )}
      </span>
      <span className="min-w-0">
        <strong className="block truncate text-sm">{row.name}</strong>
        <span className="main-dps-character-secondary block truncate text-xs text-muted-foreground">
          {tf("{} hits / {}", [row.hits, formatDuration(row.durationSeconds)])}
        </span>
      </span>
      <span className="min-w-0 text-right">
        <strong className="block whitespace-nowrap text-sm tabular-nums">
          <AnimatedNumber value={row.dps} format={formatMainMetric} /> DPS
        </strong>
        <span className="main-dps-character-secondary block truncate whitespace-nowrap text-xs text-muted-foreground">
          {tf("Damage {} / Share {}% / Taken {}", [
            formatMainMetric(row.damage),
            row.damageSharePercent.toFixed(1),
            formatMainMetric(row.damageTaken),
          ])}
        </span>
      </span>
      <span className="absolute inset-x-2 bottom-0.5 h-0.5 overflow-hidden rounded-full bg-muted">
        <span
          className="motion-bar block h-full rounded-full bg-[var(--main-dps-character-accent)]"
          style={{ width: `${Math.min(100, row.damageSharePercent)}%` }}
        />
      </span>
    </button>
  );
}

function DamageAttributionStrip({
  attribution,
  visible,
  onOpen,
}: {
  attribution: MainDpsSnapshot["readout"]["damageAttribution"];
  visible: MainDpsSnapshot["display"]["attributions"];
  onOpen(filter: MainDpsOpenDetailFilter): void;
}) {
  const percentage = (damage: number) =>
    `${damagePercent(damage, attribution.totalDamage).toFixed(1)}%`;
  const characterDamage = attribution.separateReactionDamage
    ? attribution.characterDirectDamage
    : attribution.characterDirectDamage + attribution.characterReactionDamage;
  const items = [
    ...(visible.includes("character")
      ? [
          {
            label: t(
              attribution.separateReactionDamage
                ? "Character direct"
                : "Character attributed",
            ),
            value: percentage(characterDamage),
            filter: attribution.separateReactionDamage
              ? ("characterDirect" as const)
              : ("characterAttributed" as const),
          },
        ]
      : []),
    ...(visible.includes("reaction") && attribution.separateReactionDamage
      ? [
          {
            label: t("Reaction Damage"),
            value: percentage(attribution.characterReactionDamage),
            filter: "reactionDamage" as const,
          },
        ]
      : visible.includes("reaction") && attribution.characterReactionDamage > 0
        ? [
            {
              label: tf("Includes reaction damage: {}%", [
                damagePercent(
                  attribution.characterReactionDamage,
                  attribution.totalDamage,
                ).toFixed(1),
              ]),
              value: null,
              filter: "reactionDamage" as const,
            },
          ]
        : []),
    ...(visible.includes("shared")
      ? [
          {
            label: t("Shared mechanics"),
            value: percentage(attribution.sharedDamage),
            filter: "sharedMechanics" as const,
          },
        ]
      : []),
    ...(visible.includes("unattributed")
      ? [
          {
            label: t("Unattributed"),
            value: percentage(attribution.unattributedDamage),
            filter: "unattributed" as const,
          },
        ]
      : []),
  ];

  const showMaxHpReduction = visible.includes("max_hp_reduction");
  if (items.length === 0 && !showMaxHpReduction) return null;

  return (
    <div className="main-dps-attribution flex min-w-0 flex-wrap items-center gap-1.5 text-xs">
      <span className="font-medium text-muted-foreground">
        {t("Damage attribution")}
      </span>
      {items.map((item) => (
        <button
          type="button"
          key={item.label}
          className="rounded-md border bg-background px-2 py-1 whitespace-nowrap transition-colors hover:bg-muted"
          onClick={() => onOpen(item.filter)}
        >
          {item.label}
          {item.value !== null && ` ${item.value}`}
        </button>
      ))}
      {showMaxHpReduction && (
        <MaxHpCompressionEffect
          key={attribution.maxHpReduction}
          label={t("Life reduction")}
          value={
            attribution.includeMaxHpReductionInTotalDamage
              ? `${formatMainMetric(attribution.maxHpReduction)} / ${percentage(attribution.maxHpReduction)}`
              : formatMainMetric(attribution.maxHpReduction)
          }
          active={attribution.maxHpReduction > 0}
          className="px-2 py-1"
        />
      )}
    </div>
  );
}
function EmptyCombat({
  gameDetected,
  gameDetectionStatus,
  captureRunning,
  importReplay,
  pending,
  start,
  redetect,
}: {
  gameDetected: boolean;
  gameDetectionStatus: "running" | "notRunning" | "probeFailed";
  captureRunning: boolean;
  importReplay(kind: "json" | "pcapng"): Promise<void>;
  pending: string | null;
  start(): void;
  redetect(): void;
}) {
  const [replayOpen, setReplayOpen] = useState(false);
  useCloseOnWindowBlur(replayOpen, () => setReplayOpen(false));
  const chooseReplay = (kind: "json" | "pcapng") => {
    setReplayOpen(false);
    void importReplay(kind);
  };

  return (
    <div className="m-auto flex w-full max-w-[26rem] flex-col gap-3 rounded-xl border bg-card p-5">
      <h2 className="text-sm">{t("Ready for your next combat")}</h2>
      <ol className="space-y-2.5 text-sm">
        <li className="flex items-center gap-3">
          <span
            className={cn(
              "grid size-6 place-items-center",
              gameDetected ? "text-success" : "text-muted-foreground",
            )}
          >
            {gameDetected ? <Check className="size-5" /> : "1"}
          </span>
          {t(
            gameDetectionStatus === "probeFailed"
              ? "Game process detection failed."
              : "Start HTGame.exe",
          )}
        </li>
        <li className="flex items-center gap-3">
          <span
            className={cn(
              "grid size-6 place-items-center",
              captureRunning ? "text-success" : "text-muted-foreground",
            )}
          >
            {captureRunning ? <Check className="size-5" /> : "2"}
          </span>
          {t("Start live capture")}
        </li>
        <li className="flex items-center gap-3">
          <span className="grid size-6 place-items-center text-muted-foreground">
            3
          </span>
          {t("Enter combat and deal damage")}
        </li>
      </ol>
      <div className="flex gap-2">
        {!gameDetected ? (
          <Button
            size="sm"
            variant="outline"
            disabled={pending !== null}
            onClick={redetect}
          >
            {t("Re-detect")}
          </Button>
        ) : null}
        <Button
          size="sm"
          disabled={captureRunning || pending !== null}
          onClick={start}
        >
          {t("Start Capture")}
        </Button>
        <div className="relative">
          <Menu open={replayOpen} onOpenChange={setReplayOpen}>
            <MenuTrigger
              disabled={pending !== null}
              render={<Button size="sm" variant="outline" />}
            >
              {t("Import Replay")}
            </MenuTrigger>
            <MenuPortal>
              <MenuPositioner align="start" side="top" sideOffset={8}>
                <MenuPopup
                  aria-label={t("Import Replay")}
                  className="motion-popover flex min-w-40 flex-col gap-1"
                >
                  <MenuItem
                    className="px-3 py-2"
                    onClick={() => chooseReplay("json")}
                  >
                    {t("JSON replay")}
                  </MenuItem>
                  <MenuItem
                    className="px-3 py-2"
                    onClick={() => chooseReplay("pcapng")}
                  >
                    {t("PCAPNG replay")}
                  </MenuItem>
                </MenuPopup>
              </MenuPositioner>
            </MenuPortal>
          </Menu>
        </div>
      </div>
      {captureRunning ? (
        <p className="text-xs text-muted-foreground">
          {t("Waiting for the first damage event...")}
        </p>
      ) : null}
    </div>
  );
}
