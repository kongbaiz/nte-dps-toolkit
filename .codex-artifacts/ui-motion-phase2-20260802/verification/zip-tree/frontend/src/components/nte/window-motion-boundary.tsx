import { listen } from "@tauri-apps/api/event";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { MOTION_DURATION, waitForMotion } from "@/lib/motion";

import { WindowMotionContext } from "./window-motion-context";

const WINDOW_MOTION_ENTER_EVENT = "desktop-window-motion-enter";

export function WindowMotionBoundary({ children }: { children: ReactNode }) {
  const [phase, setPhase] = useState<"enter" | "idle" | "exit">("enter");
  const transitioning = useRef(false);

  const playEnter = useCallback(() => {
    setPhase("enter");
    window.setTimeout(() => setPhase("idle"), MOTION_DURATION.slow);
  }, []);

  useEffect(() => {
    playEnter();
    let unlisten: (() => void) | undefined;
    void listen(WINDOW_MOTION_ENTER_EVENT, playEnter).then((next) => {
      unlisten = next;
    });
    return () => unlisten?.();
  }, [playEnter]);

  const transitionToWindow = useCallback(
    async (action: () => Promise<unknown>) => {
      if (transitioning.current) return;
      transitioning.current = true;
      setPhase("exit");
      await waitForMotion(MOTION_DURATION.base);
      try {
        await action();
      } finally {
        transitioning.current = false;
        playEnter();
      }
    },
    [playEnter],
  );

  return (
    <WindowMotionContext.Provider value={{ transitionToWindow }}>
      <div className="window-motion-surface" data-window-motion={phase}>
        {children}
      </div>
    </WindowMotionContext.Provider>
  );
}
