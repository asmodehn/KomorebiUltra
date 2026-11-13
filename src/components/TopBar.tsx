import { useLocale, t, type DictKey } from "../i18n";
import {
  STAGE_LABELS,
  type RelationshipState,
  type WindowOverlayMode,
} from "../api";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  CloseIcon,
  DownloadIcon,
  EarIcon,
  EyeIcon,
  GearIcon,
  GripIcon,
  LayersIcon,
  RefreshIcon,
  WarningIcon,
} from "./icons";

const OVERLAY_LABEL_KEYS: Record<WindowOverlayMode, DictKey> = {
  normal: "topbar.overlay.normal",
  overlay: "topbar.overlay.overlay",
};

export interface TopBarProps {
  mode: string;
  hasKey: boolean;
  listenEnabled: boolean;
  listenReady: boolean;
  listening: boolean;
  heard: boolean;
  onToggleListen: () => void;
  onToggleSettings: () => void;
  onToggleWizard: () => void;
  onReset: () => void;
  onQuit: () => void;
  autoWatch: boolean;
  autoWatchAvailable: boolean;
  onToggleAutoWatch: () => void;
  relationship: RelationshipState | null;
  showRelationshipBadge: boolean;
  overlayMode: WindowOverlayMode;
  onToggleOverlay: () => void;
  /** Leave overlay mode outright (back to a plain, movable window). */
  onExitOverlay: () => void;
}

/**
 * Floating top-right control strip: mode badge, relationship badge,
 * listen toggle, auto-watch toggle, reset / downloads / settings / quit.
 *
 * `props.listenEnabled` is unused by the UI directly today — the actual
 * styling reads `props.listening` (true while the mic stream is live) —
 * but it's kept on the prop type so callers can still flow it through
 * for future "want vs is" indicators without API churn.
 */
export function TopBar(props: TopBarProps) {
  useLocale(); // re-render on language change
  void props.listenEnabled;
  return (
    <div
      className="interactive"
      style={{
        position: "absolute",
        top: 8,
        right: 8,
        zIndex: 10,
        display: "flex",
        gap: 6,
        alignItems: "center",
        fontSize: 11,
        color: "#fff",
      }}
    >
      {/* Window move grip: in overlay mode the avatar no longer drags the
         window, so the top bar keeps an explicit handle. Clicking it while
         in overlay mode first leaves overlay — a layer surface cannot be
         moved by the client anyway — so the user gets a real window to drag. */}
      <button
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          if (props.overlayMode === "overlay") {
            void props.onExitOverlay();
            return;
          }
          void getCurrentWindow().startDragging();
        }}
        style={{ ...iconBtn, cursor: "move", touchAction: "none" }}
        title={t("topbar.move")}
        aria-label={t("topbar.move")}
      >
        <GripIcon size={13} />
      </button>
      <span
        style={{
          padding: "3px 8px",
          borderRadius: 8,
          background: "rgba(20,20,28,0.7)",
          opacity: 0.9,
          textTransform: "uppercase",
          letterSpacing: 0.5,
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
        }}
        title={props.hasKey ? t("topbar.key_saved") : t("topbar.no_key")}
      >
        <span>{props.mode}</span>
        {!props.hasKey && props.mode !== "local" && (
          <WarningIcon size={11} style={{ opacity: 0.85 }} />
        )}
      </span>
      {props.showRelationshipBadge && props.relationship && (
        <RelationshipBadge state={props.relationship} />
      )}
      {/* Overlay toggle — Alt+O and Escape (composer closed) do the same.
         Icon stays constant; the green dot marks the fullscreen overlay. */}
      <button
        onClick={props.onToggleOverlay}
        style={iconBtnStyle(props.overlayMode === "overlay")}
        title={`${t("topbar.overlay")}: ${t(
          OVERLAY_LABEL_KEYS[props.overlayMode],
        )} (Alt+O)`}
        aria-label={`${t("topbar.overlay")} ${t(OVERLAY_LABEL_KEYS[props.overlayMode])}`}
      >
        <LayersIcon size={14} />
        {props.overlayMode === "overlay" && (
          <span
            style={{
              position: "absolute",
              top: 3,
              right: 3,
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "#8ef0a8",
              boxShadow: "0 0 6px rgba(142,240,168,0.85)",
            }}
          />
        )}
      </button>
      <button
        onClick={props.onToggleListen}
        disabled={!props.listenReady}
        style={iconBtnStyle(props.listening)}
        title={
          !props.listenReady
            ? t("topbar.listen.tip_setup")
            : props.listening
              ? t("topbar.listen.tip_listening")
              : t("topbar.listen.tip_idle")
        }
        aria-label={t("topbar.listen.tip_idle")}
      >
        <EarIcon size={14} />
        {props.heard && (
          <span
            style={{
              position: "absolute",
              top: 3,
              right: 3,
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "#7ec6ff",
              boxShadow: "0 0 6px rgba(126,198,255,0.85)",
            }}
          />
        )}
      </button>
      <button
        onClick={props.onToggleAutoWatch}
        disabled={!props.autoWatchAvailable}
        style={iconBtnStyle(props.autoWatch)}
        title={
          !props.autoWatchAvailable
            ? t("topbar.watch.tip_setup")
            : props.autoWatch
              ? t("topbar.watch.tip_on")
              : t("topbar.watch.tip_off")
        }
        aria-label={t("topbar.watch.tip_off")}
      >
        <EyeIcon size={14} />
      </button>
      <button
        onClick={props.onReset}
        style={iconBtn}
        title={t("topbar.reset")}
        aria-label={t("topbar.reset")}
      >
        <RefreshIcon size={13} />
      </button>
      <button
        onClick={props.onToggleWizard}
        style={iconBtn}
        title={t("topbar.downloads")}
        aria-label={t("topbar.downloads")}
      >
        <DownloadIcon size={13} />
      </button>
      <button
        onClick={props.onToggleSettings}
        style={iconBtn}
        title={t("topbar.settings")}
        aria-label={t("topbar.settings")}
      >
        <GearIcon size={13} />
      </button>
      <button
        onClick={props.onQuit}
        style={iconBtn}
        title={t("topbar.quit")}
        aria-label={t("topbar.quit")}
      >
        <CloseIcon size={13} />
      </button>
    </div>
  );
}

function RelationshipBadge({ state }: { state: RelationshipState }) {
  useLocale();
  const meta = STAGE_LABELS[state.stage];
  const stages = Object.keys(STAGE_LABELS) as (keyof typeof STAGE_LABELS)[];
  const idx = stages.indexOf(state.stage);
  const thresholds = [0, 50, 150, 300, 500, 750, 1000];
  const lo = thresholds[idx] ?? 0;
  const hi = thresholds[idx + 1] ?? state.score + 1;
  const pct = Math.max(0, Math.min(1, (state.score - lo) / (hi - lo))) * 100;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const stageName = t(`stage.${state.stage}` as any);
  return (
    <span
      title={`${stageName} · ${t("rel.score", { score: state.score })} · ${t("rel.interactions", { count: state.total_interactions, streak: state.daily_streak })}`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        padding: "3px 8px",
        borderRadius: 8,
        background: "rgba(20,20,28,0.7)",
        fontSize: 11,
      }}
    >
      <span>{meta.emoji}</span>
      <span style={{ opacity: 0.95 }}>{stageName}</span>
      <span
        style={{
          width: 28,
          height: 4,
          borderRadius: 2,
          background: "rgba(255,255,255,0.18)",
          position: "relative",
          overflow: "hidden",
        }}
      >
        <span
          style={{
            position: "absolute",
            inset: 0,
            width: `${pct}%`,
            background: "rgba(255,255,255,0.55)",
          }}
        />
      </span>
    </span>
  );
}

const iconBtn: React.CSSProperties = {
  width: 26,
  height: 26,
  borderRadius: 8,
  border: "1px solid rgba(255,255,255,0.10)",
  background: "rgba(20,20,28,0.7)",
  color: "#fff",
  cursor: "pointer",
  fontSize: 13,
  lineHeight: 1,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  position: "relative",
  padding: 0,
};

/** Same neutral icon button, with a subtle white border when "active". */
function iconBtnStyle(active: boolean): React.CSSProperties {
  return {
    ...iconBtn,
    border: active
      ? "1px solid rgba(255,255,255,0.45)"
      : iconBtn.border,
    background: active ? "rgba(255,255,255,0.10)" : iconBtn.background,
  };
}

export default TopBar;
