import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// Dev-only: push a compact UI snapshot for the agent frame harness
// (`src-tauri/src/agent_frame.rs`). Tree-shaken out of production builds.
if (import.meta.env.DEV) {
  void import("./agentFrame").then((m) => m.startAgentFrames());
}

// NOTE: React.StrictMode is intentionally OFF. In dev it double-mounts effects,
// which double-subscribes to Tauri's `tts:play` event and caused overlapping
// audio playback ("flanger/echo/unintelligible" TTS).
ReactDOM.createRoot(document.getElementById("root")!).render(<App />);

