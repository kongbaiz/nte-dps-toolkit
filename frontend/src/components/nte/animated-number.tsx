import {
  animate,
  motion,
  useMotionValue,
  useReducedMotionConfig,
  useTransform,
} from "motion/react";
import { useEffect } from "react";

import { MOTION_DURATION } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface AnimatedNumberProps {
  value: number;
  format?: (value: number) => string;
  className?: string;
}

export function AnimatedNumber({
  value,
  format = String,
  className,
}: AnimatedNumberProps) {
  const displayed = useMotionValue(value);
  const text = useTransform(displayed, format);
  const reduceMotion = useReducedMotionConfig();

  useEffect(() => {
    if (reduceMotion) {
      displayed.set(value);
      return;
    }
    const controls = animate(displayed, value, {
      duration: MOTION_DURATION.slow / 1000,
      ease: "easeOut",
    });
    return () => controls.stop();
  }, [displayed, reduceMotion, value]);

  return (
    <span
      className={cn("motion-number tabular-nums", className)}
      aria-label={format(value)}
    >
      <motion.span aria-hidden="true">{text}</motion.span>
    </span>
  );
}
