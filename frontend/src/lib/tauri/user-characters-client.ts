import { createContractPrimitives } from "./contract-primitives";
import { tauriInvokeTransport, type InvokeTransport } from "./stream-client";

export const USER_CHARACTERS_CONTRACT_VERSION = 1;
export interface CharacterSummary {
  uid: string;
  itemId: string | null;
  name: string | null;
  level: string | null;
}
export interface CharacterEntry {
  name: string | null;
  fields: { label: string; value: string | null }[];
}
export interface CharacterSection {
  title: string;
  available: boolean;
  entries: CharacterEntry[];
}
export interface CharacterDetail {
  summary: CharacterSummary;
  sections: CharacterSection[];
}
export interface CharacterPage {
  snapshotId: string;
  observedUnixUs: string;
  complete: boolean;
  total: number;
  offset: number;
  records: CharacterSummary[];
  detail: CharacterDetail | null;
}
export interface UserCharacters {
  contractVersion: 1;
  connectionIdentity: string;
  state:
    | "idle"
    | "queued"
    | "reading"
    | "completed"
    | "failed"
    | "canceled"
    | "stopped";
  dirty: boolean;
  sdkCompatible: boolean;
  page: CharacterPage | null;
}
export interface CharacterRequest {
  refresh: boolean;
  expectedIdentity: string | null;
  expectedSnapshotId: string | null;
  offset: number;
  selectedUid: string | null;
  query: string;
}
export const INITIAL_CHARACTER_REQUEST: CharacterRequest = {
  refresh: false,
  expectedIdentity: null,
  expectedSnapshotId: null,
  offset: 0,
  selectedUid: null,
  query: "",
};
const fail = (message: string): never => {
  throw new TypeError(message);
};
const p = createContractPrimitives(fail);
const text = (v: unknown) =>
  p.boundedString(v, "text", 512, { allowEmpty: true });
const nullable = (v: unknown) => (v === null ? null : text(v));
const list = (v: unknown, max: number) => {
  const a = p.array(v, "list");
  if (a.length > max) fail("list too large");
  return a;
};
const count = (v: unknown, max = 2048) => {
  const n = p.integer(v, "count");
  if (n < 0 || n > max) fail("invalid count");
  return n;
};
function summary(v: unknown): CharacterSummary {
  const r = p.record(v, "character");
  const uid = text(r.uid);
  if (!/^\d{1,10}:\d{1,10}$/.test(uid)) fail("invalid UID");
  return {
    uid,
    itemId: nullable(r.itemId),
    name: nullable(r.name),
    level: nullable(r.level),
  };
}
export function parseUserCharacters(v: unknown): UserCharacters {
  const r = p.record(v, "user characters");
  if (r.contractVersion !== USER_CHARACTERS_CONTRACT_VERSION)
    fail("unsupported user characters contract");
  const connectionIdentity = p.boundedString(
    r.connectionIdentity,
    "identity",
    64,
  );
  if (!/^\d+:\d+$/.test(connectionIdentity)) fail("invalid identity");
  const state = p.enumValue(
    r.state,
    [
      "idle",
      "queued",
      "reading",
      "completed",
      "failed",
      "canceled",
      "stopped",
    ] as const,
    "state",
  );
  const dirty = p.boolean(r.dirty, "dirty"),
    sdkCompatible = p.boolean(r.sdkCompatible, "sdkCompatible");
  let page: CharacterPage | null = null;
  if (r.page !== null) {
    if (state !== "completed" || dirty || !sdkCompatible)
      fail("unavailable snapshot contains data");
    const v = p.record(r.page, "page");
    const total = count(v.total),
      offset = count(v.offset);
    const records = list(v.records, 16).map(summary);
    if (
      offset > total ||
      offset % 16 !== 0 ||
      records.length !== Math.min(16, total - offset) ||
      new Set(records.map((r) => r.uid)).size !== records.length
    )
      fail("invalid pagination");
    let detail: CharacterDetail | null = null;
    if (v.detail !== null) {
      const d = p.record(v.detail, "detail");
      detail = {
        summary: summary(d.summary),
        sections: list(d.sections, 7).map((value) => {
          const s = p.record(value, "section");
          return {
            title: text(s.title),
            available: p.boolean(s.available, "available"),
            entries: list(s.entries, 256).map((value) => {
              const e = p.record(value, "entry");
              return {
                name: nullable(e.name),
                fields: list(e.fields, 140).map((value) => {
                  const f = p.record(value, "field");
                  return { label: text(f.label), value: nullable(f.value) };
                }),
              };
            }),
          };
        }),
      };
      if (detail.sections.length !== 7) fail("missing character sections");
    }
    if ((total === 0) !== (detail === null)) fail("missing selected character");
    page = {
      snapshotId: p.boundedString(v.snapshotId, "snapshotId", 128),
      observedUnixUs: p.decimalString(v.observedUnixUs, "observedUnixUs"),
      complete: p.boolean(v.complete, "complete"),
      total,
      offset,
      records,
      detail,
    };
  }
  return {
    contractVersion: 1,
    connectionIdentity,
    state,
    dirty,
    sdkCompatible,
    page,
  };
}
export function createUserCharactersClient(
  transport: InvokeTransport = tauriInvokeTransport,
) {
  return {
    async read(request: CharacterRequest) {
      return parseUserCharacters(
        await transport.invoke("get_user_characters", { request }),
      );
    },
  };
}
export const userCharactersClient = createUserCharactersClient();
