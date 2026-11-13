import { useEffect, useRef, useState } from "react";
import { mountLive2DScene, type ModelInfo } from "./mountScene";

/**
 * Mounts a PIXI canvas and loads a Live2D Cubism 2 / 3 / 4 / 5 model
 * from `modelUrl`.
 *
 * Graceful failure modes:
 *  - Cubism core script missing → returns `null` (parent shows placeholder).
 *  - Model fetch/parse error → returns `null` + console warning.
 *  - PIXI / pixi-live2d-display import error → returns `null`.
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
  const [failed, setFailed] = useState(false);
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
    model.scale.set(scale);
    // Offsets are fractions of the canvas box: ±1.0 = ±half the box.
    const cx = canvasW / 2 + offsetXRef.current * (canvasW / 2);
    const cy = canvasH / 2 + offsetYRef.current * (canvasH / 2);
    if (anchored) {
      model.x = cx;
      model.y = cy;
    } else {
      model.x = cx - (model.width * scale) / 2;
      model.y = cy - (model.height * scale) / 2;
    }
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

    void mountLive2DScene({
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
    }).then((cleanupFn) => {
      if (disposed) {
        cleanupFn?.();
        return;
      }
      if (cleanupFn) {
        cleanup = cleanupFn;
      } else {
        setFailed(true);
      }
    });

    return () => {
      disposed = true;
      modelRef.current = null;
      if (cleanup) cleanup();
    };
  }, [modelUrl, width, height, overlay]);

  if (failed) return null;

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
