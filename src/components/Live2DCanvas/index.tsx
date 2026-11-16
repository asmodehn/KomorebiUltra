import { useEffect, useRef } from "react";
import { mountLive2DScene, type ModelInfo } from "./mountScene";

/**
 * Mounts a PIXI canvas and loads a Live2D Cubism 2 / 3 / 4 / 5 model
 * from `modelUrl`.
 *
 * Graceful failure modes:
 *  - Cubism core script missing → nothing is mounted (the stage stays empty).
 *  - Model fetch/parse error → retried with a short backoff, then a warning.
 *  - PIXI / pixi-live2d-display import error → nothing is mounted.
 */
export default function Live2DCanvas({
  modelUrl,
  width,
  height,
  zoom = 1,
  offsetX = 0,
  offsetY = 0,
  overlay = false,
  onCharacterTap,
}: {
  modelUrl: string;
  width: number;
  height: number;
  zoom?: number;
  offsetX?: number;
  offsetY?: number;
  /** Overlay mode: taps on the character open the message bar instead of
   * firing a random reaction. */
  overlay?: boolean;
  onCharacterTap?: () => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Live layout refs — let the user tweak avatar zoom/position without
  // tearing down the whole PIXI app + Live2D model.
  const modelRef = useRef<ModelInfo | null>(null);
  const zoomRef = useRef(zoom);
  const offsetXRef = useRef(offsetX);
  const offsetYRef = useRef(offsetY);
  // Overlay-mode tap hand-off. Kept in a ref so the mount effect does not
  // re-run (and re-load the model) on every render.
  const onCharacterTapRef = useRef(onCharacterTap);
  useEffect(() => {
    onCharacterTapRef.current = onCharacterTap;
  });

  // Re-apply scale + position from current refs. Re-used on first load
  // and whenever the user tweaks zoom/offset sliders in Settings.
  const applyLayout = () => {
    const ref = modelRef.current;
    if (!ref) return;
    const { model, anchored, canvasW, canvasH } = ref;
    const fit = Math.min(canvasW / model.width, canvasH / model.height) * 0.9;
    const scale = fit * Math.max(0.1, zoomRef.current);
    // Only write the transform when it actually changed. During a character
    // drag every pointermove re-runs this (via the offset effect) and
    // redundant transform writes produced transient "character zoomed in"
    // frames mid-drag before settling.
    const sp = model.scale as unknown as { x: number; set: (s: number) => void };
    if (sp.x !== scale) sp.set(scale);
    // Offsets are fractions of the canvas box: ±1.0 = ±half the box.
    const cx = canvasW / 2 + offsetXRef.current * (canvasW / 2);
    const cy = canvasH / 2 + offsetYRef.current * (canvasH / 2);
    const px = anchored ? cx : cx - (model.width * scale) / 2;
    const py = anchored ? cy : cy - (model.height * scale) / 2;
    if (model.x !== px) model.x = px;
    if (model.y !== py) model.y = py;
  };

  // Push prop updates into refs and re-layout live.
  useEffect(() => {
    zoomRef.current = zoom;
    offsetXRef.current = offsetX;
    offsetYRef.current = offsetY;
    applyLayout();
    // applyLayout is stable (closes over refs only).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, offsetX, offsetY]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;

    const container = containerRef.current;
    if (!container) return;

    // A mount right after the window was recreated can lose its first asset
    // requests: the webview's network process is still coming up while the
    // page fires its first XHRs, and WebKit reports them as `Status 0`
    // ("Load failed"). Give the load a few attempts with a short backoff
    // instead of leaving the stage empty for the whole session.
    const ATTEMPTS = 4;
    const RETRY_MS = 250;

    void (async () => {
      for (let attempt = 0; attempt < ATTEMPTS; attempt++) {
        let cleanupFn: (() => void) | null = null;
        try {
          cleanupFn = await mountLive2DScene({
            container,
            modelUrl,
            width,
            height,
            isDisposed: () => disposed,
            onModelReady: (info) => {
              modelRef.current = info;
              applyLayout();
            },
            overlay,
            onCharacterTap: () => onCharacterTapRef.current?.(),
          });
        } catch (err) {
          // A rejected import/fetch is just another failed attempt.
          console.warn(`[Live2D] mount attempt ${attempt + 1} threw:`, err);
        }
        if (disposed) {
          cleanupFn?.();
          return;
        }
        if (cleanupFn) {
          cleanup = cleanupFn;
          // Status for the dev harness: tells "the model is mounted" apart
          // from "an empty canvas exists" (the PIXI app is created before the
          // model finishes loading). `modelReadyAt` is ms since the page
          // started, so a window switch can be measured end to end.
          container.dataset.modelReady = "1";
          container.dataset.modelReadyAt = String(Math.round(performance.now()));
          return;
        }
        console.warn(
          `[Live2D] mount attempt ${attempt + 1}/${ATTEMPTS} failed`,
        );
        if (attempt + 1 < ATTEMPTS) {
          await new Promise((r) => window.setTimeout(r, RETRY_MS * (attempt + 1)));
          if (disposed) return;
        }
      }
    })();

    return () => {
      disposed = true;
      modelRef.current = null;
      delete container.dataset.modelReady;
      delete container.dataset.modelReadyAt;
      if (cleanup) cleanup();
    };
  }, [modelUrl, width, height, overlay]);

  return (
    <div
      ref={containerRef}
      style={{
        width,
        height,
        // The canvas inside enables pointer-events itself so clicks on
        // the avatar are captured. Clicks on transparent areas bubble
        // up to AvatarStage for window dragging.
        pointerEvents: "none",
      }}
    />
  );
}
