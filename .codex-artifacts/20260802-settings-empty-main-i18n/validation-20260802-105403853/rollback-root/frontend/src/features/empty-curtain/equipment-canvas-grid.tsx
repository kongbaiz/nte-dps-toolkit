import { useEffect, useRef } from "react";

import { t, tf } from "@/lib/i18n";
import {
  type EmptyCurtainItem,
  type EquipmentQuality,
} from "@/lib/tauri/empty-curtain-contract";

import {
  equipmentCanvasBackingStore,
  equipmentCanvasCell,
  equipmentCanvasHeaderMetrics,
  equipmentCanvasIndexAtContent,
  equipmentCanvasItemTop,
  equipmentCanvasLayout,
  equipmentCanvasRenderWindow,
  equipmentCanvasRenderWindowNeedsRefresh,
  equipmentCanvasVisibleStatCounts,
  equipmentStatLockColumnX,
  equipmentStatLockPresentation,
  visibleEquipmentCanvasCells,
  type EquipmentCanvasCell,
  type EquipmentCanvasLayout,
  type EquipmentCanvasRenderWindow,
} from "./equipment-canvas-layout";

interface EquipmentCanvasGridProps {
  items: EmptyCurtainItem[];
  imageUrl: (item: EmptyCurtainItem) => string | null;
  characterAvatarUrl: (characterId: number) => string | null;
  onOpenItem: (item: EmptyCurtainItem) => void;
}

interface CanvasTheme {
  background: string;
  card: string;
  foreground: string;
  muted: string;
  mutedForeground: string;
  border: string;
  destructive: string;
}

const imageCache = new Map<string, HTMLImageElement>();

export function EquipmentCanvasGrid({
  items,
  imageUrl,
  characterAvatarUrl,
  onOpenItem,
}: EquipmentCanvasGridProps) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const spacerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const hoverOverlayRef = useRef<HTMLDivElement>(null);
  const layoutRef = useRef<EquipmentCanvasLayout | null>(null);
  const hoveredIndexRef = useRef<number | null>(null);
  const focusedIndexRef = useRef(0);
  const frameRef = useRef<number | null>(null);
  const drawRef = useRef<() => void>(() => undefined);
  const itemsRef = useRef(items);
  const imageUrlRef = useRef(imageUrl);
  const characterAvatarUrlRef = useRef(characterAvatarUrl);
  itemsRef.current = items;
  imageUrlRef.current = imageUrl;
  characterAvatarUrlRef.current = characterAvatarUrl;

  useEffect(() => {
    const scroller = scrollerRef.current;
    const canvas = canvasRef.current;
    if (!scroller || !canvas) return;

    let resizeTimer: number | null = null;
    let resizeBackingStore = true;
    let backingLogicalWidth = 0;
    let backingLogicalHeight = 0;
    let renderWindow: EquipmentCanvasRenderWindow | null = null;
    let theme = readCanvasTheme();

    const draw = () => {
      frameRef.current = null;
      const width = scroller.clientWidth;
      const height = scroller.clientHeight;
      if (width <= 0 || height <= 0) return;

      const currentItems = itemsRef.current;
      const layout = equipmentCanvasLayout(width, currentItems.length);
      layoutRef.current = layout;
      const nextRenderWindow = equipmentCanvasRenderWindow(
        layout,
        scroller.scrollTop,
        height,
      );
      renderWindow = nextRenderWindow;
      const spacer = spacerRef.current;
      if (spacer) spacer.style.height = `${layout.totalHeight}px`;

      canvas.style.top = `${nextRenderWindow.top}px`;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${nextRenderWindow.height}px`;
      const logicalSizeChanged =
        backingLogicalWidth !== width ||
        backingLogicalHeight !== nextRenderWindow.height;
      if (resizeBackingStore || (resizeTimer === null && logicalSizeChanged)) {
        const backingStore = equipmentCanvasBackingStore(
          width,
          nextRenderWindow.height,
          window.devicePixelRatio,
        );
        if (
          canvas.width !== backingStore.width ||
          canvas.height !== backingStore.height
        ) {
          canvas.width = backingStore.width;
          canvas.height = backingStore.height;
        }
        backingLogicalWidth = width;
        backingLogicalHeight = nextRenderWindow.height;
        resizeBackingStore = false;
      }

      const context = canvas.getContext("2d");
      if (!context) return;
      context.setTransform(
        canvas.width / width,
        0,
        0,
        canvas.height / nextRenderWindow.height,
        0,
        0,
      );
      context.clearRect(0, 0, width, nextRenderWindow.height);
      const cells = visibleEquipmentCanvasCells(
        layout,
        currentItems.length,
        nextRenderWindow.top,
        nextRenderWindow.height,
      );
      for (const cell of cells) {
        drawEquipmentCard(
          context,
          cell,
          nextRenderWindow.top,
          currentItems[cell.index],
          theme,
          cell.index === focusedIndexRef.current &&
            document.activeElement === scroller,
          imageUrlRef.current,
          characterAvatarUrlRef.current,
          scheduleDraw,
        );
      }
    };

    const scheduleDraw = () => {
      if (frameRef.current === null) {
        frameRef.current = window.requestAnimationFrame(draw);
      }
    };
    const scheduleResize = () => {
      hoveredIndexRef.current = null;
      if (hoverOverlayRef.current) {
        hoverOverlayRef.current.style.visibility = "hidden";
      }
      scheduleDraw();
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        resizeTimer = null;
        resizeBackingStore = true;
        scheduleDraw();
      }, 72);
    };
    const scheduleScroll = () => {
      const layout = layoutRef.current;
      if (hoveredIndexRef.current !== null) {
        hoveredIndexRef.current = null;
        if (hoverOverlayRef.current) {
          hoverOverlayRef.current.style.visibility = "hidden";
        }
      }
      if (
        !layout ||
        !renderWindow ||
        equipmentCanvasRenderWindowNeedsRefresh(
          renderWindow,
          layout.totalHeight,
          scroller.scrollTop,
          scroller.clientHeight,
        )
      ) {
        scheduleDraw();
      }
    };
    drawRef.current = scheduleDraw;
    const resizeObserver = new ResizeObserver(scheduleResize);
    resizeObserver.observe(scroller);
    const themeObserver = new MutationObserver(() => {
      theme = readCanvasTheme();
      scheduleDraw();
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    scroller.addEventListener("scroll", scheduleScroll, { passive: true });
    window.addEventListener("resize", scheduleResize, { passive: true });
    scheduleDraw();

    return () => {
      resizeObserver.disconnect();
      themeObserver.disconnect();
      scroller.removeEventListener("scroll", scheduleScroll);
      window.removeEventListener("resize", scheduleResize);
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    };
  }, []);

  useEffect(() => {
    hoveredIndexRef.current = null;
    if (hoverOverlayRef.current) {
      hoverOverlayRef.current.style.visibility = "hidden";
    }
    drawRef.current();
  }, [characterAvatarUrl, imageUrl, items]);

  const indexFromPointer = (
    target: EventTarget | null,
    x: number,
    y: number,
  ) => {
    const scroller = scrollerRef.current;
    const layout = layoutRef.current;
    if (!scroller || !layout) return null;
    if (target !== spacerRef.current) return null;
    return equipmentCanvasIndexAtContent(layout, items.length, x, y);
  };

  const updateHoverOverlay = (index: number | null) => {
    const overlay = hoverOverlayRef.current;
    const layout = layoutRef.current;
    if (!overlay || !layout || index === null) {
      if (overlay) overlay.style.visibility = "hidden";
      return;
    }
    const cell = equipmentCanvasCell(layout, index);
    overlay.style.width = `${cell.width}px`;
    overlay.style.height = `${cell.height}px`;
    overlay.style.transform = `translate3d(${cell.x}px, ${cell.y}px, 0)`;
    overlay.style.visibility = "visible";
  };

  const moveFocus = (next: number) => {
    const scroller = scrollerRef.current;
    const layout = layoutRef.current;
    if (!scroller || !layout || items.length === 0) return;
    focusedIndexRef.current = Math.max(0, Math.min(items.length - 1, next));
    scroller.setAttribute(
      "aria-label",
      tf("Equipment inventory, selected {}", [
        items[focusedIndexRef.current].name,
      ]),
    );
    const top = equipmentCanvasItemTop(layout, focusedIndexRef.current);
    const bottom = top + layout.cardHeight;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (bottom > scroller.scrollTop + scroller.clientHeight) {
      scroller.scrollTop = bottom - scroller.clientHeight;
    }
    drawRef.current();
  };

  const focusedItem = items[focusedIndexRef.current] ?? items[0];
  return (
    <div
      ref={scrollerRef}
      className="relative min-h-0 flex-1 overflow-auto rounded-xl border bg-muted/10 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      role="region"
      tabIndex={0}
      aria-label={
        focusedItem
          ? tf("Equipment inventory, selected {}", [focusedItem.name])
          : t("Equipment inventory")
      }
      onPointerMove={(event) => {
        const index = indexFromPointer(
          event.target,
          event.nativeEvent.offsetX,
          event.nativeEvent.offsetY,
        );
        if (index !== hoveredIndexRef.current) {
          hoveredIndexRef.current = index;
          updateHoverOverlay(index);
          event.currentTarget.style.cursor =
            index === null ? "default" : "pointer";
        }
      }}
      onPointerLeave={() => {
        hoveredIndexRef.current = null;
        updateHoverOverlay(null);
      }}
      onClick={(event) => {
        const index = indexFromPointer(
          event.target,
          event.nativeEvent.offsetX,
          event.nativeEvent.offsetY,
        );
        if (index === null) return;
        focusedIndexRef.current = index;
        event.currentTarget.setAttribute(
          "aria-label",
          tf("Equipment inventory, selected {}", [items[index].name]),
        );
        onOpenItem(items[index]);
      }}
      onFocus={() => drawRef.current()}
      onBlur={() => drawRef.current()}
      onKeyDown={(event) => {
        const layout = layoutRef.current;
        if (!layout || items.length === 0) return;
        const current = Math.min(focusedIndexRef.current, items.length - 1);
        const target =
          event.key === "ArrowLeft"
            ? current - 1
            : event.key === "ArrowRight"
              ? current + 1
              : event.key === "ArrowUp"
                ? current - layout.columns
                : event.key === "ArrowDown"
                  ? current + layout.columns
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? items.length - 1
                      : null;
        if (target !== null) {
          event.preventDefault();
          moveFocus(target);
        } else if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpenItem(items[current]);
        }
      }}
    >
      <div
        ref={spacerRef}
        className="pointer-events-auto w-full"
        data-equipment-canvas-spacer="true"
        aria-hidden="true"
      />
      <canvas
        ref={canvasRef}
        className="pointer-events-none absolute top-0 left-0"
        aria-hidden="true"
      />
      <div
        ref={hoverOverlayRef}
        className="pointer-events-none invisible absolute top-0 left-0 box-border rounded-[12px] border border-foreground/50 will-change-transform"
        aria-hidden="true"
      />
    </div>
  );
}

function drawEquipmentCard(
  context: CanvasRenderingContext2D,
  cell: EquipmentCanvasCell,
  scrollTop: number,
  item: EmptyCurtainItem,
  theme: CanvasTheme,
  focused: boolean,
  imageUrl: (item: EmptyCurtainItem) => string | null,
  characterAvatarUrl: (characterId: number) => string | null,
  requestDraw: () => void,
) {
  const x = cell.x;
  const y = cell.y - scrollTop;
  const width = cell.width;
  const height = cell.height;
  context.save();
  context.globalAlpha = item.discarded ? 0.58 : 1;
  roundedRect(context, x, y, width, height, 12);
  context.fillStyle = theme.card;
  context.fill();
  context.lineWidth = focused ? 2 : 1;
  context.strokeStyle = focused ? theme.foreground : theme.border;
  context.stroke();

  context.save();
  roundedRect(context, x, y, width, height, 12);
  context.clip();
  context.fillStyle = qualityColor(item.quality);
  context.fillRect(x, y, 4, height);
  context.fillStyle = theme.muted;
  context.fillRect(x + 4, y, width - 4, 68);
  context.restore();

  const iconX = x + 16;
  const iconY = y + 10;
  drawEquipmentImage(
    context,
    imageUrl(item),
    iconX,
    iconY,
    48,
    theme,
    requestDraw,
  );

  const header = equipmentCanvasHeaderMetrics(
    width,
    item.equippedCharacterId !== null,
  );
  if (item.equippedCharacterId !== null && header.avatarX !== null) {
    drawCharacterAvatar(
      context,
      characterAvatarUrl(item.equippedCharacterId),
      item.equippedCharacterId,
      x + header.avatarX,
      y + 16,
      36,
      theme,
      requestDraw,
    );
  }

  context.textBaseline = "middle";
  context.fillStyle = theme.foreground;
  context.font = "600 13px Geist Variable, sans-serif";
  drawClippedText(context, item.name, x + 74, y + 22, header.titleWidth);
  context.font = "11px ui-monospace, monospace";
  context.fillStyle = theme.mutedForeground;
  const level = item.maxLevel
    ? tf("Lv.{}/{}", [String(item.level), String(item.maxLevel)])
    : tf("Lv.{}", [String(item.level)]);
  drawClippedText(context, level, x + 74, y + 46, header.levelWidth);
  if (item.locked) drawLock(context, x + width - 24, y + 21, theme.foreground);
  if (item.discarded) {
    context.fillStyle = theme.destructive;
    context.font = "700 12px sans-serif";
    context.fillText("×", x + width - 26, y + 47);
  }

  let cursorY = y + 80;
  const mainStats = item.stats.filter((stat) => stat.main);
  const secondaryStats = item.stats.filter((stat) => !stat.main);
  const visibleStatCounts = equipmentCanvasVisibleStatCounts(
    mainStats.length,
    secondaryStats.length,
  );
  for (const stat of mainStats.slice(0, visibleStatCounts.main)) {
    drawStatLine(
      context,
      x + 16,
      cursorY,
      width - 32,
      stat.label,
      formatStat(stat.value, stat.percent),
      theme,
      true,
    );
    cursorY += 28;
  }
  if (visibleStatCounts.secondary > 0) {
    context.fillStyle = theme.mutedForeground;
    context.font = "600 10px Geist Variable, sans-serif";
    context.fillText(t("Secondary Stats").toUpperCase(), x + 16, cursorY + 4);
    cursorY += 19;
  }
  for (const stat of secondaryStats.slice(0, visibleStatCounts.secondary)) {
    const lockPresentation = equipmentStatLockPresentation(
      stat.unlocked,
      stat.unlockLevel,
    );
    const locked = lockPresentation.locked;
    if (locked) {
      roundedRect(context, x + 12, cursorY - 2, width - 24, 25, 7);
      context.fillStyle = theme.muted;
      context.fill();
      context.setLineDash([3, 3]);
      context.strokeStyle = theme.border;
      context.stroke();
      context.setLineDash([]);
    }
    drawStatLine(
      context,
      x + 18,
      cursorY + 10,
      width - 36,
      stat.label,
      formatStat(stat.value, stat.percent),
      theme,
      false,
      locked,
      lockPresentation.unlockLabel,
    );
    cursorY += 29;
  }
  context.restore();
}

function drawStatLine(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  label: string,
  value: string,
  theme: CanvasTheme,
  strong: boolean,
  locked = false,
  unlockLabel: string | null = null,
) {
  context.fillStyle = locked ? theme.mutedForeground : theme.foreground;
  context.font = `${strong ? "600" : "500"} 11px Geist Variable, sans-serif`;
  const rightReserve = locked ? 88 : 62;
  drawClippedText(context, label, x, y, width - rightReserve);
  context.textAlign = "right";
  context.font = `${strong ? "600" : "500"} 11px ui-monospace, monospace`;
  if (!locked) {
    context.fillText(value, x + width, y);
  } else {
    const unlockText = unlockLabel ?? t("Locked");
    const valueRightX = x + width;
    context.fillText(unlockText, valueRightX, y);
    drawLock(
      context,
      equipmentStatLockColumnX(valueRightX),
      y,
      theme.mutedForeground,
    );
  }
  context.textAlign = "left";
}

function drawEquipmentImage(
  context: CanvasRenderingContext2D,
  url: string | null,
  x: number,
  y: number,
  size: number,
  theme: CanvasTheme,
  requestDraw: () => void,
) {
  roundedRect(context, x, y, size, size, 8);
  context.fillStyle = theme.background;
  context.fill();
  if (!url) {
    context.fillStyle = theme.mutedForeground;
    context.font = "18px sans-serif";
    context.fillText("◇", x + 15, y + size / 2);
    return;
  }
  let image = imageCache.get(url);
  if (!image) {
    image = new Image();
    image.decoding = "async";
    image.src = url;
    image.addEventListener("load", requestDraw, { once: true });
    imageCache.set(url, image);
  }
  if (!image.complete || image.naturalWidth === 0) return;
  context.save();
  roundedRect(context, x, y, size, size, 8);
  context.clip();
  context.drawImage(image, x, y, size, size);
  context.restore();
}

function drawCharacterAvatar(
  context: CanvasRenderingContext2D,
  url: string | null,
  characterId: number,
  x: number,
  y: number,
  size: number,
  theme: CanvasTheme,
  requestDraw: () => void,
) {
  context.save();
  context.beginPath();
  context.arc(x + size / 2, y + size / 2, size / 2, 0, Math.PI * 2);
  context.fillStyle = theme.background;
  context.fill();
  context.clip();
  const image = url ? loadCanvasImage(url, requestDraw) : null;
  if (image?.complete && image.naturalWidth > 0) {
    context.drawImage(image, x, y, size, size);
  } else {
    context.fillStyle = theme.mutedForeground;
    context.font = "600 10px ui-monospace, monospace";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText(String(characterId), x + size / 2, y + size / 2);
    context.textAlign = "left";
  }
  context.restore();
  context.beginPath();
  context.arc(x + size / 2, y + size / 2, size / 2, 0, Math.PI * 2);
  context.lineWidth = 2;
  context.strokeStyle = theme.card;
  context.stroke();
}

function loadCanvasImage(url: string, requestDraw: () => void) {
  let image = imageCache.get(url);
  if (!image) {
    image = new Image();
    image.decoding = "async";
    image.src = url;
    image.addEventListener("load", requestDraw, { once: true });
    imageCache.set(url, image);
  }
  return image;
}

function drawLock(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  color: string,
) {
  context.save();
  context.strokeStyle = color;
  context.lineWidth = 1.4;
  context.strokeRect(x - 4, y - 1, 8, 7);
  context.beginPath();
  context.arc(x, y - 2, 3, Math.PI, 0);
  context.stroke();
  context.restore();
}

function drawClippedText(
  context: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  maxWidth: number,
) {
  if (context.measureText(text).width <= maxWidth) {
    context.fillText(text, x, y);
    return;
  }
  let end = text.length;
  while (
    end > 0 &&
    context.measureText(`${text.slice(0, end)}…`).width > maxWidth
  ) {
    end -= 1;
  }
  context.fillText(`${text.slice(0, end)}…`, x, y);
}

function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  context.beginPath();
  context.roundRect(x, y, width, height, radius);
}

function readCanvasTheme(): CanvasTheme {
  const styles = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string) =>
    styles.getPropertyValue(name).trim() || fallback;
  return {
    background: read("--background", "#ffffff"),
    card: read("--card", "#ffffff"),
    foreground: read("--foreground", "#17191c"),
    muted: read("--muted", "#f1f3f5"),
    mutedForeground: read("--muted-foreground", "#697078"),
    border: read("--border", "#d9dde1"),
    destructive: read("--destructive", "#dc2626"),
  };
}

function qualityColor(quality: EquipmentQuality): string {
  if (quality === "orange") return "#f59e0b";
  if (quality === "purple") return "#8b5cf6";
  if (quality === "blue") return "#0ea5e9";
  return "#8b949e";
}

function formatStat(value: number, percent: boolean): string {
  const scaled = value * (percent ? 100 : 1);
  const formatted = scaled
    .toFixed(2)
    .replace(/\.00$/, "")
    .replace(/(\.\d)0$/, "$1");
  return `+${formatted}${percent ? "%" : ""}`;
}
