import { Separator as SeparatorPrimitive } from "@base-ui/react/separator";

import { cn } from "@/lib/utils";

function Separator({
  className,
  orientation = "horizontal",
  ...props
}: SeparatorPrimitive.Props) {
  return (
    <SeparatorPrimitive
      data-slot="separator"
      orientation={orientation}
      className={cn(
        "shrink-0 bg-border data-horizontal:h-(--ui-border-width) data-horizontal:w-full data-vertical:w-(--ui-border-width) data-vertical:self-stretch",
        className,
      )}
      {...props}
    />
  );
}

export { Separator };
