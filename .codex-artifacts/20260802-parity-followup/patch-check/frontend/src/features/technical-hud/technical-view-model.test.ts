import { describe, expect, it } from "vitest";

import type { TechnicalSnapshot } from "@/lib/tauri/technical-contract";

import {
  acceptSnapshot,
  bridgeTone,
  captureAction,
  captureMessage,
  formatHudDuration,
  formatHudNumber,
  hudCharacterName,
  hudDataTone,
  hudModuleConfiguredVisible,
  hudModuleDropInsertAfter,
  hudModuleKeyboardMove,
  hudModuleLabelKey,
  hudModulesInOrder,
  hudSurfaceTone,
  parseHudWidthDraft,
  type TechnicalPageState,
  visibleHudModules,
} from "./technical-view-model";

const snapshot = (sequence: string): TechnicalSnapshot => ({
  contractVersion: 5,
  sequence,
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 100,
  supportedLocales: ["en"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
  capture: {
    phase: "idle",
    messageKey: "No live capture task right now",
    messageArguments: [],
    issue: null,
  },
  hud: {
    version: 3,
    dataState: "empty",
    config: {
      width: 380,
      moduleOrder: ["summary", "characters"],
      showTitle: false,
      showTeamDps: true,
      showDuration: true,
      showTotalDamage: true,
      showCharacterRows: true,
      showDamageTaken: false,
      showAbyssHalf: false,
      showPassthroughState: false,
      showMiniTimeline: false,
    },
    summary: null,
    characters: [],
    status: {
      abyssDetected: false,
      abyssFloor: null,
      abyssHalf: null,
      abyssSuccess: false,
    },
    timeline: null,
  },
});

describe("technical HUD projection", () => {
  it("projects loading, empty and error states explicitly", () => {
    const loading: TechnicalPageState = { status: "loading" };
    const error: TechnicalPageState = {
      status: "error",
      error: {
        code: "bridge_error",
        messageKey: "Rust bridge unavailable",
        messageArguments: [],
      },
    };

    expect(loading.status).toBe("loading");
    expect(error.status).toBe("error");
  });

  it("ignores duplicate and out-of-order snapshots", () => {
    const current: TechnicalPageState = {
      status: "ready",
      snapshot: snapshot("12"),
    };

    expect(acceptSnapshot(current, snapshot("11"))).toBe(current);
    expect(acceptSnapshot(current, snapshot("12"))).toBe(current);
    expect(acceptSnapshot(current, snapshot("13"))).not.toBe(current);
  });

  it("maps unknown bridge enums to the visible unknown tone", () => {
    expect(bridgeTone("future_state")).toBe("unknown");
    expect(hudDataTone("future_state")).toBe("unknown");
  });

  it("maps capture lifecycle to one explicit control action and safe message", () => {
    expect(captureAction("idle")).toBe("start");
    expect(captureAction("running")).toBe("stop");
    expect(captureAction("starting")).toBe("pending");
    expect(captureAction("future_state")).toBe("pending");

    const current = snapshot("1");
    current.capture.issue = {
      code: "game_not_detected",
      messageKey: "Game not detected",
      messageArguments: [],
    };
    expect(captureMessage(current.capture)).toEqual({
      key: "Game not detected",
      arguments: [],
    });
  });

  it("keeps the blurred surface exclusive to interactive mode", () => {
    expect(hudSurfaceTone(false)).toBe("blurred");
    expect(hudSurfaceTone(true)).toBe("transparent");
  });

  it("projects Rust HUD configuration without recreating its defaults", () => {
    const current = snapshot("1");
    current.hud.dataState = "preview";
    current.hud.summary = {
      teamDps: 64301,
      durationSeconds: 34.9,
      totalDamage: 2246285,
      totalDamageTaken: 2834,
    };
    current.hud.characters = [
      {
        characterId: 1,
        name: "",
        previewLabelSuffix: "A",
        hits: "38",
        damage: 1227500,
        dps: 35172,
        damageSharePercent: 54.6,
        damageTaken: 0,
        color: "#A72648",
      },
    ];

    expect(visibleHudModules(current.hud)).toEqual(["summary", "characters"]);
    expect(hudCharacterName(current.hud.characters[0])).toBe("A");
    expect(formatHudNumber(current.hud.summary.totalDamage)).toBe("2,246,285");
    expect(formatHudDuration(current.hud.summary.durationSeconds)).toBe(
      "34.9s",
    );
  });

  it("shows the timeline module only when configuration and data agree", () => {
    const current = snapshot("1");
    current.hud.config.moduleOrder = ["timeline", "summary"];
    current.hud.config.showMiniTimeline = true;

    expect(visibleHudModules(current.hud)).toEqual(["summary"]);

    current.hud.timeline = {
      bucketSeconds: 1,
      durationSeconds: 2,
      peakDps: 200,
      buckets: [
        {
          startSeconds: 0,
          endSeconds: 1,
          damage: 100,
          dps: 100,
          hits: "1",
        },
        {
          startSeconds: 1,
          endSeconds: 2,
          damage: 200,
          dps: 200,
          hits: "2",
        },
      ],
    };

    expect(visibleHudModules(current.hud)).toEqual(["timeline", "summary"]);
  });

  it("projects module controls from Rust order without copying defaults", () => {
    const current = snapshot("1");
    current.hud.config.moduleOrder = [
      "timeline",
      "future_module",
      "summary",
      "timeline",
      "characters",
    ];
    current.hud.config.showMiniTimeline = false;
    current.hud.config.showTeamDps = false;
    current.hud.config.showDuration = false;
    current.hud.config.showTotalDamage = false;

    expect(hudModulesInOrder(current.hud.config)).toEqual([
      "timeline",
      "summary",
      "characters",
    ]);
    expect(hudModuleConfiguredVisible(current.hud.config, "timeline")).toBe(
      false,
    );
    expect(hudModuleConfiguredVisible(current.hud.config, "summary")).toBe(
      false,
    );
    expect(hudModuleLabelKey("characters")).toBe("Character Ranking");
  });

  it("maps pointer and keyboard reordering to the existing Rust move semantics", () => {
    const modules = [
      "title",
      "summary",
      "status",
      "characters",
      "timeline",
    ] as const;

    expect(hudModuleDropInsertAfter(19, 10, 20)).toBe(false);
    expect(hudModuleDropInsertAfter(20, 10, 20)).toBe(true);
    expect(hudModuleKeyboardMove([...modules], "status", "up")).toEqual({
      target: "summary",
      insertAfter: false,
    });
    expect(hudModuleKeyboardMove([...modules], "status", "down")).toEqual({
      target: "characters",
      insertAfter: true,
    });
    expect(hudModuleKeyboardMove([...modules], "title", "up")).toBeNull();
    expect(hudModuleKeyboardMove([...modules], "timeline", "down")).toBeNull();
  });

  it("accepts only integer HUD width drafts that fit the Tauri command", () => {
    expect(parseHudWidthDraft("512")).toBe(512);
    expect(parseHudWidthDraft("-1")).toBe(-1);
    expect(parseHudWidthDraft("512.5")).toBeNull();
    expect(parseHudWidthDraft("")).toBeNull();
    expect(parseHudWidthDraft("9007199254740991")).toBeNull();
  });
});
