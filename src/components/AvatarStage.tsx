import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import AnimatedPlaceholder from "./AnimatedPlaceholder";
import Live2DCanvas from "./Live2DCanvas";

/**
 * Avatar stage: renders the Live2D canvas when a model URL is configured
 * (and the Cubism runtime is available), otherwise shows an animated SVG
 * placeholder. Doubles as the window drag handle.
 *
 * Tracks window size so the avatar scales responsively when the user
 * resizes the Komorebi window.
 */
export default function AvatarStage({
  modelUrl,
  zoom = 1,
  offsetX = 0,
  offsetY = 0,
}: {
  modelUrl: string | null;
  zoom?: number;
  offsetX?: number;
  offsetY?: number;
}) {
  const [size, setSize] = useState(() => ({
    w: window.innerWidth,
    h: window.innerHeight,
  }));

  useEffect(() => {
    const onResize = () =>
      setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const lastDragReactRef = useRef(0);

  const startDrag = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    // Defer native window-drag until the pointer actually moves a few
    // pixels. Otherwise even a quick click is consumed by the OS drag
    // loop and the canvas's `pointerup` (tap-to-react / play special
    // motion) never fires.
    const startX = e.clientX;
    const startY = e.clientY;
    let dragStarted = false;
    const onMove = (ev: PointerEvent) => {
      if (dragStarted) return;
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 6) return;
      dragStarted = true;
      cleanup();
      void getCurrentWindow().startDragging();
      // Throttle drag reactions: at most one every 12 s so a long drag
      // session doesn't spam TTS.
      const now = performance.now();
      if (now - lastDragReactRef.current > 12_000) {
        lastDragReactRef.current = now;
        void invoke("react_event", { kind: "drag" }).catch(() => {});
      }
    };
    const onUp = () => cleanup();
    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
  };

  // Leave ~100 px of headroom for the chat bubble and top bar.
  const H = Math.max(260, size.h - 120);
  // Window width minus 12 px side gutters (no longer capped by the
  // height), so the horizontal nudge can reach the edge of a wide tiled
  // or full-screen window.
  const W = Math.max(220, size.w - 24);

  return (
    <div
      className="interactive"
      onPointerDown={startDrag}
      style={{
        position: "absolute",
        left: "50%",
        bottom: 0,
        transform: "translateX(-50%)",
        width: W,
        height: H,
        display: "flex",
        alignItems: "flex-end",
        justifyContent: "center",
        cursor: "grab",
      }}
    >
      {modelUrl ? (
        <Live2DCanvas
          modelUrl={modelUrl}
          width={W}
          height={H}
          zoom={zoom}
          offsetX={offsetX}
          offsetY={offsetY}
        />
      ) : (
        <AnimatedPlaceholder />
      )}
    </div>
  );
}
