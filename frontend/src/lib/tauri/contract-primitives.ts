export type ContractFailure = (message: string) => never;

const U64_MAX_DECIMAL = "18446744073709551615";
const SEMVER_PATTERN =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

function isU64Decimal(value: string): boolean {
  return (
    value.length < U64_MAX_DECIMAL.length ||
    (value.length === U64_MAX_DECIMAL.length && value <= U64_MAX_DECIMAL)
  );
}

export function isCanonicalSemver(value: string, maxLength = 128): boolean {
  if (value.length === 0 || value.length > maxLength) {
    return false;
  }
  const match = SEMVER_PATTERN.exec(value);
  if (match === null) {
    return false;
  }
  if (![match[1], match[2], match[3]].every(isU64Decimal)) {
    return false;
  }
  const prerelease = match[4];
  return (
    prerelease === undefined ||
    prerelease
      .split(".")
      .every(
        (identifier) => !/^\d+$/.test(identifier) || !/^0\d/.test(identifier),
      )
  );
}

export function createContractPrimitives(fail: ContractFailure) {
  const isRecord = (value: unknown): value is Record<string, unknown> =>
    typeof value === "object" && value !== null && !Array.isArray(value);

  const record = (value: unknown, field: string): Record<string, unknown> => {
    if (!isRecord(value)) {
      fail(`${field} must be an object`);
    }
    return value;
  };

  const array = (value: unknown, field: string): unknown[] => {
    if (!Array.isArray(value)) {
      fail(`${field} must be an array`);
    }
    return value;
  };

  const string = (value: unknown, field: string): string => {
    if (typeof value !== "string") {
      fail(`${field} must be a string`);
    }
    return value;
  };

  const boundedString = (
    value: unknown,
    field: string,
    maxLength: number,
  ): string => {
    const parsed = string(value, field);
    if (parsed.length === 0 || parsed.length > maxLength) {
      fail(`${field} must contain 1-${maxLength} characters`);
    }
    return parsed;
  };

  const boolean = (value: unknown, field: string): boolean => {
    if (typeof value !== "boolean") {
      fail(`${field} must be a boolean`);
    }
    return value;
  };

  const nonNegativeInteger = (value: unknown, field: string): number => {
    if (
      typeof value !== "number" ||
      !Number.isSafeInteger(value) ||
      value < 0
    ) {
      fail(`${field} must be a non-negative safe integer`);
    }
    return value;
  };

  const positiveInteger = (value: unknown, field: string): number => {
    const parsed = nonNegativeInteger(value, field);
    if (parsed === 0) {
      fail(`${field} must be positive`);
    }
    return parsed;
  };

  const exactFields = (
    value: Record<string, unknown>,
    field: string,
    allowed: readonly string[],
  ): void => {
    const allowedFields = new Set(allowed);
    const unexpected = Object.keys(value).find(
      (key) => !allowedFields.has(key),
    );
    if (unexpected !== undefined) {
      fail(`${field}.${unexpected} is not allowed for this status`);
    }
  };

  return {
    array,
    boolean,
    boundedString,
    exactFields,
    isRecord,
    nonNegativeInteger,
    positiveInteger,
    record,
    string,
  };
}
