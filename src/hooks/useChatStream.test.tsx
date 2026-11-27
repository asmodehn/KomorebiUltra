import { renderHook, act } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { useRef, useState } from "react";
import type { ChatEvent } from "../api";
import { useChatStream, type LastTurn, type Route } from "./useChatStream";

let chatCb: ((e: ChatEvent) => void) | null = null;

vi.mock("../api", () => ({
  onChat: vi.fn((cb: (e: ChatEvent) => void) => {
    chatCb = cb;
    return Promise.resolve(() => {
      chatCb = null;
    });
  }),
}));

vi.mock("../avatarState", () => ({
  avatarState: {
    setThinking: vi.fn(),
    setTalking: vi.fn(),
    setMood: vi.fn(),
    setIdle: vi.fn(),
    onToken: vi.fn(),
    onDone: vi.fn(),
  },
}));

vi.mock("../emotion", () => ({
  stripMoodTags: (s: string) => s,
}));

function useHarness() {
  const [bubbleText, setBubbleText] = useState("");
  const [thinking, setThinking] = useState(false);
  const [route, setRoute] = useState<Route | null>(null);
  const [feedbackKey, setFeedbackKey] = useState(0);
  const activeIdRef = useRef<string | null>("probe-id");
  const rawTextRef = useRef<string>("");
  const settingsRef = useRef<any>(null);
  const lastTurnRef = useRef<LastTurn | null>({
    id: "probe-id",
    prompt: "probe prompt",
    response: "",
    route: "acp",
    modelLabel: "",
  });
  useChatStream({
    activeIdRef,
    lastTurnRef,
    settingsRef,
    rawTextRef,
    setBubbleText: (text) => setBubbleText(text ?? ""),
    setThinking,
    setRoute,
    setFeedbackKey,
    scheduleBubbleHide: vi.fn(),
  });
  return { bubbleText, thinking, route, activeIdRef, lastTurnRef, rawTextRef, feedbackKey };
}

function emit(e: ChatEvent) {
  if (!chatCb) throw new Error("chat callback not registered");
  act(() => chatCb!(e));
}

describe("useChatStream ACP progress", () => {
  beforeEach(() => {
    chatCb = null;
  });

  it("shows progress display-only and never leaks into reply/feedback state", async () => {
    const { result } = renderHook(() => useHarness());
    // allow effect to register listener
    await act(async () => {});

    emit({ kind: "started", id: "probe-id", route: "acp" });
    expect(result.current.route).toBe("acp");
    expect(result.current.thinking).toBe(true);

    emit({ kind: "progress", id: "probe-id", stage: "thought", text: "PROGRESS-THOUGHT-SENTINEL" });
    expect(result.current.bubbleText).toContain("PROGRESS-THOUGHT-SENTINEL");

    emit({
      kind: "progress",
      id: "probe-id",
      stage: "tool_call",
      tool_id: "call-1",
      title: "Read file",
      status: "pending",
      input: "RAW-INPUT-SENTINEL-DO-NOT-RENDER",
    } as ChatEvent);
    expect(result.current.bubbleText).toContain("Read file");
    expect(result.current.bubbleText).not.toContain("RAW-INPUT-SENTINEL-DO-NOT-RENDER");
    expect(result.current.bubbleText).not.toContain("PROGRESS-THOUGHT-SENTINEL");

    emit({
      kind: "progress",
      id: "probe-id",
      stage: "tool_update",
      tool_id: "call-1",
      status: "completed",
      output: "RAW-OUTPUT-SENTINEL-DO-NOT-RENDER",
    } as ChatEvent);
    expect(result.current.bubbleText).toContain("completed");
    expect(result.current.bubbleText).not.toContain("RAW-OUTPUT-SENTINEL-DO-NOT-RENDER");

    emit({ kind: "progress", id: "other-id", stage: "thought", text: "WRONG-ID-SENTINEL" });
    expect(result.current.bubbleText).not.toContain("WRONG-ID-SENTINEL");
    expect(result.current.bubbleText).toContain("completed");

    emit({ kind: "token", id: "probe-id", text: "Final " });
    emit({ kind: "token", id: "probe-id", text: "answer." });
    expect(result.current.bubbleText).toBe("Final answer.");

    emit({ kind: "done", id: "probe-id", full_text: "Final answer." });
    expect(result.current.thinking).toBe(false);
    expect(result.current.activeIdRef.current).toBeNull();
    expect(result.current.lastTurnRef.current?.response).toBe("Final answer.");
    expect(result.current.lastTurnRef.current?.response).not.toContain("PROGRESS-THOUGHT-SENTINEL");
    expect(result.current.lastTurnRef.current?.response).not.toContain("RAW-INPUT-SENTINEL-DO-NOT-RENDER");
    expect(result.current.lastTurnRef.current?.response).not.toContain("RAW-OUTPUT-SENTINEL-DO-NOT-RENDER");

    emit({ kind: "token", id: "probe-id", text: "LATE-SENTINEL" });
    emit({ kind: "progress", id: "probe-id", stage: "thought", text: "LOG-SENTINEL-THOUGHT" });
    expect(result.current.bubbleText).toBe("Final answer.");
    expect(result.current.lastTurnRef.current?.response).toBe("Final answer.");
  });
});
