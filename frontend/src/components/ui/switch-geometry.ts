export type SwitchSize = "sm" | "default";

type SwitchGeometry = {
  trackWidthPx: number;
  trackHeightPx: number;
  thumbDiameterPx: number;
  thumbTravelPx: number;
  rootClassName: string;
  thumbClassName: string;
};

export const SWITCH_GEOMETRY = {
  default: {
    trackWidthPx: 32,
    trackHeightPx: 18,
    thumbDiameterPx: 16,
    thumbTravelPx: 14,
    rootClassName: "h-[18px] w-[32px]",
    thumbClassName: "size-[16px] data-checked:translate-x-[14px]",
  },
  sm: {
    trackWidthPx: 24,
    trackHeightPx: 14,
    thumbDiameterPx: 12,
    thumbTravelPx: 10,
    rootClassName: "h-[14px] w-[24px]",
    thumbClassName: "size-[12px] data-checked:translate-x-[10px]",
  },
} as const satisfies Record<SwitchSize, SwitchGeometry>;
