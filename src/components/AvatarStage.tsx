import { useEffect, useRef, useState } from "react";
import { setAvatarOffset } from "../api";
import AnimatedPlaceholder from "./AnimatedPlaceholder";
import Live2DCanvas from "./Live2DCanvas";
import { modelClientRect } from "./Live2DCanvas/mountScene";

/**
 * Avatar stage: renders the Live2D canvas when a model URL is configured
 * (and the Cubism runtime is available), otherwise shows an animated SVG
 * placeholder. Doubles as the character move handle: a grab nudges the
 * character; the top bar keeps the explicit window move grip.
 *
 * Tracks window size so the avatar scales responsively when the user
 * resizes the Komorebi window.
 */
export default function AvatarStage({
  modelUrl,
  zoom = 1,
  offsetX = 0,
  offsetY = 0,
  overlayMode = false,
  onStageTap,
}: {
  modelUrl: string | null;
  zoom?: number;
  offsetX?: number;
  offsetY?: number;
  /** Character move handle in both modes; empty-stage grabs are
   * click-through (overlay) / no-ops (windowed). */
  overlayMode?: boolean;
  /** Overlay-mode character tap: open the message bar. */
  onStageTap?: () => void;
}) {
  const [size, setSize] = useState(() => ({
    w: window.innerWidth,
    h: window.innerHeight,
  }));
  // Live drag offset (fractions of the avatar box), committed on release.
  const [offset, setOffset] = useState({ x: offsetX, y: offsetY });

  useEffect(() => {
    const onResize = () =>
      setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // External offset changes (settings sliders, reset) flow back in.
  useEffect(() => {
    setOffset({ x: offsetX, y: offsetY });
  }, [offsetX, offsetY]);

  const dragRef = useRef<{
    x0: number;
    y0: number;
    t: number;
    ox: number;
    oy: number;
  } | null>(null);

  // ── The character is the move handle in BOTH modes (windowed included):
  // a grab on her nudges the character (avatar nudge, saved on release); a
  // grab on the empty stage does nothing (overlay: click-through, the
  // desktop keeps the click). The window itself is moved through the top-bar
  // move grip, which converts to a native window drag in windowed mode.
  const onStagePointerDown = (e: React.PointerEvent) => {
    const model = modelClientRect();
    const inside =
      !model ||
      (e.clientX >= model.left &&
        e.clientX <= model.left + model.width &&
        e.clientY >= model.top &&
        e.clientY <= model.top + model.height);
    if (!inside) {
      // Grabbed empty stage: click-through (overlay) / nothing to move
      // (windowed) — neither drags the window.
      return;
    }
    dragRef.current = {
      x0: e.clientX,
      y0: e.clientY,
      t: performance.now(),
      ox: offset.x,
      oy: offset.y,
    };
  };
  const onStagePointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const dx = e.clientX - d.x0;
    const dy = e.clientY - d.y0;
    const nx = d.ox + (W > 0 ? dx / (W / 2) : 0);
    const ny = d.oy + (H > 0 ? dy / (H / 2) : 0);
    setOffset((o) => (o.x === nx && o.y === ny ? o : { x: nx, y: ny }));
  };

  const onStagePointerUp = async (e: React.PointerEvent) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d) return;
    const dx = e.clientX - d.x0;
    const dy = e.clientY - d.y0;
    const dt = performance.now() - d.t;
    const isTap = Math.hypot(dx, dy) <= 6 && dt <= 350;
    const nx = d.ox + (W > 0 ? dx / (W / 2) : 0);
    const ny = d.oy + (H > 0 ? dy / (H / 2) : 0);
    if (isTap) {
      // Tap: the canvas' gated handler owns the reaction/input hand-off.
      return;
    }
    setOffset({ x: nx, y: ny });
    try {
      await setAvatarOffset(nx, ny);
    } catch {
      // Best effort: the live preview already applied the drag.
    }
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
      data-avatar-stage
      onPointerDown={onStagePointerDown}
      onPointerMove={onStagePointerMove}
      onPointerUp={onStagePointerUp}
      onPointerCancel={(e: React.PointerEvent) => {
        dragRef.current = null;
        void onStagePointerUp(e);
      }}
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
        cursor: "default",
      }}
    >
      {modelUrl ? (
        <Live2DCanvas
          modelUrl={modelUrl}
          width={W}
          height={H}
          zoom={zoom}
          offsetX={offset.x}
          offsetY={offset.y}
          overlay={overlayMode}
          onCharacterTap={() => onStageTap?.()}
        />
      ) : (
        <AnimatedPlaceholder />
      )}
    </div>
  );
}
