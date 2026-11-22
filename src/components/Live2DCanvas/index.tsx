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
  onCharacterTap,
}: {
  modelUrl: string;
  width: number;
  height: number;
  zoom?: number;
  offsetX?: number;
  offsetY?: number;
  /** A clean tap on the character toggles the message bar; the reaction still
   * plays so the tap does not feel dead. */
  onCharacterTap?: () => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Live layout refs — let the user tweak avatar zoom/position without
  // tearing down the whole PIXI app + Live2D model.
  const modelRef = useRef<ModelInfo | null>(null);
  const zoomRef = useRef(zoom);
  const offsetXRef = useRef(offsetX);
  const offsetYRef = useRef(offsetY);
  // Character-tap hand-off. Kept in a ref so the mount effect does not
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
    const { model, modelW, modelH, anchored, canvasW, canvasH } = ref;
    // Fit from the scale-free size. `model.width`/`height` are
    // `scale * localBounds` (PIXI `Container`), so a fit derived from them is
    // inversely proportional to the applied scale: successive calls alternate
    // between two sizes instead of settling. That is the scale flip seen while
    // dragging, when this runs on every pointermove.
    if (!(modelW > 0) || !(modelH > 0)) return;
    // She is furniture on the desktop: a third of the screen, leaving the
    // rest to the desktop she stands on.
    const fit = Math.min(canvasW / modelW, canvasH / modelH) * (1 / 3);
    const scale = fit * Math.max(0.1, zoomRef.current);
    // Only write the transform when it actually changed.
    const sp = model.scale as unknown as { x: number; set: (s: number) => void };
    if (sp.x !== scale) sp.set(scale);
    // Offsets are fractions of the canvas box: ±1.0 = ±half the box.
    const cx = canvasW / 2 + offsetXRef.current * (canvasW / 2);
    const cy = canvasH / 2 + offsetYRef.current * (canvasH / 2);
    const px = anchored ? cx : cx - (modelW * scale) / 2;
    const py = anchored ? cy : cy - (modelH * scale) / 2;
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

    // A fresh webview can lose its first asset requests while its network
    // process is coming up; WebKit reports them as `Status 0` ("Load failed").
    // Give the load a few attempts with a short backoff instead of leaving the
    // stage empty for the whole session.
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
          // started, so a slow mount can be measured end to end.
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
  }, [modelUrl, width, height]);

  return (
    <div
      ref={containerRef}
      style={{
        width,
        height,
        // The canvas inside enables pointer-events itself so clicks on
        // the avatar are captured. Clicks on transparent areas are
        // click-through; they never reach the page.
        pointerEvents: "none",
      }}
    />
  );
}
