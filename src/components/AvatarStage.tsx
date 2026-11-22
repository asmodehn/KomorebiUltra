import { useEffect, useRef, useState } from "react";
import {
  avatarCornerPending,
  markAvatarCornerDone,
  setAvatarOffset,
} from "../api";
import AnimatedPlaceholder from "./AnimatedPlaceholder";
import Live2DCanvas from "./Live2DCanvas";
import { modelClientRect } from "./Live2DCanvas/mountScene";

// The stored offset is a fraction of half the stage: ±1 puts her centre on the
// window edge, so half of her is clipped; ±2 puts it half a stage past the edge
// and draws her clear off the window side. Beyond ±2 there is nothing left to
// see, so the range stops there. Hanging out of the window, and out of sight,
// is the point of it, not a defect.
const clampOffset = (v: number) => Math.min(2, Math.max(-2, v));

/** Gap between her and the stage's right and bottom edges when she is
 * placed in the corner (the stage is inset from the window horizontally). */
const CORNER_MARGIN = 8;
// The revealed control strip sits below her, so the launch corner reserves
// its height as well.
const BOTTOM_RESERVE = 84;

/**
 * Avatar stage: renders the Live2D canvas when a model URL is configured
 * (and the Cubism runtime is available), otherwise shows an animated SVG
 * placeholder. A grab nudges the character; empty stage space is click-through.
 *
 * Tracks window size so the avatar scales responsively with the overlay.
 */
export default function AvatarStage({
  modelUrl,
  zoom = 1,
  offsetX = 0,
  offsetY = 0,
  placementReady = false,
  onPositionSaved,
  onStageTap,
}: {
  modelUrl: string | null;
  zoom?: number;
  offsetX?: number;
  offsetY?: number;
  /** True once the stored settings have loaded: the launch placement uses
   * the stored offset, so it must not run against the defaults. */
  placementReady?: boolean;
  /** Called after a placement was persisted, so the settings panel re-reads
   * the offset it now holds instead of showing the previous launch's. */
  onPositionSaved?: () => void;
  /** A clean character tap toggles the revealed controls. */
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

  const stageRef = useRef<HTMLDivElement>(null);
  const offsetRef = useRef(offset);
  useEffect(() => {
    offsetRef.current = offset;
  }, [offset]);

  // Until she can walk to where you are, every launch puts her back in the
  // lower-right corner: a place you can find her, rather than wherever the
  // last drag left her (possibly off the window, and forgotten).
  // The corner is measured from her drawn bounds, not assumed: the model is
  // not a fixed shape, and the offset is a fraction of half the stage, so the
  // shift that lands her in the corner depends on her size and the window's.
  // It runs once per launch, after the stage and the stored offset have
  // settled, and the launch state lives in the process: a page reload must
  // keep a drag. Measured once only: after a resize the 8 px gap is no longer
  // exact, and re-running it would fight a drag, which is fine — she may sit
  // anywhere.
  const cornered = useRef(false);
  useEffect(() => {
    if (!modelUrl || !placementReady) return;
    const stage = stageRef.current;
    if (!stage) return;
    let cancelled = false;
    const deadline = performance.now() + 30000;
    let raf = 0;
    const save = async (x: number, y: number) => {
      try {
        await setAvatarOffset(x, y);
      } catch {
        // Leave the launch pending: a later page of it retries the placement
        // and the save, where marking it done would keep her behind a store
        // that never got the value.
        return;
      }
      // Done only once the save landed: the page can be destroyed in between,
      // and a launch recorded as done without the stored value would leave
      // the next page showing her somewhere the store does not agree with.
      await markAvatarCornerDone().catch(() => {
        // Best effort: a later page places her again, which is the same place.
      });
      // Re-read the stored settings so the app does not keep the previous
      // launch's numbers — unless a drag landed while the save was in flight,
      // in which case those props are already behind.
      const now = offsetRef.current;
      if (now.x === x && now.y === y) onPositionSaved?.();
    };
    const place = () => {
      if (cancelled || cornered.current) return;
      // Out of time (a model that never draws): leave the launch pending, so a
      // later page places her rather than the position being lost for good.
      if (performance.now() > deadline) return;
      const model = stage.querySelector("[data-model-ready]")
        ? modelClientRect()
        : null;
      const box = stage.getBoundingClientRect();
      // The stage box is sized from a `resize` handler, and a relayout can
      // update the window before that handler runs: measuring now would corner
      // her against stale geometry and mark the launch done. Wait for the box
      // to match the window.
      const liveW = Math.max(220, window.innerWidth - 24);
      const liveH = Math.max(260, window.innerHeight - 120);
      const boxStale =
        Math.abs(box.width - liveW) > 1 || Math.abs(box.height - liveH) > 1;
      if (
        !model ||
        model.width < 1 ||
        box.width < 1 ||
        box.height < 1 ||
        boxStale
      ) {
        raf = requestAnimationFrame(place);
        return;
      }
      cornered.current = true;
      const bottomMargin = CORNER_MARGIN + BOTTOM_RESERVE;
      const { x, y } = offsetRef.current;
      const nx = clampOffset(
        x + (box.right - CORNER_MARGIN - model.right) / (box.width / 2),
      );
      const ny = clampOffset(
        y + (box.bottom - bottomMargin - model.bottom) / (box.height / 2),
      );
      setOffset({ x: nx, y: ny });
      void save(nx, ny);
    };
    // The launch state lives in the process, so a page reload cannot mark it
    // and the page that survives still places her.
    void avatarCornerPending()
      .catch(() => false)
      .then((pending) => {
        if (!cancelled && pending) raf = requestAnimationFrame(place);
      });
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [modelUrl, placementReady, onPositionSaved]);

  // ── The character is the move handle: a grab on her nudges the character
  // (avatar nudge, saved on release); a grab on the empty stage does nothing —
  // it is click-through, so the desktop keeps the click.
  const onStagePointerDown = (e: React.PointerEvent) => {
    const model = modelClientRect();
    const inside =
      !model ||
      (e.clientX >= model.left &&
        e.clientX <= model.left + model.width &&
        e.clientY >= model.top &&
        e.clientY <= model.top + model.height);
    if (!inside) {
      // Grabbed empty stage: click-through, the desktop keeps the click.
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
    const nx = clampOffset(d.ox + (W > 0 ? dx / (W / 2) : 0));
    const ny = clampOffset(d.oy + (H > 0 ? dy / (H / 2) : 0));
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
    const nx = clampOffset(d.ox + (W > 0 ? dx / (W / 2) : 0));
    const ny = clampOffset(d.oy + (H > 0 ? dy / (H / 2) : 0));
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
      ref={stageRef}
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
          onCharacterTap={() => onStageTap?.()}
        />
      ) : (
        <AnimatedPlaceholder />
      )}
    </div>
  );
}
