import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { cancelRecording, startRecording, stopRecording } from "../api";
import { t, useLocale } from "../i18n";
import {
  EyeIcon,
  MicIcon,
  PaperclipIcon,
  StopIcon,
  WandIcon,
} from "./icons";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (text: string) => void;
  /// Vision callbacks. Each takes the current question text (may be empty).
  onVisionFull?: (prompt: string) => void;
  onOpenVisionRegionPicker?: (prompt: string) => void;
  onVisionImage?: (prompt: string, pngBase64: string) => void;
  onImagePrompt?: (prompt: string) => void;
  sttEnabled?: boolean;
  visionEnabled?: boolean;
  /** Bumped by the avatar stage on overlay-mode character taps so the
   * composer can (re)focus even while staying mounted. */
  focusTick?: number;
}

export default function InputField({
  open,
  onClose,
  onSubmit,
  onVisionFull,
  onOpenVisionRegionPicker,
  onVisionImage,
  onImagePrompt,
  sttEnabled = false,
  visionEnabled = false,
  focusTick = 0,
}: Props) {
  useLocale();
  const [value, setValue] = useState("");
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [micError, setMicError] = useState<string | null>(null);
  const [visionMenuOpen, setVisionMenuOpen] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setValue("");
      setMicError(null);
      queueMicrotask(() => ref.current?.focus());
    } else if (recording) {
      // Closing while recording: discard.
      cancelRecording().catch(() => {});
      setRecording(false);
    }
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  // Character-tap hand-off: when the avatar stage opens the composer via
  // a focus tick, move focus to the input right away.
  useEffect(() => {
    if (open && focusTick > 0) queueMicrotask(() => ref.current?.focus());
  }, [open, focusTick]); // eslint-disable-line react-hooks/exhaustive-deps

  const toggleMic = async () => {
    setMicError(null);
    if (recording) {
      setBusy(true);
      setRecording(false);
      setTranscribing(true);
      try {
        const text = await stopRecording();
        const trimmed = text.trim();
        if (trimmed) {
          // Auto-submit: user expectation is that speaking then tapping
          // "stop" sends the message. Pre-filling the text box and
          // requiring another Enter press is surprising.
          setValue("");
          onSubmit(trimmed);
        } else {
          setMicError(t("input.no_speech"));
          queueMicrotask(() => ref.current?.focus());
        }
      } catch (err) {
        setMicError(String(err));
      } finally {
        setTranscribing(false);
        setBusy(false);
      }
    } else {
      setBusy(true);
      try {
        await startRecording();
        setRecording(true);
      } catch (err) {
        setMicError(String(err));
      } finally {
        setBusy(false);
      }
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.form
          key="input"
          className="interactive"
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 4 }}
          transition={{ duration: 0.16, ease: "easeOut" }}
          onSubmit={(e) => {
            e.preventDefault();
            const text = value.trim();
            if (text) onSubmit(text);
          }}
          style={{
            position: "absolute",
            bottom: 16,
            left: 12,
            right: 12,
            zIndex: 10,
            padding: "6px 8px",
            borderRadius: 14,
            background: "rgba(20, 20, 28, 0.86)",
            backdropFilter: "blur(14px)",
            WebkitBackdropFilter: "blur(14px)",
            boxShadow:
              "0 8px 28px rgba(0,0,0,0.45), inset 0 0 0 1px rgba(255,255,255,0.06)",
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
            {sttEnabled && (
              <button
                type="button"
                onClick={toggleMic}
                disabled={busy}
                title={recording ? t("input.mic.stop") : t("input.mic.start")}
                aria-label={recording ? t("input.mic.stop") : t("input.mic.start")}
                style={composerBtn(recording)}
              >
                {recording ? <StopIcon size={13} /> : <MicIcon size={15} />}
              </button>
            )}
            <input
              ref={ref}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape" && (e.target as HTMLInputElement).value === "") {
                  onClose();
                }
              }}
              placeholder={
                transcribing
                  ? t("input.transcribing")
                  : recording
                  ? t("input.listening")
                  : t("input.placeholder")
              }
              style={{
                flex: 1,
                background: "transparent",
                border: "none",
                outline: "none",
                color: "#fff",
                fontSize: 14,
                padding: "8px 8px",
                minWidth: 0,
              }}
            />
            {onImagePrompt && (
              <button
                type="button"
                onClick={() => {
                  const prompt = value.trim();
                  if (!prompt) return;
                  onImagePrompt(prompt);
                  setValue("");
                }}
                disabled={!value.trim()}
                title={t("input.image_prompt")}
                aria-label={t("input.image_prompt")}
                style={composerBtn(false)}
              >
                <WandIcon size={15} />
              </button>
            )}
            {visionEnabled && (
              <div style={{ position: "relative" }}>
                <button
                  type="button"
                  onClick={() => setVisionMenuOpen((v) => !v)}
                  title={t("input.vision.tooltip")}
                  aria-label={t("input.vision.tooltip")}
                  style={composerBtn(visionMenuOpen)}
                >
                  <EyeIcon size={15} />
                </button>
                {visionMenuOpen && (
                  <div
                    style={{
                      position: "absolute",
                      bottom: 38,
                      right: 0,
                      minWidth: 200,
                      padding: 4,
                      borderRadius: 10,
                      background: "rgba(20,20,28,0.96)",
                      boxShadow:
                        "0 10px 28px rgba(0,0,0,0.5), inset 0 0 0 1px rgba(255,255,255,0.06)",
                      display: "flex",
                      flexDirection: "column",
                      gap: 1,
                      zIndex: 20,
                      backdropFilter: "blur(10px)",
                      WebkitBackdropFilter: "blur(10px)",
                    }}
                  >
                    <button
                      type="button"
                      style={menuBtn}
                      onClick={() => {
                        setVisionMenuOpen(false);
                        onVisionFull?.(value.trim());
                        setValue("");
                      }}
                    >
                      <EyeIcon size={13} style={{ opacity: 0.75 }} />
                      <span>{t("input.vision.full_screen")}</span>
                    </button>
                    <button
                      type="button"
                      style={menuBtn}
                      onClick={() => {
                        setVisionMenuOpen(false);
                        onOpenVisionRegionPicker?.(value.trim());
                        setValue("");
                      }}
                    >
                      <span
                        style={{
                          width: 13,
                          height: 13,
                          border: "1.4px dashed currentColor",
                          borderRadius: 2,
                          opacity: 0.75,
                          flexShrink: 0,
                        }}
                      />
                      <span>{t("input.vision.select_region")}</span>
                    </button>
                    <button
                      type="button"
                      style={menuBtn}
                      onClick={() => {
                        setVisionMenuOpen(false);
                        fileRef.current?.click();
                      }}
                    >
                      <PaperclipIcon size={13} style={{ opacity: 0.75 }} />
                      <span>{t("input.vision.attach_image")}</span>
                    </button>
                  </div>
                )}
                <input
                  ref={fileRef}
                  type="file"
                  accept="image/*"
                  style={{ display: "none" }}
                  onChange={async (e) => {
                    const file = e.target.files?.[0];
                    e.target.value = "";
                    if (!file) return;
                    const buf = await file.arrayBuffer();
                    const png = await convertToPngBase64(buf, file.type);
                    onVisionImage?.(value.trim(), png);
                    setValue("");
                  }}
                />
              </div>
            )}
          </div>
          {micError && (
            <div style={{ fontSize: 11, color: "#ff8888", padding: "0 6px" }}>
              {micError}
            </div>
          )}
        </motion.form>
      )}
    </AnimatePresence>
  );
}

const menuBtn: React.CSSProperties = {
  textAlign: "left",
  padding: "8px 10px",
  background: "transparent",
  color: "#fff",
  border: "none",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 13,
  display: "flex",
  alignItems: "center",
  gap: 8,
};

/// Compact, icon-only composer button. `active` lights up the background
/// (used for the recording mic + open vision menu).
function composerBtn(active: boolean): React.CSSProperties {
  return {
    width: 30,
    height: 30,
    borderRadius: 9,
    border: "1px solid rgba(255,255,255,0.10)",
    background: active ? "rgba(255,255,255,0.16)" : "rgba(255,255,255,0.04)",
    color: "#fff",
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    cursor: "pointer",
    flexShrink: 0,
    padding: 0,
  };
}

/// Re-encode an arbitrary uploaded image (jpeg/webp/etc.) to PNG base64.
/// Keeps the backend pipeline simple — vision_with_image expects PNG bytes.
async function convertToPngBase64(
  buf: ArrayBuffer,
  mime: string,
): Promise<string> {
  const blob = new Blob([buf], { type: mime || "image/png" });
  const url = URL.createObjectURL(blob);
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const i = new Image();
      i.onload = () => resolve(i);
      i.onerror = () => reject(new Error("image decode failed"));
      i.src = url;
    });
    // Cap at 1600px wide to keep base64 payloads small.
    const maxW = 1600;
    const scale = img.width > maxW ? maxW / img.width : 1;
    const w = Math.max(1, Math.round(img.width * scale));
    const h = Math.max(1, Math.round(img.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no 2d context");
    ctx.drawImage(img, 0, 0, w, h);
    const dataUrl = canvas.toDataURL("image/png");
    return dataUrl.replace(/^data:image\/png;base64,/, "");
  } finally {
    URL.revokeObjectURL(url);
  }
}
