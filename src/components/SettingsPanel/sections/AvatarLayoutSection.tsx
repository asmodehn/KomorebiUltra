// Avatar zoom + offset live controls. The avatar is rendered via PIXI;
// these sliders push directly into the running scene without recreating
// the model, so dragging gives instant visual feedback.

import { useEffect, useState } from "react";
import {
  setAvatarOffset,
  setAvatarZoom,
  type PublicSettings,
} from "../../../api";
import { t, useLocale } from "../../../i18n";
import Slider from "../lib/Slider";
import { subCardStyle } from "../styles";

interface Props {
  settings: PublicSettings | null;
  refresh: () => Promise<void>;
}

export default function AvatarLayoutSection({ settings, refresh }: Props) {
  useLocale();
  const [zoom, setZoom] = useState<number>(settings?.avatar_zoom ?? 1);
  const [ox, setOx] = useState<number>(settings?.avatar_offset_x ?? 0);
  const [oy, setOy] = useState<number>(settings?.avatar_offset_y ?? 0);

  useEffect(() => {
    setZoom(settings?.avatar_zoom ?? 1);
    setOx(settings?.avatar_offset_x ?? 0);
    setOy(settings?.avatar_offset_y ?? 0);
  }, [
    settings?.avatar_zoom,
    settings?.avatar_offset_x,
    settings?.avatar_offset_y,
  ]);

  // Persist on commit (pointer up). Live preview is driven by local state
  // which flows back through getSettings → AvatarStage prop.
  const commitZoom = async (v: number) => {
    await setAvatarZoom(v);
    await refresh();
  };
  const commitOffset = async (x: number, y: number) => {
    await setAvatarOffset(x, y);
    await refresh();
  };

  return (
    <div style={subCardStyle}>
      <div style={{ opacity: 0.7, fontSize: 11, marginBottom: 4 }}>
        {t("settings.avatar.title")}
      </div>
      <Slider
        label={t("settings.avatar.zoom", { value: zoom.toFixed(2) })}
        min={0.3}
        max={2.5}
        step={0.05}
        value={zoom}
        onChange={(v) => {
          setZoom(v);
          void commitZoom(v);
        }}
        onCommit={() => void commitZoom(zoom)}
      />
      <Slider
        label={t("settings.avatar.offset_x", {
          value: `${ox >= 0 ? "+" : ""}${ox.toFixed(2)}`,
        })}
        min={-2}
        max={2}
        step={0.02}
        value={ox}
        onChange={(v) => {
          setOx(v);
          void commitOffset(v, oy);
        }}
        onCommit={() => void commitOffset(ox, oy)}
      />
      <Slider
        label={t("settings.avatar.offset_y", {
          value: `${oy >= 0 ? "+" : ""}${oy.toFixed(2)}`,
        })}
        min={-2}
        max={2}
        step={0.02}
        value={oy}
        onChange={(v) => {
          setOy(v);
          void commitOffset(ox, v);
        }}
        onCommit={() => void commitOffset(ox, oy)}
      />
      <button
        onClick={() => {
          setZoom(1);
          setOx(0);
          setOy(0);
          void commitZoom(1);
          void commitOffset(0, 0);
        }}
        style={{
          marginTop: 6,
          padding: "4px 10px",
          fontSize: 11,
          borderRadius: 4,
          border: "1px solid rgba(255,255,255,0.15)",
          background: "rgba(255,255,255,0.06)",
          color: "#fff",
          cursor: "pointer",
        }}
      >
        {t("settings.avatar.reset")}
      </button>
      <div style={{ opacity: 0.5, fontSize: 11, marginTop: 4 }}>
        {t("settings.avatar.reset_hint")}
      </div>
    </div>
  );
}
