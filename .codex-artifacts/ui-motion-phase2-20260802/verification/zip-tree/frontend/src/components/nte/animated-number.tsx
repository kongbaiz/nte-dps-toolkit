import { useEffect, useRef, useState } from "react";

import {
  MOTION_DURATION,
  interpolateNumber,
  motionIsReduced,
} from "@/lib/motion";
import { cn } from "@/lib/utils";

interface AnimatedNumberProps {
  value: number;
  format?: (value: number) => string;
  className?: string;
  duration?: number;
}

export function AnimatedNumber({
  value,
  format = String,
  className,
  duration = MOTION_DURATION.slow,
}: AnimatedNumberProps) {
  const [displayed, setDisplayed] = useState(value);
  const displayedRef = useRef(value);

  useEffect(() => {
    const from = displayedRef.current;
    if (from === value || motionIsReduced()) {
      displayedRef.current = value;
      setDisplayed(value);
      return;
    }

    const startedAt = performance.now();
    let frame = 0;
    const advance = (now: number) => {
      const progress = Math.min(1, (now - startedAt) / duration);
      const next = interpolateNumber(from, value, progress);
      displayedRef.current = next;
      setDisplayed(next);
      if (progress < 1) frame = requestAnimationFrame(advance);
    };
    frame = requestAnimationFrame(advance);
    return () => cancelAnimationFrame(frame);
  }, [duration, value]);

  const exact = format(value);
  return (
    <span
      className={cn("motion-number tabular-nums", className)}
      aria-label={exact}
    >
      <span aria-hidden="true">{format(displayed)}</span>
    </span>
  );
}
