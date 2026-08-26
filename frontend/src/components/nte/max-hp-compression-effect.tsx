import { Minimize2 } from "lucide-react";

import { cn } from "@/lib/utils";

export function MaxHpCompressionEffect({
  label,
  value,
  percentage,
  active = true,
  compact = false,
  className,
}: {
  label: string;
  value: string;
  percentage?: number;
  active?: boolean;
  compact?: boolean;
  className?: string;
}) {
  const boundedPercentage =
    percentage === undefined || !Number.isFinite(percentage)
      ? null
      : Math.max(0, Math.min(100, percentage));
  const percentageText =
    boundedPercentage === null
      ? null
      : `${boundedPercentage.toFixed(boundedPercentage < 0.1 ? 2 : 1)}%`;
  const accessibleText = `${label}: -${value}${percentageText === null ? "" : `, -${percentageText}`}`;

  return (
    <span
      className={cn("max-hp-compression-effect", className)}
      data-active={active}
      data-compact={compact}
      aria-label={accessibleText}
      title={accessibleText}
    >
      <Minimize2
        className="max-hp-compression-icon size-3 shrink-0"
        strokeWidth={2}
        aria-hidden="true"
      />
      {compact ? (
        <strong className="tabular-nums">-{percentageText ?? value}</strong>
      ) : (
        <>
          <span className="truncate">{label}</span>
          <strong className="tabular-nums">-{value}</strong>
          {percentageText !== null && (
            <span className="max-hp-compression-percent tabular-nums">
              -{percentageText}
            </span>
          )}
        </>
      )}
    </span>
  );
}
