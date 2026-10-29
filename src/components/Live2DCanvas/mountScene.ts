import { invoke } from "@tauri-apps/api/core";
import { avatarState, type AvatarState } from "../../avatarState";
import type { Emotion } from "../../emotion";
import { lipSync } from "../../lipsync";
import {
  EMOTION_MOTION_MAP,
  EXPRESSION_MAP,
  MOTION_MAP,
  paramSetFor,
} from "./params";
import { detectRuntime, ensureRuntime } from "./runtime";
import {
  tryExpressionAny,
  tryMotion,
  tryMotionAny,
  trySet,
  type CoreModelLike,
} from "./modelHelpers";
import { patchPixiUtilsUrl } from "./pixiPatch";

export interface ModelInfo {
  model: {
    scale: { set: (s: number) => void };
    x: number;
    y: number;
    width: number;
    height: number;
  };
  anchored: boolean;
  canvasW: number;
  canvasH: number;
}

export interface MountOptions {
  container: HTMLDivElement;
  modelUrl: string;
  width: number;
  height: number;
  /** Returns true when the calling effect has been torn down. */
  isDisposed: () => boolean;
  /** Called once the Live2D model is ready so the caller can re-layout. */
  onModelReady: (info: ModelInfo) => void;
}

/**
 * Mounts a PIXI canvas, loads the Live2D model and wires up lipsync,
 * idle motion, eye-tracking, blink, click reactions and avatar-state →
 * expression/motion mapping.
 *
 * Resolves with a `cleanup()` function on success, or `null` on any
 * fatal error (the caller should render a placeholder).
 */
export async function mountLive2DScene(
  opts: MountOptions,
): Promise<(() => void) | null> {
  const { container, modelUrl, width, height, isDisposed, onModelReady } = opts;

  const runtime = detectRuntime(modelUrl);
  const coreReady = await ensureRuntime(runtime);
  if (!coreReady) {
    console.warn(
      `[Live2D] ${runtime} runtime failed to load (local /public and CDN both unreachable)`,
    );
    return null;
  }

  try {
    const PIXI = await import("pixi.js");
    patchPixiUtilsUrl(PIXI);

    // The default entry of pixi-live2d-display-lipsyncpatch is Cubism 4
    // only. Load the matching sub-bundle so each runtime actually finds
    // its core lib.
    const mod =
      runtime === "cubism4"
        ? await import("pixi-live2d-display-lipsyncpatch/cubism4")
        : await import("pixi-live2d-display-lipsyncpatch/cubism2");
    const { Live2DModel } = mod;

    Live2DModel.registerTicker(PIXI.Ticker);

    if (isDisposed()) return null;

    const app = new PIXI.Application({
      width,
      height,
      backgroundAlpha: 0,
      antialias: true,
      autoDensity: true,
      resolution: window.devicePixelRatio || 1,
    });
    container.appendChild(app.view as unknown as Node);

    const model = await Live2DModel.from(modelUrl, {
      // `autoInteract` was deprecated in pixi-live2d-display v0.5.0 in
      // favour of these two granular flags. We drive both hit-testing
      // and focus tracking ourselves (see onPointerDown/onMove below),
      // so disable the library's global listeners.
      autoHitTest: false,
      autoFocus: false,
    });
    if (isDisposed()) {
      app.destroy(true, { children: true, texture: true, baseTexture: true });
      return null;
    }

    // Anchor to model centre so scaling + positioning is predictable
    // across both Cubism 2 and Cubism 4 models (their internal origins
    // differ — Cubism 2 uses top-left, Cubism 4 often uses centre).
    let anchored = false;
    try {
      (model as unknown as { anchor: { set: (x: number, y: number) => void } })
        .anchor.set(0.5, 0.5);
      anchored = true;
    } catch {
      /* some builds expose anchor on the internal sprite only */
    }

    onModelReady({
      model: model as unknown as ModelInfo["model"],
      anchored,
      canvasW: width,
      canvasH: height,
    });

    app.stage.addChild(
      model as unknown as (typeof PIXI)["DisplayObject"]["prototype"],
    );

    // Lip-sync: feed AnalyserNode RMS into ParamMouthOpenY every frame.
    let mouth = 0;
    const unsubscribe = lipSync.subscribe((level) => {
      mouth = level;
    });
    const coreModel = (
      model as unknown as { internalModel?: { coreModel?: CoreModelLike } }
    ).internalModel?.coreModel;

    if (coreModel) {
      const apis = [
        typeof coreModel.setParameterValueById === "function" && "ById",
        typeof coreModel.setParamFloat === "function" && "ParamFloat",
        typeof coreModel.setParameterValueByIndex === "function" && "ByIndex",
      ].filter(Boolean);
      console.log(
        `[live2d] runtime=${runtime} coreModel APIs: [${apis.join(", ")}]`,
      );
    } else {
      console.warn("[live2d] no coreModel — lipsync will be disabled");
    }

    const P = paramSetFor(runtime);

    // If the model3.json declares an explicit LipSync parameter group,
    // honour it: those IDs are always the right ones to drive.
    const declaredLipSyncIds: string[] = (() => {
      try {
        const settings = (
          model as unknown as {
            internalModel?: {
              settings?: {
                groups?: Array<{ Name?: string; Ids?: string[] }>;
              };
            };
          }
        ).internalModel?.settings;
        const groups = settings?.groups ?? [];
        const grp = groups.find((g) => g?.Name === "LipSync");
        return Array.isArray(grp?.Ids) ? grp!.Ids! : [];
      } catch {
        return [];
      }
    })();
    if (declaredLipSyncIds.length > 0) {
      const merged = [...declaredLipSyncIds];
      for (const id of P.mouthIds) if (!merged.includes(id)) merged.push(id);
      P.mouthIds = merged;
      console.log(
        `[live2d] LipSync group: [${declaredLipSyncIds.join(", ")}]`,
      );
    }

    // Idle body sway: gentle sinusoidal drift on head/body params.
    const bornAt = performance.now();

    // Track mouse for natural eye-tracking. We use pixi-live2d-display's
    // built-in `focus(x, y)` API: it smoothly drives ParamAngle*/ParamBody*
    // /ParamEyeBall* with damping AND cooperates with the model's
    // physics/idle motions.
    const focusable = model as unknown as {
      focus?: (x: number, y: number, instant?: boolean) => void;
    };
    const onMove = (e: PointerEvent) => {
      const r = container.getBoundingClientRect();
      const px = e.clientX - r.left;
      const py = e.clientY - r.top;
      if (typeof focusable.focus === "function") {
        try {
          focusable.focus(px, py);
        } catch {
          /* ignore — some runtimes don't expose focus */
        }
      }
    };
    window.addEventListener("pointermove", onMove);

    // Blink on a stochastic schedule, like a human.
    let nextBlinkAt = performance.now() + 2000 + Math.random() * 3000;
    let blinkStart = 0;
    const BLINK_MS = 140;

    const applyMouth = () => {
      const now = performance.now();
      const t = (now - bornAt) / 1000;
      if (!coreModel) return;
      // Drive every candidate LipSync param — the model only owns a
      // subset and the rest no-op silently.
      const mouthVal = Math.min(1, mouth * 1.6);
      for (const id of P.mouthIds) trySet(coreModel, id, mouthVal);
      // Subtle body sway and breathing so she still feels alive when
      // the cursor is parked. Head/eye angles are driven by `focus(...)`.
      trySet(coreModel, P.bodyX, Math.sin(t * 0.3) * 3);
      trySet(coreModel, P.breath, 0.5 + Math.sin(t * 1.2) * 0.5);
      // Blink.
      if (blinkStart === 0 && now >= nextBlinkAt) {
        blinkStart = now;
      }
      if (blinkStart > 0) {
        const dt = now - blinkStart;
        if (dt < BLINK_MS) {
          // Triangular envelope: closed at the midpoint.
          const k = 1 - Math.abs(dt - BLINK_MS / 2) / (BLINK_MS / 2);
          const open = 1 - k;
          trySet(coreModel, P.eyeLOpen, open);
          trySet(coreModel, P.eyeROpen, open);
        } else {
          blinkStart = 0;
          nextBlinkAt = now + 2500 + Math.random() * 4000;
          trySet(coreModel, P.eyeLOpen, 1);
          trySet(coreModel, P.eyeROpen, 1);
        }
      }
    };
    app.ticker.add(applyMouth);

    // Drive Live2D expressions and occasional motions from avatar state.
    // For Cubism 2 models expressions usually don't exist, so we also
    // map emotions to motion groups as a fallback.
    let lastEmotion: Emotion = "neutral";
    let lastMode: AvatarState["mode"] = "idle";
    const unsubState = avatarState.subscribe((s) => {
      if (s.emotion !== lastEmotion) {
        lastEmotion = s.emotion;
        console.log(`[avatar] emotion → ${s.emotion}`);
        tryExpressionAny(model, EXPRESSION_MAP[s.emotion]);
        const emotionMotion = EMOTION_MOTION_MAP[s.emotion];
        if (emotionMotion) tryMotionAny(model, emotionMotion);
      }
      if (s.mode !== lastMode) {
        lastMode = s.mode;
        const motion = MOTION_MAP[s.mode];
        if (motion) tryMotionAny(model, [motion]);
      }
    });

    // Periodic "special" motion when idle — every 45–90 s the avatar
    // plays one of the model's flair animations (e.g. mao_pro's brush
    // strokes via special_01..03), so she feels alive instead of
    // statically idling. Skipped while she is speaking or listening.
    //
    // mao_pro packs ALL non-idle motions (mtn_02..04 + special_01..03)
    // into the unnamed group `""` of the model3.json, so we randomise
    // an index within that group on top of trying conventional names.
    let nextSpecialAt = performance.now() + 30000 + Math.random() * 30000;
    const specialTimer = window.setInterval(() => {
      const now = performance.now();
      if (now < nextSpecialAt) return;
      if (lastMode !== "idle") return;
      const played = tryMotionAny(model, [
        "special_01",
        "special_02",
        "special_03",
        "Special",
        "TapSpecial",
      ]);
      if (!played) {
        // mao_pro's "" group has 6 entries (indices 0–5); 3–5 are the
        // brush "special" animations. Picking a random one keeps the
        // performance varied.
        tryMotion(model, "", 3 + Math.floor(Math.random() * 3));
      }
      nextSpecialAt = now + 45000 + Math.random() * 45000;
    }, 5000);

    // Click-to-interact: tap on the avatar → play a random body/head
    // motion AND have her say a random reaction line. We allow the
    // event to bubble so AvatarStage's window drag handler still fires.
    const canvas = app.view as HTMLCanvasElement;
    canvas.style.pointerEvents = "auto";
    canvas.style.cursor = "grab";
    let pressX = 0;
    let pressY = 0;
    let pressAt = 0;
    const onPointerDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      pressX = e.clientX;
      pressY = e.clientY;
      pressAt = performance.now();
    };
    const onPointerUp = (e: PointerEvent) => {
      if (e.button !== 0 || pressAt === 0) return;
      const dx = e.clientX - pressX;
      const dy = e.clientY - pressY;
      const dt = performance.now() - pressAt;
      pressAt = 0;
      // Drag threshold: if the pointer moved more than ~6 px or was
      // held longer than 350 ms, treat it as a drag (AvatarStage is
      // doing its thing) and don't fire a tap.
      if (Math.hypot(dx, dy) > 6 || dt > 350) return;
      const r = canvas.getBoundingClientRect();
      // The canvas can be much wider than the model; a tap on empty
      // canvas is not a tap on her. With autoDensity, CSS pixels from
      // the canvas corner are stage coordinates.
      const bounds = (
        model as unknown as {
          getBounds: () => { contains: (x: number, y: number) => boolean };
        }
      ).getBounds();
      if (!bounds.contains(e.clientX - r.left, e.clientY - r.top)) return;
      const ny = (e.clientY - r.top) / r.height;
      // Three vertical zones — the lower third of the avatar covers
      // her hand holding the paint brush, so taps there should always
      // play the brush-stroke specials and trigger the "drawing"
      // reaction kind. Mid-body falls through to body taps.
      const zone = ny < 0.33 ? "head" : ny > 0.7 ? "hand" : "body";
      const headGroups = ["tap_head", "TapHead"];
      let played = false;
      if (zone === "head") {
        played = tryMotionAny(model, headGroups);
      }
      if (!played) {
        // Indices 3, 4, 5 of group "" map to special_01/02/03 in
        // mao_pro.model3.json — the brush-stroke animations.
        const idx = 3 + Math.floor(Math.random() * 3);
        played = tryMotion(model, "", idx);
      }
      if (!played) {
        // Last-ditch fallback: try named special groups.
        tryMotionAny(model, ["special_01", "special_02", "special_03"]);
      }
      // Fire-and-forget reaction line through whichever TTS provider
      // is active. Silent when TTS is disabled. The Rust side now
      // generates the line via LLM (mode-aware, multilingual) and
      // falls back to canned localized strings on timeout.
      invoke("speak_reaction", { zone }).catch(() => {});
    };
    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointerup", onPointerUp);

    return () => {
      unsubscribe();
      unsubState();
      window.removeEventListener("pointermove", onMove);
      window.clearInterval(specialTimer);
      try {
        canvas.removeEventListener("pointerdown", onPointerDown);
        canvas.removeEventListener("pointerup", onPointerUp);
      } catch {
        /* ignore */
      }
      try {
        app.ticker.remove(applyMouth);
      } catch {
        /* ignore */
      }
      try {
        model.destroy({ children: true, texture: true, baseTexture: true });
      } catch {
        /* ignore */
      }
      app.destroy(true, { children: true, texture: true, baseTexture: true });
    };
  } catch (err) {
    console.warn("[Live2D] failed to initialize:", err);
    return null;
  }
}
