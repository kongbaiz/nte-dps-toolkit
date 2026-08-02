export function compareDecimalStrings(left: string, right: string): number {
  return left.length === right.length
    ? left.localeCompare(right)
    : left.length - right.length;
}

export function shouldAcceptDecimalVersion(
  accepted: string | null,
  incoming: string,
  allowEqual = false,
): boolean {
  if (accepted === null) return true;
  const comparison = compareDecimalStrings(incoming, accepted);
  return allowEqual ? comparison >= 0 : comparison > 0;
}
