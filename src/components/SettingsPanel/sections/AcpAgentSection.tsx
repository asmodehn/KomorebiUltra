// ACP agent: when a command is set, Cloud-routed chat turns go to that
// Agent Client Protocol agent (e.g. claude-agent-acp) instead of
// OpenRouter. Classifier, coach and vision calls stay on OpenRouter.

import { useEffect, useState } from "react";
import { setAcpAgentCommand, type PublicSettings } from "../../../api";
import { t, useLocale } from "../../../i18n";
import { inputStyle } from "../styles";

interface Props {
  settings: PublicSettings | null;
  refresh: () => Promise<void>;
}

export default function AcpAgentSection({ settings, refresh }: Props) {
  useLocale();
  const [command, setCommand] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setCommand(settings?.acp_agent_command ?? "");
  }, [settings?.acp_agent_command]);

  const save = async (value: string) => {
    setBusy(true);
    try {
      await setAcpAgentCommand(value);
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <div style={{ opacity: 0.7, marginBottom: 6 }}>
        {t("settings.acp.label")}{" "}
        {settings?.acp_agent_command && (
          <span style={{ color: "#a5d6a7" }}>{t("settings.status.on")}</span>
        )}
      </div>
      <input
        type="text"
        placeholder="claude-agent-acp"
        value={command}
        disabled={busy}
        onChange={(e) => setCommand(e.target.value)}
        onBlur={() =>
          command !== (settings?.acp_agent_command ?? "") && save(command)
        }
        style={inputStyle}
      />
      <div style={{ opacity: 0.5, fontSize: 11, marginTop: 6 }}>
        {t("settings.acp.hint")}
      </div>
    </div>
  );
}
