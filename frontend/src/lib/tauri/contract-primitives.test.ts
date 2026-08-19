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
    expect(() => primitives.nonNegativeInteger(-1, "count")).toThrow(
      TestContractError,
    );
    expect(() =>
      primitives.exactFields({ status: "ok", detail: "private" }, "state", [
        "status",
      ]),
    ).toThrow(TestContractError);
  });
});
