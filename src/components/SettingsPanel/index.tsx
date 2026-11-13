// Settings panel orchestrator.
//
// Responsibilities:
//   - Fetch the current PublicSettings snapshot when opened
//   - Provide a single `refresh()` callback (re-fetch + bubble up) that
//     every section calls after mutating state
//   - Compose the full section tree
//
// Every section is a self-contained file under ./sections/. Shared inline
// styles live in ./styles.ts. Reusable bits (Slider, model filter hook,
// helpers) are in ./lib/. See [TZ.md](../../../TZ.md) and the ADRs under
// docs/architecture/ for the broader design context.

import { AnimatePresence, motion } from "framer-motion";
import { useCallback, useEffect, useState } from "react";
import {
  getSettings,
  setMode,
  setOpenRouterKey,
  type Mode,
  type PublicSettings,
} from "../../api";
import { t, useLocale } from "../../i18n";
import { toast } from "../Toast";
import SectionCard from "./lib/SectionCard";
import { btnStyle, cardTitleStyle, inputStyle } from "./styles";
import AvatarLayoutSection from "./sections/AvatarLayoutSection";
import GameCoachSection from "./sections/GameCoachSection";
import HardwareSection from "./sections/HardwareSection";
import ImageGenSection from "./sections/ImageGenSection";
import IntentSection from "./sections/IntentSection";
import Live2DSection from "./sections/Live2DSection";
import OpenRouterModelSection from "./sections/OpenRouterModelSection";
import RagSection from "./sections/RagSection";
import RelationshipSection from "./sections/RelationshipSection";
import SmartRoutingSection from "./sections/SmartRoutingSection";
import AcpAgentSection from "./sections/AcpAgentSection";
import WakeWordSection from "./sections/WakeWordSection";
import CommunitySection from "./sections/community/CommunitySection";
import TrainingSection from "./sections/community/TrainingSection";
import LanguageSection from "./sections/locale/LanguageSection";
import WeatherSection from "./sections/locale/WeatherSection";
import SttSection from "./sections/stt/SttSection";
import TtsSection from "./sections/voice/TtsSection";

interface Props {
  open: boolean;
  onClose: () => void;
  onChanged: () => void;
}

/// Two-level disclosure: most users only need a small subset of
/// settings ("enter API key, toggle game coach, pick avatar"), while
/// advanced users want fine-grained controls (local model paths, RAG
/// chunking, hardware overrides). Splitting them into tabs keeps the
/// novice path uncluttered without hiding power features behind a
/// dev-mode toggle. Persisted in localStorage so the user lands on
/// whichever tab they last used.
type Tab = "basic" | "advanced";
const TAB_STORAGE_KEY = "settings.tab";

function loadTab(): Tab {
  try {
    const v = localStorage.getItem(TAB_STORAGE_KEY);
    return v === "advanced" ? "advanced" : "basic";
  } catch {
    return "basic";
  }
}

export default function SettingsPanel({ open, onClose, onChanged }: Props) {
  useLocale();
  const [settings, setSettings] = useState<PublicSettings | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [tab, setTab] = useState<Tab>(loadTab);

  useEffect(() => {
    if (open) {
      getSettings().then(setSettings);
      setKeyInput("");
    }
  }, [open]);

  useEffect(() => {
    try {
      localStorage.setItem(TAB_STORAGE_KEY, tab);
    } catch {
      /* localStorage unavailable — fine, just don't persist */
    }
  }, [tab]);

  // Single refresh callback shared by every section. Replaces the 11 inline
  // copies of this same closure that lived in the legacy monolith.
  const refresh = useCallback(async () => {
    const next = await getSettings();
    setSettings(next);
    onChanged();
  }, [onChanged]);

  const onModeChange = async (mode: Mode) => {
    await setMode(mode);
    await refresh();
  };

  const onSaveKey = async () => {
    if (!keyInput.trim()) return;
    setSaving(true);
    try {
      await setOpenRouterKey(keyInput.trim());
      setKeyInput("");
      await refresh();
      toast.success(t("settings.status.saved"));
    } catch (err) {
      toast.error(String(err));
    } finally {
      setSaving(false);
    }
  };

  const onClearKey = async () => {
    try {
      await setOpenRouterKey("");
      await refresh();
      toast.info(t("common.clear"));
    } catch (err) {
      toast.error(String(err));
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          key="settings"
          className="interactive"
          initial={{ opacity: 0, y: -6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -4 }}
          transition={{ duration: 0.16 }}
          onKeyDown={(e) => e.key === "Escape" && onClose()}
          style={{
            position: "absolute",
            top: 48,
            left: 12,
            right: 12,
            bottom: 12,
            padding: 0,
            borderRadius: 14,
            background: "rgba(18, 18, 26, 0.92)",
            color: "#fff",
            fontSize: 13,
            backdropFilter: "blur(14px)",
            WebkitBackdropFilter: "blur(14px)",
            boxShadow: "0 8px 28px rgba(0,0,0,0.5)",
            border: "1px solid rgba(255,255,255,0.08)",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          }}
        >
          <Header onClose={onClose} />
          <Tabs tab={tab} setTab={setTab} />
          <div
            style={{
              overflowY: "auto",
              padding: 14,
              display: "flex",
              flexDirection: "column",
              gap: 10,
              flex: 1,
              minHeight: 0,
            }}
          >
            {/* Mode + API key always visible at the top of either tab —
                they're the prerequisite for everything else and should
                never be hidden behind a "switch to advanced" click. */}
            <SectionCard>
              <ModeSection
                mode={settings?.mode as Mode | undefined}
                onChange={onModeChange}
              />
            </SectionCard>
            <SectionCard>
              <ApiKeySection
                settings={settings}
                keyInput={keyInput}
                setKeyInput={setKeyInput}
                saving={saving}
                onSaveKey={onSaveKey}
                onClearKey={onClearKey}
              />
            </SectionCard>

            {tab === "basic" ? (
              <>
                <SectionCard>
                  <AcpAgentSection settings={settings} refresh={refresh} />
                </SectionCard>
                {/* Pick a model the OpenRouter key applies to. */}
                <SectionCard>
                  <OpenRouterModelSection settings={settings} refresh={refresh} />
                </SectionCard>
                {/* Voice in/out — these have user-facing provider toggles
                    so they belong in basic even though local options
                    exist (Piper, faster-whisper). */}
                <SectionCard>
                  <TtsSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <SttSection settings={settings} refresh={refresh} />
                </SectionCard>
                {/* Cloud routing controls. */}
                <SectionCard>
                  <SmartRoutingSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <GameCoachSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <ImageGenSection settings={settings} refresh={refresh} />
                </SectionCard>
                {/* Avatar customisation. */}
                <SectionCard>
                  <Live2DSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <AvatarLayoutSection settings={settings} refresh={refresh} />
                </SectionCard>
                {/* Personalisation. */}
                <SectionCard>
                  <LanguageSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <WeatherSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <RelationshipSection settings={settings} refresh={refresh} />
                </SectionCard>
              </>
            ) : (
              <>
                {/* Power-user features: local models, hardware tuning,
                    retrieval-augmented generation, training tools. */}
                <SectionCard>
                  <HardwareSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <IntentSection />
                </SectionCard>
                <SectionCard>
                  <RagSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <WakeWordSection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <CommunitySection settings={settings} refresh={refresh} />
                </SectionCard>
                <SectionCard>
                  <TrainingSection settings={settings} refresh={refresh} />
                </SectionCard>
              </>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function Header({ onClose }: { onClose: () => void }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "10px 14px",
        borderBottom: "1px solid rgba(255,255,255,0.06)",
        flexShrink: 0,
      }}
    >
      <strong style={{ fontSize: 14 }}>{t("settings.title")}</strong>
      <button
        onClick={onClose}
        style={{
          background: "transparent",
          border: "none",
          color: "#aaa",
          cursor: "pointer",
          fontSize: 18,
          lineHeight: 1,
          padding: 0,
          width: 24,
          height: 24,
          borderRadius: 6,
        }}
        aria-label={t("common.close")}
      >
        ×
      </button>
    </div>
  );
}

function Tabs({ tab, setTab }: { tab: Tab; setTab: (t: Tab) => void }) {
  const tabs: ReadonlyArray<{ id: Tab; label: string; hint: string }> = [
    {
      id: "basic",
      label: t("settings.tab.basic"),
      hint: t("settings.tab.basic.hint"),
    },
    {
      id: "advanced",
      label: t("settings.tab.advanced"),
      hint: t("settings.tab.advanced.hint"),
    },
  ];
  return (
    <div
      style={{
        display: "flex",
        gap: 4,
        padding: "8px 14px 0 14px",
        borderBottom: "1px solid rgba(255,255,255,0.06)",
        flexShrink: 0,
      }}
    >
      {tabs.map((tDef) => {
        const active = tab === tDef.id;
        return (
          <button
            key={tDef.id}
            onClick={() => setTab(tDef.id)}
            title={tDef.hint}
            style={{
              padding: "8px 14px",
              borderRadius: "8px 8px 0 0",
              border: "1px solid rgba(255,255,255,0.08)",
              borderBottom: active
                ? "1px solid rgba(18,18,26,0.92)"
                : "1px solid rgba(255,255,255,0.06)",
              background: active ? "rgba(40,40,52,0.85)" : "transparent",
              color: active ? "#fff" : "#bbb",
              cursor: "pointer",
              fontSize: 12,
              fontWeight: active ? 600 : 400,
              marginBottom: -1,
            }}
          >
            {tDef.label}
          </button>
        );
      })}
    </div>
  );
}

function ModeSection({
  mode,
  onChange,
}: {
  mode: Mode | undefined;
  onChange: (m: Mode) => void;
}) {
  const modes: readonly Mode[] = ["auto", "local", "cloud"] as const;
  return (
    <div>
      <div style={cardTitleStyle}>{t("settings.routing.title")}</div>
      <div style={{ display: "flex", gap: 6 }}>
        {modes.map((m) => {
          const active = mode === m;
          return (
            <button
              key={m}
              onClick={() => onChange(m)}
              style={{
                ...btnStyle,
                flex: 1,
                padding: "6px 10px",
                border: active
                  ? "1px solid rgba(255,255,255,0.55)"
                  : "1px solid rgba(255,255,255,0.1)",
                background: active ? "rgba(20,20,28,0.7)" : "transparent",
              }}
            >
              {t(`settings.routing.${m}` as const)}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ApiKeySection({
  settings,
  keyInput,
  setKeyInput,
  saving,
  onSaveKey,
  onClearKey,
}: {
  settings: PublicSettings | null;
  keyInput: string;
  setKeyInput: (v: string) => void;
  saving: boolean;
  onSaveKey: () => void;
  onClearKey: () => void;
}) {
  return (
    <div>
      <div style={cardTitleStyle}>
        {t("settings.openrouter.key")}{" "}
        {settings?.has_openrouter_key && (
          <span style={{ color: "#a5d6a7", fontWeight: 400 }}>
            {t("settings.status.saved")}
          </span>
        )}
      </div>
      <div style={{ display: "flex", gap: 6 }}>
        <input
          type="password"
          placeholder={
            settings?.has_openrouter_key
              ? t("settings.openrouter.key.placeholder_saved")
              : t("settings.openrouter.key.placeholder_empty")
          }
          value={keyInput}
          onChange={(e) => setKeyInput(e.target.value)}
          style={{ ...inputStyle, flex: 1 }}
        />
        <button
          disabled={saving || !keyInput.trim()}
          onClick={onSaveKey}
          style={btnStyle}
        >
          {t("common.save")}
        </button>
        {settings?.has_openrouter_key && (
          <button
            onClick={onClearKey}
            style={{
              ...btnStyle,
              background: "transparent",
              color: "#e57373",
            }}
          >
            {t("common.clear")}
          </button>
        )}
      </div>
      <div style={{ opacity: 0.5, fontSize: 11, marginTop: 6 }}>
        {t("settings.openrouter.model_info", {
          model: settings?.openrouter_model ?? "…",
        })}
      </div>
    </div>
  );
}
