import { useEffect, type MutableRefObject } from "react";
import { avatarState } from "../avatarState";
import { stripMoodTags } from "../emotion";
import { onChat, type ChatEvent, type PublicSettings } from "../api";

export type Route = "local" | "cloud" | "skill";

export interface LastTurn {
  id: string | number;
  prompt: string;
  response: string;
  route: Route;
  modelLabel: string;
}

/**
 * Bridges the backend `chat:*` event stream onto the visible bubble +
 * route badge + emotion engine, and freezes each completed turn into
 * `lastTurnRef` for the feedback widget.
 *
 * `activeIdRef` is the source of truth for "which chat turn is currently
 * streaming" — it's a sentinel `"pending"` between submit and the
 * backend's first event, then the real id once `started` fires.
 */
export function useChatStream(opts: {
  activeIdRef: MutableRefObject<string | null>;
  lastTurnRef: MutableRefObject<LastTurn | null>;
  settingsRef: MutableRefObject<PublicSettings | null>;
  rawTextRef: MutableRefObject<string>;
  setRoute: (r: Route | null) => void;
  setBubbleText: (text: string | null) => void;
  setThinking: (v: boolean) => void;
  setFeedbackKey: (updater: (k: number) => number) => void;
  scheduleBubbleHide: (ms?: number) => void;
}): void {
  const {
    activeIdRef,
    lastTurnRef,
    settingsRef,
    rawTextRef,
    setRoute,
    setBubbleText,
    setThinking,
    setFeedbackKey,
    scheduleBubbleHide,
  } = opts;

  useEffect(() => {
    const p = onChat((e: ChatEvent) => {
      // Race-free id matching: the backend starts emitting events on a
      // spawned task before `sendMessage` returns the id. Reserve the slot
      // with the sentinel "pending" on submit; the first event adopts its
      // real id.
      if (activeIdRef.current === "pending") activeIdRef.current = e.id;
      if (e.id !== activeIdRef.current) return;
      switch (e.kind) {
        case "started":
          setRoute(e.route);
          setBubbleText("");
          rawTextRef.current = "";
          setThinking(true);
          // Open a fresh feedback turn. Prompt was stashed in `lastTurnRef`
          // by handleSubmit; record route + model_label here when known.
          if (lastTurnRef.current) {
            const s = settingsRef.current;
            const modelLabel =
              e.route === "cloud"
                ? s?.acp_agent_command
                  ? `acp:${s.acp_agent_command.trim().split(/\s+/)[0].split(/[\\/]/).pop()}`
                  : `openrouter:${s?.openrouter_model ?? "?"}`
                : e.route === "local"
                  ? `local:${(s?.local_model_path ?? "").split(/[\\/]/).pop() || "?"}`
                  : `skill:${e.route}`;
            lastTurnRef.current = {
              ...lastTurnRef.current,
              id: e.id,
              route: e.route,
              modelLabel,
              response: "",
            };
          }
          break;
        case "token":
          setThinking(false);
          rawTextRef.current += e.text;
          avatarState.onToken(rawTextRef.current);
          setBubbleText(stripMoodTags(rawTextRef.current));
          break;
        case "done":
          setThinking(false);
          avatarState.onDone();
          // Freeze the response text for feedback before scheduling hide.
          if (lastTurnRef.current && lastTurnRef.current.id === e.id) {
            lastTurnRef.current.response = stripMoodTags(rawTextRef.current);
            setFeedbackKey((k) => k + 1);
          }
          scheduleBubbleHide();
          activeIdRef.current = null;
          rawTextRef.current = "";
          break;
        case "error":
          setThinking(false);
          avatarState.onDone(500);
          setBubbleText(`⚠ ${e.message}`);
          scheduleBubbleHide(6000);
          activeIdRef.current = null;
          rawTextRef.current = "";
          // Cancel the pending feedback turn — no usable reply.
          lastTurnRef.current = null;
          break;
      }
    });
    return () => {
      p.then((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
