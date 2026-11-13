import { useCallback, useEffect, useRef, useState } from "react";
import AvatarStage from "./components/AvatarStage";
import ChatBubble from "./components/ChatBubble";
import InputField from "./components/InputField";
import ModelWizard from "./components/ModelWizard";
import RegionPicker from "./components/RegionPicker";
import SettingsPanel from "./components/SettingsPanel";
import { ToastHost, toast } from "./components/Toast";
import TopBar from "./components/TopBar";
import { exit as tauriExit } from "@tauri-apps/plugin-process";
import { invoke } from "@tauri-apps/api/core";
import { avatarState } from "./avatarState";
import { lipSync } from "./lipsync";
import { checkForUpdatesQuietly } from "./updater";
import { bootstrapLocale, useLocale } from "./i18n";
import {
  cancelGeneration,
  cancelImageGeneration,
  desktopListScreens,
  desktopScreenshot,
  enterRegionPickerMode,
  exitRegionPickerMode,
  feedbackRecord,
  generateImage,
  getSettings,
  getWeather,
  resetChat,
  saveGeneratedImage,
  sendMessage,
  setListenEnabled,
  getWindowOverlayInfo,
  setWindowOverlay,
  type WindowOverlayMode,
  visionCaptureFull,
  visionWithImage,
  type PublicSettings,
  type ScreenInfo,
} from "./api";
import { useHotkeys } from "./hooks/useHotkeys";
import { useTtsAudio } from "./hooks/useTtsAudio";
import { useTransientBubbles } from "./hooks/useTransientBubbles";
import { useChatStream, type LastTurn, type Route } from "./hooks/useChatStream";
import { useImageStream } from "./hooks/useImageStream";
import { useRelationship } from "./hooks/useRelationship";
import { useListenController } from "./hooks/useListenController";

const OVERLAY_STORAGE_KEY = "komorebi.overlay_mode";

/**
 * Overlay is the product default: a transparent, frontmost layer covering
 * the monitor. The choice is remembered so an explicit "normal" survives a
 * restart instead of the startup effect forcing overlay every launch.
 */
function readStoredOverlayMode(): WindowOverlayMode {
  try {
    const v = localStorage.getItem(OVERLAY_STORAGE_KEY);
    // Only two modes exist: the monitor-covering overlay and a windowed
    // window. `always_on_top` was an early label for "frontmost without the
    // overlay treatment" — it never covered the monitor, so it is retired
    // into `overlay`.
    if (v === "normal" || v === "overlay") return v;
    if (v === "always_on_top") return "overlay";
  } catch {
    /* storage unavailable (private mode, etc.) — use the default */
  }
  return "overlay";
}

function storeOverlayMode(mode: WindowOverlayMode): void {
  try {
    localStorage.setItem(OVERLAY_STORAGE_KEY, mode);
  } catch {
    /* best effort: the mode still applies for this session */
  }
}

export default function App() {
  // ── UI panels ──────────────────────────────────────────────────────
  const [inputOpen, setInputOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerInitialPrompt, setPickerInitialPrompt] = useState("");
  const [pickerPrebuilt, setPickerPrebuilt] = useState<{
    bytes: Uint8Array;
    screen: ScreenInfo | null;
  } | null>(null);

  // ── Bubble / chat-turn state ───────────────────────────────────────
  const [bubbleText, setBubbleText] = useState<string | null>(null);
  const [userEcho, setUserEcho] = useState<string | null>(null);
  const [thinking, setThinking] = useState(false);
  const [route, setRoute] = useState<Route | null>(null);
  const [feedbackKey, setFeedbackKey] = useState<number>(0);

  // ── Image generation state ─────────────────────────────────────────
  const [imageBase64, setImageBase64] = useState<string | null>(null);
  const [imageSavePath, setImageSavePath] = useState<string | null>(null);
  const [imageStatus, setImageStatus] = useState<
    "generating" | "done" | "error" | null
  >(null);
  const [imageError, setImageError] = useState<string | null>(null);

  // ── Backend snapshot ───────────────────────────────────────────────
  const [settings, setSettings] = useState<PublicSettings | null>(null);

  // ── Refs ───────────────────────────────────────────────────────────
  const activeImageIdRef = useRef<string | null>(null);
  const activeIdRef = useRef<string | null>(null);
  // Accumulated *raw* token stream including any `<mood:X>` markers, used
  // for emotion classification. The visible bubble text strips the markers
  // before display, so we can't re-derive the raw text from it.
  const rawTextRef = useRef<string>("");
  const bubbleTimer = useRef<number | null>(null);
  const settingsRef = useRef<PublicSettings | null>(null);
  const handleSubmitRef = useRef<((text: string) => void) | null>(null);
  // Tracks the most recent assistant turn so the feedback buttons can
  // attach the right prompt/response pair to a 👍/👎 click. Lives in a
  // ref because the chat-event handler closure mustn't depend on it.
  const lastTurnRef = useRef<LastTurn | null>(null);

  // ── Settings refresh ───────────────────────────────────────────────
  const refreshSettings = useCallback(() => {
    getSettings().then(setSettings).catch(() => {});
  }, []);
  useEffect(() => {
    refreshSettings();
    void bootstrapLocale();
  }, [refreshSettings]);

  // Re-bootstrap locale whenever the language preference changes.
  useEffect(() => {
    if (settings?.language !== undefined) void bootstrapLocale();
  }, [settings?.language]);
  // Subscribe to locale changes so all `t(...)` calls re-render.
  useLocale();

  // Check for app updates in the background on startup.
  useEffect(() => {
    checkForUpdatesQuietly();
  }, []);

  // Keep a ref of the latest settings so hooks created once
  // (like ListenController) always read the current values.
  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  // ── Helpers shared by handlers and effects ────────────────────────
  const scheduleBubbleHide = useCallback((ms = 8000) => {
    if (bubbleTimer.current) window.clearTimeout(bubbleTimer.current);
    bubbleTimer.current = window.setTimeout(() => {
      setBubbleText(null);
      setUserEcho(null);
      setRoute(null);
      setImageBase64(null);
      setImageSavePath(null);
      setImageStatus(null);
      setImageError(null);
    }, ms);
  }, []);

  const openPickerWithPrompt = useCallback(
    async (prompt: string) => {
      try {
        setPickerInitialPrompt(prompt);
        // Capture the screenshot BEFORE we resize/raise the main window
        // to fullscreen. Otherwise, when an external app is also
        // capturing the desktop (e.g. Discord screen-share), DWM stops
        // compositing through our transparent layered window and the
        // monitor capture comes back as a black rectangle covering the
        // desktop.
        const screens = await desktopListScreens();
        const primary = screens.find((s) => s.is_primary) ?? screens[0] ?? null;
        const bytes = await desktopScreenshot(0);
        setPickerPrebuilt({ bytes, screen: primary });
        await enterRegionPickerMode(prompt);
        setPickerOpen(true);
      } catch (err) {
        setBubbleText(`⚠ ${String(err)}`);
        scheduleBubbleHide(5000);
      }
    },
    [scheduleBubbleHide],
  );

  const closePicker = useCallback(async () => {
    setPickerOpen(false);
    setPickerPrebuilt(null);
    try {
      await exitRegionPickerMode();
    } catch {
      // best effort restore
    }
  }, []);

  // ── Overlay mode: frontmost, covering the monitor ──────────────────────
  const [overlayMode, setOverlayMode] = useState<WindowOverlayMode>(readStoredOverlayMode);

  /** Single place that changes the mode: keeps the window, the state and
   * the remembered choice in sync. */
  const changeOverlayMode = useCallback((next: WindowOverlayMode) => {
    setOverlayMode(next);
    storeOverlayMode(next);
  }, []);

  /** Apply a requested overlay/windowed mode through the backend and sync
   * the local state + remembered choice. Shared by the F11 and Alt+O
   * handlers so both paths run the exact same two-mode logic (and surface
   * backend failures instead of silently leaving stale state behind).
   *
   * The choice is persisted *before* the backend call: a window-type change
   * recreates the webview in-process, so the page reloads. If the write
   * landed after the reload the fresh page would read the old mode and
   * immediately switch back. */
  const applyRequestedMode = useCallback(
    async (next: WindowOverlayMode) => {
      changeOverlayMode(next);
      try {
        const eff = await setWindowOverlay(next);
        if (eff && eff !== next) changeOverlayMode(eff);
      } catch (err) {
        // The invoke may never answer when the window is being replaced; the
        // reloaded page re-reads the mode from the backend, so this is safe.
        console.warn("komorebi: failed to apply overlay mode", next, err);
      }
    },
    [changeOverlayMode],
  );

  /** F11: toggle between the two window states we offer — the overlay
   * (frontmost, covering the monitor) and a plain windowed window. There is
   * deliberately no third state: compositor
   * fullscreen would lose the transparency, so "fullscreen without overlay"
   * is not a mode we expose. */
  const handleToggleWindow = useCallback(async () => {
    await applyRequestedMode(overlayMode === "overlay" ? "normal" : "overlay");
  }, [overlayMode, applyRequestedMode]);

  /** Leave overlay mode outright. Overlay → normal; the window becomes
   * draggable like a normal Komorebi window again. */
  const handleExitOverlay = useCallback(async () => {
    await applyRequestedMode("normal");
  }, [applyRequestedMode]);

  // Escape leaves overlay mode (unless the composer owns the key while
  // open) — an explicit off-ramp that does not depend on a global hotkey
  // being pressed.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (overlayMode !== "overlay") return;
      if (inputOpen) return;
      void handleExitOverlay();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [overlayMode, inputOpen, handleExitOverlay]);

  // ── Hooks (effects extracted out of App for clarity) ──────────────
  useHotkeys({
    onToggleInput: useCallback(() => setInputOpen((v) => !v), []),
    onVisionRegion: useCallback(
      () => void openPickerWithPrompt(""),
      [openPickerWithPrompt],
    ),
    onOverlay: useCallback(async () => {
      // Two-mode toggle (matches F11 and the move grip): fullscreen overlay
      // ↔ windowed. `always_on_top` is an implementation detail of overlay,
      // not a third user-facing state.
      await applyRequestedMode(overlayMode === "overlay" ? "normal" : "overlay");
    }, [overlayMode, applyRequestedMode]),
    onToggleWindow: handleToggleWindow,
  });

  useTtsAudio({
    outputDevice: settings?.audio_output_device,
    volume: settings?.tts_volume,
  });

  useTransientBubbles({ activeIdRef, setBubbleText, bubbleTimer });

  const { listening, heardHint } = useListenController({
    settings,
    settingsRef,
    refreshSettings,
    handleSubmitRef,
    setBubbleText,
    setUserEcho,
  });

  useEffect(() => {
    let mounted = true;
    (async () => {
      const info = await getWindowOverlayInfo();
      if (!mounted || !info.supported) return;
      // A pinned KOMOREBI_START_MODE wins; otherwise use the remembered
      // choice. Two modes only: normal (windowed) vs overlay.
      // If the backend is not already in that mode (e.g. a plain dev start
      // with a remembered overlay), request it: changing the window type
      // recreates the window in-process, which `applyRequestedMode` handles.
      const next: WindowOverlayMode = info.forced
        ? info.mode
        : readStoredOverlayMode();
      if (!mounted) return;
      if (next !== info.mode) await applyRequestedMode(next);
      else changeOverlayMode(next);
    })().catch(() => {
      /* window mode is cosmetic: keep the in-page state on failure */
    });
    return () => {
      mounted = false;
    };
  }, [applyRequestedMode, changeOverlayMode]);

  useChatStream({
    activeIdRef,
    lastTurnRef,
    settingsRef,
    rawTextRef,
    setRoute,
    setBubbleText,
    setThinking,
    setFeedbackKey,
    scheduleBubbleHide,
  });

  useImageStream({
    activeImageIdRef,
    activeIdRef,
    setImageStatus,
    setImageError,
    setImageBase64,
    setImageSavePath,
    setThinking,
    scheduleBubbleHide,
  });

  const relationship = useRelationship({ setBubbleText, scheduleBubbleHide });

  // ── Handlers ──────────────────────────────────────────────────────
  const handleToggleListen = useCallback(async () => {
    const next = !(settings?.listen_enabled ?? false);
    await setListenEnabled(next);
    refreshSettings();
  }, [settings?.listen_enabled, refreshSettings]);

  const handleImagePrompt = useCallback(
    async (prompt: string, size?: { width: number; height: number }) => {
      if (bubbleTimer.current) window.clearTimeout(bubbleTimer.current);
      setBubbleText(null);
      setUserEcho(`🎨 ${prompt}`);
      setRoute(null);
      setImageBase64(null);
      setImageSavePath(null);
      setImageError(null);
      setImageStatus("generating");
      setThinking(true);
      avatarState.setThinking();
      try {
        const id = await generateImage(prompt, size);
        activeImageIdRef.current = id;
        activeIdRef.current = id;
      } catch (err) {
        setImageStatus("error");
        setImageError(String(err));
        setThinking(false);
        avatarState.onDone(500);
      }
    },
    [],
  );

  const handleSaveImage = useCallback(async () => {
    if (!imageBase64) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: `image-${Date.now()}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (typeof target === "string" && target.trim()) {
        await saveGeneratedImage(imageBase64, target);
        setImageSavePath(target);
        toast.success(`Saved to ${target}`);
      }
    } catch (err) {
      setImageError(String(err));
      toast.error(`Save failed: ${String(err)}`);
    }
  }, [imageBase64]);

  const handleCopyImage = useCallback(async () => {
    if (!imageBase64) return;
    try {
      const bin = atob(imageBase64);
      const buf = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
      const blob = new Blob([buf], { type: "image/png" });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const CI: any = (window as any).ClipboardItem;
      if (CI && navigator.clipboard?.write) {
        await navigator.clipboard.write([new CI({ "image/png": blob })]);
        toast.success("Copied to clipboard");
      }
    } catch (err) {
      setImageError(String(err));
      toast.error(`Copy failed: ${String(err)}`);
    }
  }, [imageBase64]);

  const handleCancelImage = useCallback(async () => {
    try {
      await cancelImageGeneration();
    } catch {
      // best-effort
    }
  }, []);

  // Weather slash command — always emits a chat-style bubble in response.
  const handleWeatherSlash = useCallback(
    async (city: string) => {
      if (bubbleTimer.current) window.clearTimeout(bubbleTimer.current);
      setUserEcho(city ? `🌤️ /weather ${city}` : "🌤️ /weather");
      setBubbleText("");
      setRoute("skill");
      setThinking(true);
      avatarState.setThinking();
      try {
        const r = await getWeather(city || undefined);
        const loc = r.location.country
          ? `${r.location.name}, ${r.location.country}`
          : r.location.name;
        const tempUnit = r.units === "imperial" ? "°F" : "°C";
        const windUnit = r.units === "imperial" ? "mph" : "м/с";
        const parts = [
          `${r.icon} ${loc}: ${Math.round(r.temperature)}${tempUnit}`,
        ];
        if (r.feels_like != null)
          parts.push(`ощущается ${Math.round(r.feels_like)}${tempUnit}`);
        parts.push(r.description);
        if (r.wind_speed != null)
          parts.push(`ветер ${r.wind_speed.toFixed(1)} ${windUnit}`);
        if (r.humidity != null)
          parts.push(`влажность ${Math.round(r.humidity)}%`);
        setBubbleText(parts.join(" · "));
      } catch (err) {
        setBubbleText(`⚠ Не удалось получить погоду: ${String(err)}`);
      } finally {
        setThinking(false);
        avatarState.onDone();
        scheduleBubbleHide(12000);
      }
    },
    [scheduleBubbleHide],
  );

  const handleSubmit = async (text: string) => {
    if (
      text.trim().toLowerCase().startsWith("/img ") ||
      text.trim().toLowerCase() === "/img"
    ) {
      const prompt = text.trim().replace(/^\/img\s*/i, "").trim();
      if (!prompt) {
        setBubbleText("Usage: /img <prompt>");
        scheduleBubbleHide(4000);
        return;
      }
      void handleImagePrompt(prompt);
      return;
    }
    if (
      text.trim().toLowerCase().startsWith("/weather") ||
      text.trim().toLowerCase() === "/weather"
    ) {
      const city = text.trim().replace(/^\/weather\s*/i, "").trim();
      void handleWeatherSlash(city);
      return;
    }
    if (bubbleTimer.current) window.clearTimeout(bubbleTimer.current);
    setBubbleText("");
    setUserEcho(text);
    setThinking(true);
    setRoute(null);
    setImageBase64(null);
    setImageSavePath(null);
    setImageStatus(null);
    setImageError(null);
    avatarState.setThinking();
    activeIdRef.current = "pending";
    // Stash the prompt now; route + modelLabel are filled in once the
    // backend emits "started" with the resolved route.
    lastTurnRef.current = {
      id: "pending",
      prompt: text,
      response: "",
      route: "local",
      modelLabel: "",
    };
    try {
      // Auto screen-watch mode: every text turn implicitly attaches a
      // fresh screenshot, so the assistant is "looking" at the desktop
      // throughout the conversation. Costs an OpenRouter vision request
      // per message — gated behind an explicit setting + key.
      const id =
        settings?.auto_screen_watch_enabled && settings?.has_openrouter_key
          ? await visionCaptureFull(text)
          : await sendMessage(text);
      if (activeIdRef.current === "pending") activeIdRef.current = id;
    } catch (err) {
      activeIdRef.current = null;
      setThinking(false);
      avatarState.onDone(500);
      setBubbleText(`⚠ ${String(err)}`);
      scheduleBubbleHide(6000);
    }
  };
  handleSubmitRef.current = handleSubmit;

  // Generic vision dispatcher: kicks the chat-state machine the same way
  // handleSubmit does, then awaits the chosen invoke. Backend emits the
  // normal `chat:*` event stream so the bubble, route badge, emotion tags,
  // and TTS pipeline all work without further wiring.
  const runVision = async (
    label: string,
    prompt: string,
    invoker: () => Promise<string>,
  ) => {
    if (bubbleTimer.current) window.clearTimeout(bubbleTimer.current);
    setBubbleText("");
    setUserEcho(prompt ? `${label}: ${prompt}` : label);
    setThinking(true);
    setRoute(null);
    avatarState.setThinking();
    activeIdRef.current = "pending";
    try {
      const id = await invoker();
      if (activeIdRef.current === "pending") activeIdRef.current = id;
    } catch (err) {
      activeIdRef.current = null;
      setThinking(false);
      avatarState.onDone(500);
      setBubbleText(`⚠ ${String(err)}`);
      scheduleBubbleHide(6000);
    }
  };

  const handleVisionFull = (prompt: string) =>
    runVision("👁 screen", prompt, () => visionCaptureFull(prompt));
  const handleVisionImage = (prompt: string, pngBase64: string) =>
    runVision("🖼 image", prompt, () => visionWithImage(prompt, pngBase64));

  const handleReset = async () => {
    await cancelGeneration();
    await resetChat();
    lipSync.stop();
    avatarState.reset();
    setBubbleText(null);
    setUserEcho(null);
    setRoute(null);
    setThinking(false);
  };

  // ── Render ────────────────────────────────────────────────────────
  const sttReady = Boolean(
    (settings?.stt_available && settings?.whisper_model_path) ||
      (settings?.openrouter_stt_enabled && settings?.has_openrouter_key) ||
      settings?.faster_whisper_enabled ||
      (settings?.deepgram_enabled && settings?.has_deepgram_key),
  );

  return (
    <>
      <AvatarStage
        modelUrl={
          settings?.live2d_model_url ?? "/live2d/mao_pro/mao_pro.model3.json"
        }
        zoom={settings?.avatar_zoom ?? 1}
        offsetX={settings?.avatar_offset_x ?? 0}
        offsetY={settings?.avatar_offset_y ?? 0}
      />
      <ChatBubble
        text={bubbleText}
        route={route}
        thinking={thinking}
        userEcho={userEcho}
        imageBase64={imageBase64}
        imageSavePath={imageSavePath}
        imageStatus={imageStatus}
        imageError={imageError}
        onSaveImage={handleSaveImage}
        onCopyImage={handleCopyImage}
        onCancelImage={handleCancelImage}
        feedbackKey={feedbackKey}
        onFeedback={(rating) => {
          const turn = lastTurnRef.current;
          if (!turn || !turn.response) return;
          const lang = settingsRef.current?.language || "en";
          void feedbackRecord({
            modelLabel: turn.modelLabel,
            route: turn.route,
            prompt: turn.prompt,
            response: turn.response,
            rating,
            lang,
          }).catch(() => {});
        }}
      />
      <InputField
        open={inputOpen && !settingsOpen && !wizardOpen}
        onClose={() => setInputOpen(false)}
        onSubmit={handleSubmit}
        onVisionFull={handleVisionFull}
        onOpenVisionRegionPicker={(prompt) => {
          void openPickerWithPrompt(prompt);
        }}
        onVisionImage={handleVisionImage}
        onImagePrompt={(prompt) => void handleImagePrompt(prompt)}
        visionEnabled={Boolean(settings?.has_openrouter_key)}
        sttEnabled={sttReady}
      />
      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onChanged={refreshSettings}
      />
      <ModelWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onSettingsChanged={refreshSettings}
        settings={settings}
      />
      <TopBar
        mode={settings?.mode ?? "auto"}
        hasKey={settings?.has_openrouter_key ?? false}
        listenEnabled={settings?.listen_enabled ?? false}
        listenReady={sttReady}
        listening={listening}
        heard={heardHint}
        onToggleListen={handleToggleListen}
        onToggleSettings={() => {
          setWizardOpen(false);
          setSettingsOpen((v) => !v);
        }}
        onToggleWizard={() => {
          setSettingsOpen(false);
          setWizardOpen((v) => !v);
        }}
        onReset={handleReset}
        onQuit={() => tauriExit(0)}
        autoWatch={settings?.auto_screen_watch_enabled === true}
        autoWatchAvailable={Boolean(settings?.has_openrouter_key)}
        onToggleAutoWatch={async () => {
          const next = !(settings?.auto_screen_watch_enabled ?? false);
          await invoke("set_auto_screen_watch_enabled", { enabled: next });
          refreshSettings();
        }}
        relationship={relationship}
        showRelationshipBadge={
          (settings?.relationship_visibility ?? "indicator") !== "hidden"
        }
        overlayMode={overlayMode}
        onToggleOverlay={() => void handleToggleWindow()}
        onExitOverlay={() => void handleExitOverlay()}
      />
      <RegionPicker
        open={pickerOpen}
        initialPrompt={pickerInitialPrompt}
        prebuilt={pickerPrebuilt ?? undefined}
        onCancel={() => {
          void closePicker();
        }}
        onSubmit={(s) => {
          void closePicker();
          // Composited PNG already includes every region rectangle and
          // its importance tag, so route through `visionWithImage` rather
          // than asking the backend to re-crop a single rect.
          handleVisionImage(s.prompt, s.imageBase64);
        }}
        onGenerateVariant={(prompt, size) => {
          void closePicker();
          void handleImagePrompt(prompt, size);
        }}
      />
      <ToastHost />
    </>
  );
}
