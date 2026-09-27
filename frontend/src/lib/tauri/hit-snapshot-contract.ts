import { TechnicalContractError } from "./technical-contract";
import { createContractPrimitives } from "./contract-primitives";
const p = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});
export interface EffectCounts {
  positive: number;
  negative: number;
  other: number;
  complete: boolean | null;
}
export function parseEffectCounts(v: unknown): EffectCounts | null {
  if (v === null) return null;
  const r = p.record(v, "effect counts");
  const result = {
    positive: p.nonNegativeInteger(r.positive, "positive"),
    negative: p.nonNegativeInteger(r.negative, "negative"),
    other: p.nonNegativeInteger(r.other, "other"),
    complete: r.complete === null ? null : p.boolean(r.complete, "complete"),
  };
  if (result.positive + result.negative + result.other > 512)
    throw new TechnicalContractError("effect count budget exceeded");
  return result;
}
export interface AttributeDatum {
  value: number | boolean | null;
  status: string;
  source: string;
}
export interface AttributeSnapshot {
  id: string | null;
  generation: string | null;
  sampledUnixUs: string | null;
  finishedUnixUs: string | null;
  actorIndex: number;
  actorSerial: number;
  actorName: string | null;
  stage: string | null;
  status: string | null;
  values: Record<string, AttributeDatum> | null;
}
export interface NativeEffect {
  instanceKey: string;
  key: string;
  name: string;
  description: string;
  source: string;
  kind: number;
  stacks: number;
  duration: number;
  startWorldTime: number;
  level: number;
  inhibited: boolean;
}
export interface EffectSnapshot {
  id: string | null;
  actorIndex: number;
  actorSerial: number;
  observedUs: string | null;
  complete: boolean | null;
  status: string | null;
  effects: NativeEffect[] | null;
}
export interface HitSnapshotData {
  attackerAttributes: AttributeSnapshot | null;
  victimAttributes: AttributeSnapshot | null;
  attackerEffects: EffectSnapshot | null;
  victimEffects: EffectSnapshot | null;
}
export interface HitSnapshot {
  key: string;
  critical: boolean | null;
  criticalSource: string | null;
  retention: "available" | "missing" | "budget_exceeded";
  roleEffects: EffectCounts | null;
  enemyEffects: EffectCounts | null;
  data: HitSnapshotData | null;
}
const text = (v: unknown, field: string, limit = 512) =>
  p.boundedUtf8StringAllowEmpty(v, field, limit);
const nullableText = (v: unknown, field: string, limit = 512) =>
  v === null ? null : text(v, field, limit);
const id = (v: unknown, field: string) => {
  if (v === null) return null;
  const result = text(v, field, 20);
  if (!/^\d{1,20}$/.test(result) || BigInt(result) > 18446744073709551615n)
    throw new TechnicalContractError("invalid snapshot identity");
  return result;
};
function attributes(v: unknown): AttributeSnapshot | null {
  if (v === null) return null;
  const r = p.record(v, "attributes");
  let values: Record<string, AttributeDatum> | null = null;
  if (r.values !== null) {
    const entries = Object.entries(p.record(r.values, "attribute values"));
    if (entries.length > 64)
      throw new TechnicalContractError("attribute budget exceeded");
    values = Object.fromEntries(
      entries.map(([key, raw]) => {
        text(key, "attribute key", 64);
        const value = p.record(raw, "attribute");
        return [
          key,
          {
            value:
              value.value === null
                ? null
                : typeof value.value === "boolean"
                  ? value.value
                  : p.finiteNumber(value.value, "attribute.value"),
            status: text(value.status, "attribute.status", 128),
            source: text(value.source, "attribute.source", 256),
          },
        ];
      }),
    );
  }
  return {
    id: id(r.id, "id"),
    generation: id(r.generation, "generation"),
    sampledUnixUs: id(r.sampledUnixUs, "sampledUnixUs"),
    finishedUnixUs: id(r.finishedUnixUs, "finishedUnixUs"),
    actorIndex: p.integer(r.actorIndex, "actorIndex"),
    actorSerial: p.integer(r.actorSerial, "actorSerial"),
    actorName: nullableText(r.actorName, "actorName"),
    stage: nullableText(r.stage, "stage"),
    status: nullableText(r.status, "status"),
    values,
  };
}
function effects(v: unknown): EffectSnapshot | null {
  if (v === null) return null;
  const r = p.record(v, "effects");
  return {
    id: id(r.id, "id"),
    actorIndex: p.integer(r.actorIndex, "actorIndex"),
    actorSerial: p.integer(r.actorSerial, "actorSerial"),
    observedUs: id(r.observedUs, "observedUs"),
    complete: r.complete === null ? null : p.boolean(r.complete, "complete"),
    status: nullableText(r.status, "status", 128),
    effects:
      r.effects === null
        ? null
        : p.boundedArray(r.effects, "effects", 512).map((raw) => {
            const e = p.record(raw, "effect");
            const kind = p.integer(e.kind, "kind");
            if (kind < 0 || kind > 255)
              throw new TechnicalContractError("invalid effect kind");
            return {
              instanceKey: text(e.instanceKey, "instanceKey"),
              key: text(e.key, "key"),
              name: text(e.name, "name"),
              description: text(e.description, "description", 4096),
              source: text(e.source, "source"),
              kind,
              stacks: p.integer(e.stacks, "stacks"),
              duration: p.finiteNumber(e.duration, "duration"),
              startWorldTime: p.finiteNumber(
                e.startWorldTime,
                "startWorldTime",
              ),
              level: p.finiteNumber(e.level, "level"),
              inhibited: p.boolean(e.inhibited, "inhibited"),
            };
          }),
  };
}
export function parseHitSnapshotReply(
  v: unknown,
  hitId: string,
  key: string,
): HitSnapshot {
  const r = p.record(v, "hit snapshot reply");
  if (r.contractVersion !== 1 || r.hitId !== hitId)
    throw new TechnicalContractError("hit snapshot reply identity mismatch");
  const s = p.record(r.snapshot, "hit snapshot");
  if (r.snapshotKey !== key)
    throw new TechnicalContractError("hit snapshot key mismatch");
  text(s.key, "key", 256);
  const retention = p.enumValue(
    s.retention,
    ["available", "missing", "budget_exceeded"] as const,
    "retention",
  );
  if ((retention === "available") !== (s.data !== null))
    throw new TechnicalContractError("inconsistent snapshot retention");
  let data: HitSnapshotData | null = null;
  if (s.data !== null) {
    const d = p.record(s.data, "data");
    data = {
      attackerAttributes: attributes(d.attackerAttributes),
      victimAttributes: attributes(d.victimAttributes),
      attackerEffects: effects(d.attackerEffects),
      victimEffects: effects(d.victimEffects),
    };
  }
  const parsed = {
    key: text(s.key, "key", 256),
    critical: s.critical === null ? null : p.boolean(s.critical, "critical"),
    criticalSource: nullableText(s.criticalSource, "criticalSource", 256),
    roleEffects: parseEffectCounts(s.roleEffects),
    enemyEffects: parseEffectCounts(s.enemyEffects),
    retention,
    data,
  };
  if (new TextEncoder().encode(JSON.stringify(parsed)).length > 1024 * 1024)
    throw new TechnicalContractError("snapshot byte budget exceeded");
  return parsed;
}
