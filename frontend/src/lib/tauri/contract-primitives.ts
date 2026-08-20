export type ContractFailure = (message: string) => never;

export interface BoundedStringOptions {
  allowEmpty?: boolean;
}

export interface DecimalStringOptions {
  canonical?: boolean;
  maxLength?: number;
  maxValue?: string;
  positive?: boolean;
}

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

export function createContractPrimitives(
  fail: ContractFailure,
) {
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
    options: BoundedStringOptions = {},
  ): string => {
    const parsed = string(value, field);
    if (
      (!options.allowEmpty && parsed.length === 0) ||
      parsed.length > maxLength
    ) {
      fail(
        options.allowEmpty
          ? `${field} must contain at most ${maxLength} characters`
          : `${field} must contain 1-${maxLength} characters`,
      );
    }
    return parsed;
  };

  const boundedStringAllowEmpty = (
    value: unknown,
    field: string,
    maxLength: number,
  ): string => boundedString(value, field, maxLength, { allowEmpty: true });

  const boundedUtf8String = (
    value: unknown,
    field: string,
    maxBytes: number,
    options: BoundedStringOptions = {},
  ): string => {
    const parsed = string(value, field);
    if (
      (!options.allowEmpty && parsed.length === 0) ||
      new TextEncoder().encode(parsed).byteLength > maxBytes
    ) {
      fail(
        options.allowEmpty
          ? `${field} must contain at most ${maxBytes} UTF-8 bytes`
          : `${field} must contain 1-${maxBytes} UTF-8 bytes`,
      );
    }
    return parsed;
  };

  const boundedUtf8StringAllowEmpty = (
    value: unknown,
    field: string,
    maxBytes: number,
  ): string => boundedUtf8String(value, field, maxBytes, { allowEmpty: true });

  const nonEmptyString = (value: unknown, field: string): string => {
    const parsed = string(value, field);
    if (parsed.length === 0) {
      fail(`${field} must not be empty`);
    }
    return parsed;
  };

  const boolean = (value: unknown, field: string): boolean => {
    if (typeof value !== "boolean") {
      fail(`${field} must be a boolean`);
    }
    return value;
  };

  const finiteNumber = (value: unknown, field: string): number => {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      fail(`${field} must be a finite number`);
    }
    return value;
  };

  const integer = (value: unknown, field: string): number => {
    const parsed = finiteNumber(value, field);
    if (!Number.isSafeInteger(parsed)) {
      fail(`${field} must be a safe integer`);
    }
    return parsed;
  };

  const nonNegativeNumber = (value: unknown, field: string): number => {
    if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
      fail(`${field} must be a finite non-negative number`);
    }
    return value;
  };

  const positiveNumber = (value: unknown, field: string): number => {
    const parsed = finiteNumber(value, field);
    if (parsed <= 0) {
      fail(`${field} must be positive`);
    }
    return parsed;
  };

  const nonNegativeInteger = (value: unknown, field: string): number => {
    const parsed = integer(value, field);
    if (parsed < 0) {
      fail(`${field} must be a non-negative safe integer`);
    }
    return parsed;
  };

  const positiveInteger = (value: unknown, field: string): number => {
    const parsed = integer(value, field);
    if (parsed <= 0) {
      fail(`${field} must be positive`);
    }
    return parsed;
  };

  const boundedInteger = (
    value: unknown,
    field: string,
    minimum: number,
    maximum: number,
  ): number => {
    const parsed = integer(value, field);
    if (parsed < minimum || parsed > maximum) {
      fail(`${field} must be between ${minimum} and ${maximum}`);
    }
    return parsed;
  };

  const unsigned32 = (value: unknown, field: string): number =>
    boundedInteger(value, field, 0, 0xffff_ffff);

  const boundedArray = (
    value: unknown,
    field: string,
    maximum: number,
  ): unknown[] => {
    const parsed = array(value, field);
    if (parsed.length > maximum) {
      fail(`${field} exceeds the contract limit of ${maximum}`);
    }
    return parsed;
  };

  const boundedMap = <T>(
    value: unknown,
    field: string,
    maximum: number,
    parser: (value: unknown, field: string) => T,
  ): T[] =>
    boundedArray(value, field, maximum).map((item, index) =>
      parser(item, `${field}[${index}]`),
    );

  const stringArray = (value: unknown, field: string): string[] =>
    array(value, field).map((item, index) =>
      string(item, `${field}[${index}]`),
    );

  const enumValue = <const T extends readonly string[]>(
    value: unknown,
    allowed: T,
    field: string,
  ): T[number] => {
    const parsed = string(value, field);
    if (!allowed.includes(parsed)) {
      fail(`${field} has an unsupported value`);
    }
    return parsed as T[number];
  };

  const nullable = <T>(
    value: unknown,
    parser: (value: unknown) => T,
  ): T | null => (value === null ? null : parser(value));

  const optional = <T>(
    value: unknown,
    parser: (value: unknown) => T,
  ): T | null => (value === null || value === undefined ? null : parser(value));

  const nullableString = (value: unknown, field: string): string | null =>
    nullable(value, (candidate) => string(candidate, field));

  const optionalString = (value: unknown, field: string): string | null =>
    optional(value, (candidate) => string(candidate, field));

  const nullableBoundedString = (
    value: unknown,
    field: string,
    maxLength: number,
  ): string | null =>
    nullable(value, (candidate) => boundedString(candidate, field, maxLength));

  const nullableBoundedStringAllowEmpty = (
    value: unknown,
    field: string,
    maxLength: number,
  ): string | null =>
    nullable(value, (candidate) =>
      boundedStringAllowEmpty(candidate, field, maxLength),
    );

  const nullableBoundedUtf8StringAllowEmpty = (
    value: unknown,
    field: string,
    maxBytes: number,
  ): string | null =>
    nullable(value, (candidate) =>
      boundedUtf8StringAllowEmpty(candidate, field, maxBytes),
    );

  const nullableInteger = (value: unknown, field: string): number | null =>
    nullable(value, (candidate) => integer(candidate, field));

  const nullableNonNegativeInteger = (
    value: unknown,
    field: string,
  ): number | null =>
    nullable(value, (candidate) => nonNegativeInteger(candidate, field));

  const optionalNonNegativeInteger = (
    value: unknown,
    field: string,
  ): number | null =>
    optional(value, (candidate) => nonNegativeInteger(candidate, field));

  const nullableNumber = (value: unknown, field: string): number | null =>
    nullable(value, (candidate) => finiteNumber(candidate, field));

  const nullableEnumValue = <const T extends readonly string[]>(
    value: unknown,
    allowed: T,
    field: string,
  ): T[number] | null =>
    nullable(value, (candidate) => enumValue(candidate, allowed, field));

  const decimalString = (
    value: unknown,
    field: string,
    options: DecimalStringOptions = {},
  ): string => {
    if (typeof value !== "string") {
      fail(`${field} must be a valid decimal string`);
    }
    const parsed = value;
    const pattern = options.canonical === false ? /^\d+$/ : /^(0|[1-9]\d*)$/;
    if (
      !pattern.test(parsed) ||
      (options.maxLength !== undefined && parsed.length > options.maxLength) ||
      (options.maxValue !== undefined &&
        (parsed.length > options.maxValue.length ||
          (parsed.length === options.maxValue.length &&
            parsed > options.maxValue))) ||
      (options.positive && parsed === "0")
    ) {
      fail(`${field} must be a valid decimal string`);
    }
    return parsed;
  };

  const digitString = (
    value: unknown,
    field: string,
    maxLength?: number,
  ): string => decimalString(value, field, { canonical: false, maxLength });

  const decimalString32 = (value: unknown, field: string): string =>
    digitString(value, field, 32);

  const canonicalDecimalString128 = (value: unknown, field: string): string =>
    decimalString(value, field, { maxLength: 128 });

  const u64DecimalString = (
    value: unknown,
    field: string,
    positive = false,
  ): string => {
    if (
      typeof value !== "string" ||
      !/^(0|[1-9]\d*)$/.test(value) ||
      value.length > U64_MAX_DECIMAL.length ||
      (value.length === U64_MAX_DECIMAL.length && value > U64_MAX_DECIMAL)
    ) {
      fail(`${field} must be a u64 decimal string`);
    }
    if (positive && value === "0") {
      fail(`${field} must be positive`);
    }
    return value;
  };

  const positiveU64DecimalString = (value: unknown, field: string): string =>
    u64DecimalString(value, field, true);

  const cssHex = (value: unknown, field: string): string => {
    const parsed = string(value, field);
    if (!/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(parsed)) {
      fail(`${field} must be a CSS hex color`);
    }
    return parsed;
  };

  const cssHex6OrEmpty = (value: unknown, field: string): string => {
    const parsed = string(value, field);
    if (parsed !== "" && !/^#[0-9a-fA-F]{6}$/.test(parsed)) {
      fail(`${field} must be #RRGGBB or empty`);
    }
    return parsed;
  };

  const nullableUnsigned32 = (value: unknown, field: string): number | null =>
    nullable(value, (candidate) => unsigned32(candidate, field));

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
    boundedArray,
    boundedInteger,
    boundedMap,
    boundedString,
    boundedStringAllowEmpty,
    boundedUtf8String,
    boundedUtf8StringAllowEmpty,
    canonicalDecimalString128,
    cssHex,
    cssHex6OrEmpty,
    decimalString,
    decimalString32,
    digitString,
    enumValue,
    exactFields,
    finiteNumber,
    integer,
    isRecord,
    nonNegativeNumber,
    nonNegativeInteger,
    nonEmptyString,
    nullable,
    nullableBoundedString,
    nullableBoundedStringAllowEmpty,
    nullableBoundedUtf8StringAllowEmpty,
    nullableEnumValue,
    nullableInteger,
    nullableNonNegativeInteger,
    nullableNumber,
    nullableString,
    nullableUnsigned32,
    optional,
    optionalNonNegativeInteger,
    optionalString,
    positiveNumber,
    positiveU64DecimalString,
    positiveInteger,
    record,
    string,
    stringArray,
    u64DecimalString,
    unsigned32,
  };
}
