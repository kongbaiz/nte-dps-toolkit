import { createContext, useContext } from "react";

export interface WindowMotionContextValue {
  transitionToWindow: (action: () => Promise<unknown>) => Promise<void>;
}

export const WindowMotionContext =
  createContext<WindowMotionContextValue | null>(null);

export function useWindowMotion() {
  const value = useContext(WindowMotionContext);
  if (value === null) {
    throw new Error("useWindowMotion must be used inside WindowMotionBoundary");
  }
  return value;
}
