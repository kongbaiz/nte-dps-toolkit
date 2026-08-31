import { Switch as SwitchPrimitive } from "@base-ui/react/switch";

import { cn } from "@/lib/utils";

type SwitchSize = "sm" | "default";

const switchClasses = {
  default: ["h-[18px] w-[32px]", "size-[16px] data-checked:translate-x-[14px]"],
  sm: ["h-[14px] w-[24px]", "size-[12px] data-checked:translate-x-[10px]"],
} as const satisfies Record<SwitchSize, readonly [string, string]>;

function Switch({
  className,
  size = "default",
  ...props
}: SwitchPrimitive.Root.Props & {
  size?: SwitchSize;
}) {
  const [rootClassName, thumbClassName] = switchClasses[size];

  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        "peer group/switch relative inline-flex shrink-0 items-center rounded-full border border-transparent transition-[background-color,border-color,box-shadow,opacity,transform] duration-200 ease-out outline-none after:absolute after:-inset-x-3 after:-inset-y-2 active:scale-[0.97] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 data-checked:bg-primary data-unchecked:bg-input dark:data-unchecked:bg-input/80 data-disabled:cursor-not-allowed data-disabled:opacity-50",
        rootClassName,
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          "pointer-events-none block translate-x-0 rounded-full bg-background ring-0 transition-transform duration-200 [transition-timing-function:var(--motion-ease-emphasized)] dark:data-checked:bg-primary-foreground dark:data-unchecked:bg-foreground",
          thumbClassName,
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
