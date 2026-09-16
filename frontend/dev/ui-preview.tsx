// Development-only entry; excluded from production Vite inputs.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { TIMELINE_UI_PREVIEW } from "@/features/timeline/timeline-ui-preview";
import { parseTimelineSnapshot } from "@/lib/tauri/timeline-contract";
import { parseSettingsSnapshot } from "@/lib/tauri/settings-contract";
import {
  settingsFixture,
  historyFixture,
  skillsFixture,
  packetsFixture,
  equipmentFixture,
  diagnosticsFixture,
  iniFixture,
} from "./fixtures";
import { parseHistorySnapshot } from "@/lib/tauri/history-contract";
import { parseSkillsSnapshot } from "@/lib/tauri/skills-contract";
import { parsePacketsSnapshot } from "@/lib/tauri/packets-contract";
import { parseEmptyCurtainSnapshot } from "@/lib/tauri/empty-curtain-contract";
import { parseDiagnosticsSnapshot } from "@/lib/tauri/diagnostics-contract";

const params = new URLSearchParams(location.search);
const settings = parseSettingsSnapshot(settingsFixture());
settings.interface.darkMode = params.get("theme") !== "light";
settings.interface.reduceMotion = params.get("motion") === "reduced";
if (params.get("accent") === "blue") settings.interface.accent = "blue";
if (params.get("preset") === "high-contrast")
  settings.interface.themePreset = "high-contrast";
if (params.get("preset") === "tactical")
  settings.interface.themePreset = "tactical";
if (params.get("density") === "compact") settings.interface.density = "compact";
if (params.get("density") === "comfortable")
  settings.interface.density = "comfortable";
if (params.get("language") === "en") settings.interface.language = "en";
const timeline = parseTimelineSnapshot({
  ...TIMELINE_UI_PREVIEW,
  contractVersion: 3,
  generation: "1",
  scope: "all",
  viewMode: "team",
  bucketSecondsMin: 0.2,
  bucketSecondsMax: 10,
  bucketSecondsStep: 0.1,
  hasData: params.get("state") !== "empty",
  omittedRoleDamage: 0,
  omittedRoleHits: "0",
  timeStopDuration: 5,
  compactedTimeStopIntervals: "0",
  segments: [{ start: 0, end: 48, dps: 118506 }],
});
const previewPackets = parsePacketsSnapshot({
  ...packetsFixture,
  packets: Array.from({ length: 8 }, (_, index) => ({
    ...packetsFixture.packets[0],
    sequence: String(index + 1),
    timestamp: packetsFixture.packets[0].timestamp + index * 0.125,
    parsedHits: index % 3 === 0 ? 1 : 0,
  })),
});
const previewSkills = parseSkillsSnapshot({
  ...skillsFixture,
  totalDamage: 5688311,
  totalHits: "96",
  characters: [
    { id: 7, name: "真红", color: "#e66a65", damage: 3132330, entries: 3 },
    { id: 8, name: "哈索尔", color: "#d4a54c", damage: 1734840, entries: 3 },
    { id: 9, name: "娜娜莉", color: "#9d86db", damage: 821141, entries: 3 },
  ],
  rows: [3132330, 1734840, 821141].flatMap((damage, index) =>
    [0.5, 0.3, 0.2].map((share, skill) => ({
      ...skillsFixture.rows[0],
      id: `preview-${index}-${skill}`,
      characterId: index + 7,
      characterName: ["真红", "哈索尔", "娜娜莉"][index],
      name: ["普通攻击", "战技", "终结技"][skill],
      hits: String([16, 12, 4][skill]),
      damage:
        skill === 2
          ? damage - Math.floor(damage * 0.5) - Math.floor(damage * 0.3)
          : Math.floor(damage * share),
    })),
  ),
});
let generation = 1;
mockWindows("console");
mockIPC(
  (command, args) => {
    const payload =
      args &&
      !(args instanceof ArrayBuffer) &&
      !ArrayBuffer.isView(args) &&
      !Array.isArray(args)
        ? args
        : {};
    if (command === "get_history_snapshot")
      return parseHistorySnapshot(historyFixture());
    if (command === "get_skills_snapshot")
      return parseSkillsSnapshot({ ...previewSkills, scope: payload.scope });
    if (command === "get_packets_snapshot") return previewPackets;
    if (command === "get_empty_curtain_snapshot")
      return parseEmptyCurtainSnapshot(equipmentFixture);
    if (command === "get_diagnostics_snapshot")
      return parseDiagnosticsSnapshot(diagnosticsFixture);
    if (command === "get_encrypted_ini_snapshot") return iniFixture;
    if (command === "get_character_data_snapshot")
      return {
        contractVersion: 1,
        generation: "1",
        attributes: ["灵", "咒"],
        records: [
          {
            id: 1003,
            nameZh: "早雾",
            nameEn: "Sagiri",
            codename: "Sagiri",
            attribute: "咒",
            verified: true,
            color: "#529f88",
            avatar: "",
          },
          {
            id: 1004,
            nameZh: "娜娜莉",
            nameEn: "Nanally",
            codename: "Nanally",
            attribute: "灵",
            verified: true,
            color: "#a78bfa",
            avatar: "",
          },
        ],
      };
    if (command === "get_plugin_panel")
      return {
        contractVersion: 1,
        mode: "packet_capture",
        connection: "notRequested",
        directorySelected: false,
        plugins: [],
        combat: null,
        operation: null,
      };
    if (
      /^subscribe_(skills|packets|empty_curtain|diagnostics|history)$/.test(
        command,
      )
    )
      return {
        subscriptionId: payload.subscriptionId,
        streamKind:
          command === "subscribe_empty_curtain"
            ? "emptyCurtain"
            : command.replace("subscribe_", ""),
        streamProtocolVersion: 1,
        streamIntervalMs: 100,
        streamGeneration: "1",
      };
    if (command.startsWith("unsubscribe_")) return null;
    if (command === "get_settings_snapshot")
      return parseSettingsSnapshot(settings);
    if (command === "set_settings_interface") {
      settings.interface = {
        ...settings.interface,
        ...Object(payload.settings),
      };
      settings.generation = String(++generation);
      return parseSettingsSnapshot(settings);
    }
    if (
      command === "get_timeline_snapshot" ||
      command === "set_timeline_preferences"
    ) {
      if (params.get("state") === "error")
        throw {
          code: "unavailable",
          messageKey: "Timeline data could not be loaded",
          messageArguments: [],
        };
      if (command === "set_timeline_preferences") {
        timeline.bucketSeconds = Number(payload.bucketSeconds);
        timeline.effectiveBucketSeconds = Number(payload.bucketSeconds);
        timeline.viewMode =
          payload.viewMode === "characters" ? "characters" : "team";
      }
      return parseTimelineSnapshot({
        ...timeline,
        scope: payload.scope,
        generation: String(++generation),
      });
    }
    if (command === "subscribe_settings" || command === "subscribe_timeline") {
      if (command === "subscribe_settings") {
        const channel = payload.onEvent;
        if (
          typeof channel !== "object" ||
          channel === null ||
          !("onmessage" in channel) ||
          typeof channel.onmessage !== "function"
        )
          throw new Error("QA settings channel is missing");
        channel.onmessage({
          streamProtocolVersion: 1,
          events: [
            { event: "snapshot", payload: parseSettingsSnapshot(settings) },
          ],
        });
      }
      return {
        subscriptionId: payload.subscriptionId,
        streamKind: command === "subscribe_settings" ? "settings" : "timeline",
        streamProtocolVersion: 1,
        streamIntervalMs: 100,
        streamGeneration: String(generation),
      };
    }
    if (
      command === "unsubscribe_settings" ||
      command === "unsubscribe_timeline"
    )
      return null;
    if (command === "plugin:window|is_always_on_top") return false;
    if (
      command === "plugin:window|set_background_color" ||
      command === "plugin:webview|set_webview_background_color"
    )
      return null;
    // Native operations intentionally fail closed in this offline preview.
    throw {
      code: "unavailable",
      messageKey: "Offline fixture: native operation is not available",
      messageArguments: [],
    };
  },
  { shouldMockEvents: true },
);

// Use the actual production shell, navigation, command palette and pages.
const { ConsolePage } = await import("@/features/console/console-page");
const { renderWindow } = await import("@/entries/window-bootstrap");
renderWindow(
  <div className="flex h-dvh flex-col">
    <div className="flex shrink-0 items-center justify-between gap-3 border-b bg-muted px-3 py-1 text-xs text-muted-foreground">
      <span>UI 预览 · 示例数据 · 原生操作不可用</span>
      <div className="flex gap-3">
        <a href="?theme=light">浅色</a>
        <a href="?theme=dark">深色</a>
        <a href="?theme=light&language=en">English</a>
      </div>
    </div>
    <div className="min-h-0 flex-1">
      <ConsolePage />
    </div>
  </div>,
  {
    windowRoute: "console",
    characterAvatars: false,
  },
);
