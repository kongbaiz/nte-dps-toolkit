const CANONICAL_DECIMAL_PATTERN = /^(0|[1-9]\d*)$/;

function assertCanonicalDecimal(value: string, field: string): void {
  if (!CANONICAL_DECIMAL_PATTERN.test(value)) {
    throw new TypeError(`${field} must be a canonical decimal string`);
  }
}

export function compareDecimalStrings(left: string, right: string): number {
  assertCanonicalDecimal(left, "left");
  assertCanonicalDecimal(right, "right");
  return left.length === right.length
    ? left.localeCompare(right)
    : left.length - right.length;
}

export function shouldAcceptDecimalVersion(
  accepted: string | null,
  incoming: string,
  allowEqual = false,
): boolean {
  assertCanonicalDecimal(incoming, "incoming");
  if (accepted === null) return true;
  assertCanonicalDecimal(accepted, "accepted");
  const comparison = compareDecimalStrings(incoming, accepted);
  return allowEqual ? comparison >= 0 : comparison > 0;
}
