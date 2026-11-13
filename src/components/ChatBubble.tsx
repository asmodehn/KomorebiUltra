import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useState, type JSX } from "react";
import { t, useLocale } from "../i18n";
import { ThumbDownIcon, ThumbUpIcon } from "./icons";

const imgButtonStyle: React.CSSProperties = {
  fontSize: 11,
  padding: "3px 10px",
  borderRadius: 6,
  background: "rgba(20,20,28,0.7)",
  color: "#fff",
  border: "1px solid rgba(255,255,255,0.12)",
  cursor: "pointer",
};

const feedbackBtn: React.CSSProperties = {
  width: 26,
  height: 24,
  padding: 0,
  borderRadius: 6,
  color: "#fff",
  border: "1px solid rgba(255,255,255,0.12)",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
};

interface Props {
  text: string | null;
  route?: "local" | "cloud" | "skill" | "acp" | null;
  thinking?: boolean;
  userEcho?: string | null;
  imageBase64?: string | null;
  imageSavePath?: string | null;
  imageStatus?: "generating" | "done" | "error" | null;
  imageError?: string | null;
  onSaveImage?: () => void;
  onCopyImage?: () => void;
  onCancelImage?: () => void;
  /**
   * Phase-1 feedback hook. When provided and the bubble shows a final
   * (non-thinking) assistant reply, two thumbs buttons appear underneath.
   * The parent is responsible for calling [`feedbackRecord`] with the
   * captured prompt/response pair — the bubble only emits +1 / -1.
   */
  onFeedback?: (rating: 1 | -1) => void;
  /**
   * Stable id for the current turn. When this changes, the local
   * "already rated" lock resets so a new reply can be rated again.
   */
  feedbackKey?: string | number | null;
}

// Minimal markdown renderer: triple-backtick fenced code blocks (with
// optional language), inline `code`, and **bold**. Anything else passes
// through as plain text with whitespace preserved.
function renderMarkdown(src: string): JSX.Element[] {
  const out: JSX.Element[] = [];
  const fence = /```([a-zA-Z0-9_+-]*)\n([\s\S]*?)```/g;
  let lastIndex = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  while ((m = fence.exec(src)) !== null) {
    if (m.index > lastIndex) {
      out.push(
        <span key={key++}>{renderInline(src.slice(lastIndex, m.index), key)}</span>
      );
      key += 100;
    }
    const lang = m[1] || "";
    const code = m[2].replace(/\n+$/, "");
    out.push(
      <pre
        key={key++}
        style={{
          margin: "6px 0",
          padding: "8px 10px",
          borderRadius: 8,
          background: "rgba(0,0,0,0.45)",
          border: "1px solid rgba(255,255,255,0.08)",
          fontFamily:
            "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
          fontSize: 12,
          lineHeight: 1.45,
          overflowX: "auto",
          whiteSpace: "pre",
        }}
      >
        {lang && (
          <div
            style={{
              fontSize: 9,
              opacity: 0.55,
              letterSpacing: 0.4,
              textTransform: "uppercase",
              marginBottom: 4,
            }}
          >
            {lang}
          </div>
        )}
        <code>{code}</code>
      </pre>
    );
    lastIndex = m.index + m[0].length;
  }
  if (lastIndex < src.length) {
    out.push(<span key={key++}>{renderInline(src.slice(lastIndex), key)}</span>);
  }
  return out;
}

function renderInline(src: string, baseKey: number): JSX.Element[] {
  const out: JSX.Element[] = [];
  // Tokenize on inline backticks and **bold**.
  const re = /`([^`\n]+)`|\*\*([^*\n]+)\*\*/g;
  let last = 0;
  let k = baseKey;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (m.index > last) {
      out.push(<span key={k++}>{src.slice(last, m.index)}</span>);
    }
    if (m[1] !== undefined) {
      out.push(
        <code
          key={k++}
          style={{
            padding: "1px 5px",
            borderRadius: 4,
            background: "rgba(0,0,0,0.4)",
            fontFamily:
              "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
            fontSize: 12.5,
          }}
        >
          {m[1]}
        </code>
      );
    } else if (m[2] !== undefined) {
      out.push(
        <strong key={k++}>{m[2]}</strong>
      );
    }
    last = m.index + m[0].length;
  }
  if (last < src.length) out.push(<span key={k++}>{src.slice(last)}</span>);
  return out;
}

export default function ChatBubble({
  text,
  route,
  thinking,
  userEcho,
  imageBase64,
  imageSavePath,
  imageStatus,
  imageError,
  onSaveImage,
  onCopyImage,
  onCancelImage,
  onFeedback,
  feedbackKey,
}: Props) {
  useLocale();
  // Local lock: only allow one rating per turn (resets when feedbackKey
  // changes \u2014 a new reply is a new chance to rate).
  const [rated, setRated] = useState<1 | -1 | null>(null);
  useEffect(() => {
    setRated(null);
  }, [feedbackKey]);
  const handleRate = (r: 1 | -1) => {
    if (rated || !onFeedback) return;
    setRated(r);
    try {
      onFeedback(r);
    } catch {
      /* swallow */
    }
  };
  const showFeedback =
    !!onFeedback && !!text && !thinking && !!route && route !== "skill";
  const show =
    !!text ||
    !!thinking ||
    !!userEcho ||
    !!imageBase64 ||
    imageStatus === "generating" ||
    imageStatus === "error";
  return (
    <AnimatePresence>
      {show && (
        <motion.div
          key="bubble"
          className="interactive"
          initial={{ opacity: 0, y: 8, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 4, scale: 0.98 }}
          transition={{ duration: 0.18, ease: "easeOut" }}
          style={{
            position: "absolute",
            top: 44,
            left: 12,
            right: 12,
            zIndex: 10,
            padding: "10px 14px",
            borderRadius: 14,
            background: "rgba(20, 20, 28, 0.88)",
            color: "#fff",
            fontSize: 14,
            lineHeight: 1.4,
            backdropFilter: "blur(10px)",
            WebkitBackdropFilter: "blur(10px)",
            boxShadow: "0 6px 24px rgba(0,0,0,0.45)",
            border: "1px solid rgba(255,255,255,0.1)",
            whiteSpace: "pre-wrap",
            maxHeight: "45vh",
            overflowY: "auto",
          }}
        >
          {route && (
            <div
              style={{
                fontSize: 10,
                opacity: 0.65,
                letterSpacing: 0.5,
                textTransform: "uppercase",
                marginBottom: 4,
              }}
            >
              {route}
            </div>
          )}
          {userEcho && (
            <div
              style={{
                fontSize: 12,
                opacity: 0.85,
                padding: "4px 8px",
                borderRadius: 8,
                background: "rgba(255,255,255,0.06)",
                border: "1px solid rgba(255,255,255,0.12)",
                marginBottom: text || thinking ? 8 : 0,
              }}
            >
              <span style={{ opacity: 0.6, marginRight: 4 }}>{t("bubble.user")}</span>
              {userEcho}
            </div>
          )}
          {thinking && !text ? (
            <span style={{ opacity: 0.7 }}>…</span>
          ) : text ? (
            <div>{renderMarkdown(text)}</div>
          ) : null}
          {showFeedback && (
            <div
              style={{
                display: "flex",
                gap: 6,
                marginTop: 8,
                alignItems: "center",
              }}
            >
              <button
                type="button"
                onClick={() => handleRate(1)}
                disabled={!!rated}
                title={t("bubble.feedback.up")}
                aria-label={t("bubble.feedback.up")}
                style={{
                  ...feedbackBtn,
                  background:
                    rated === 1
                      ? "rgba(120,200,140,0.22)"
                      : "rgba(255,255,255,0.05)",
                  borderColor:
                    rated === 1
                      ? "rgba(160,220,170,0.45)"
                      : "rgba(255,255,255,0.12)",
                  cursor: rated ? "default" : "pointer",
                  opacity: rated && rated !== 1 ? 0.4 : 1,
                }}
              >
                <ThumbUpIcon size={13} />
              </button>
              <button
                type="button"
                onClick={() => handleRate(-1)}
                disabled={!!rated}
                title={t("bubble.feedback.down")}
                aria-label={t("bubble.feedback.down")}
                style={{
                  ...feedbackBtn,
                  background:
                    rated === -1
                      ? "rgba(220,140,140,0.22)"
                      : "rgba(255,255,255,0.05)",
                  borderColor:
                    rated === -1
                      ? "rgba(240,170,170,0.45)"
                      : "rgba(255,255,255,0.12)",
                  cursor: rated ? "default" : "pointer",
                  opacity: rated && rated !== -1 ? 0.4 : 1,
                }}
              >
                <ThumbDownIcon size={13} />
              </button>
              {rated && (
                <span style={{ fontSize: 11, opacity: 0.6 }}>
                  {t("bubble.feedback.thanks")}
                </span>
              )}
            </div>
          )}
          {imageStatus === "generating" && !imageBase64 && (
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: text ? 8 : 0 }}>
              <span style={{ opacity: 0.75 }}>{t("bubble.image.generating")}</span>
              {onCancelImage && (
                <button
                  onClick={onCancelImage}
                  style={{
                    fontSize: 11,
                    padding: "2px 8px",
                    borderRadius: 6,
                    background: "rgba(255,255,255,0.08)",
                    color: "#fff",
                    border: "1px solid rgba(255,255,255,0.18)",
                    cursor: "pointer",
                  }}
                >
                  {t("bubble.image.cancel")}
                </button>
              )}
            </div>
          )}
          {imageStatus === "error" && imageError && (
            <div style={{ marginTop: 6, color: "#ff8080", fontSize: 12 }}>
              {t("bubble.image.error")} {imageError}
            </div>
          )}
          {imageBase64 && (
            <div style={{ marginTop: text || userEcho ? 8 : 0 }}>
              <img
                src={`data:image/png;base64,${imageBase64}`}
                alt="generated"
                style={{
                  maxWidth: "100%",
                  borderRadius: 10,
                  border: "1px solid rgba(255,255,255,0.12)",
                  display: "block",
                }}
              />
              <div style={{ display: "flex", gap: 6, marginTop: 6, flexWrap: "wrap" }}>
                {onSaveImage && (
                  <button onClick={onSaveImage} style={imgButtonStyle}>
                    {t("bubble.image.save_as")}
                  </button>
                )}
                {onCopyImage && (
                  <button onClick={onCopyImage} style={imgButtonStyle}>
                    {t("bubble.image.copy")}
                  </button>
                )}
                {imageSavePath && (
                  <span style={{ fontSize: 10, opacity: 0.55, alignSelf: "center" }}>
                    {t("bubble.image.saved")} {imageSavePath}
                  </span>
                )}
              </div>
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
