import { describe, expect, it } from "vitest";

import semverConformance from "@res/contract-semver-conformance.json";

import {
  createContractPrimitives,
  isCanonicalSemver,
} from "./contract-primitives";

class TestContractError extends Error {}

const primitives = createContractPrimitives((message) => {
  throw new TestContractError(message);
});

const contractSources = import.meta.glob("./*-contract.ts", {
  eager: true,
  import: "default",
  query: "?raw",
}) as Record<string, string>;

describe("contract primitives", () => {
  it("matches the shared canonical SemVer corpus", () => {
    for (const vector of semverConformance) {
      expect(isCanonicalSemver(vector.value), vector.value).toBe(vector.valid);
    }
  });

  it("keeps type, bound, integer, and exact-field failures typed", () => {
    expect(() => primitives.record([], "payload")).toThrow(TestContractError);
    expect(() => primitives.array({}, "items")).toThrow(TestContractError);
    expect(() => primitives.boundedString("", "name", 8)).toThrow(
      TestContractError,
    );
    expect(
      primitives.boundedUtf8String(`${"界".repeat(42)}ab`, "name", 128),
    ).toHaveLength(44);
    expect(() =>
      primitives.boundedUtf8String("界".repeat(43), "name", 128),
    ).toThrow(TestContractError);
    expect(primitives.boundedUtf8StringAllowEmpty("", "name", 128)).toBe("");
    expect(
      primitives.nullableBoundedUtf8StringAllowEmpty(null, "name", 128),
    ).toBeNull();
    expect(() =>
      primitives.nullableBoundedUtf8StringAllowEmpty(
        "界".repeat(43),
        "name",
        128,
      ),
    ).toThrow(TestContractError);
    expect(() => primitives.nonNegativeInteger(-1, "count")).toThrow(
      TestContractError,
    );
    expect(() =>
      primitives.exactFields({ status: "ok", detail: "private" }, "state", [
        "status",
      ]),
    ).toThrow(TestContractError);
  });

  it("conforms across nullable, enum, decimal, numeric, and bounded-list primitives", () => {
    expect(primitives.nullableString(null, "value")).toBeNull();
    expect(primitives.optionalString(undefined, "value")).toBeNull();
    expect(primitives.enumValue("one", ["one", "two"] as const, "value")).toBe(
      "one",
    );
    expect(primitives.decimalString("9007199254740993", "value")).toBe(
      "9007199254740993",
    );
    expect(primitives.digitString("0007", "value", 4)).toBe("0007");
    expect(primitives.u64DecimalString("18446744073709551615", "value")).toBe(
      "18446744073709551615",
    );
    expect(primitives.unsigned32(0xffff_ffff, "value")).toBe(0xffff_ffff);
    expect(
      primitives.boundedMap(["a", "b"], "rows", 2, (value, field) =>
        primitives.string(value, field),
      ),
    ).toEqual(["a", "b"]);

    expect(() => primitives.decimalString("01", "value")).toThrow(
      TestContractError,
    );
    expect(() =>
      primitives.u64DecimalString("18446744073709551616", "value"),
    ).toThrow(TestContractError);
    expect(() => primitives.unsigned32(0x1_0000_0000, "value")).toThrow(
      TestContractError,
    );
    expect(() => primitives.boundedMap([1, 2, 3], "rows", 2, String)).toThrow(
      TestContractError,
    );
  });

  it("keeps every Tauri contract on the shared primitive factory", () => {
    const localPrimitivePattern =
      /^function (?:object|record|array|list|text|string|stringArray|flag|boolean|finite|finiteNumber|number|integer|nonNegativeInteger|positiveInteger|boundedText|boundedList|decimal|decimalString|enumValue|oneOf|nullable|optional|cssHex|color|uint|unsigned32|isRecord)(?:<|\()/m;
    for (const [path, source] of Object.entries(contractSources)) {
      expect(source, path).toContain("createContractPrimitives");
      expect(source, path).not.toMatch(localPrimitivePattern);
    }
  });
});
