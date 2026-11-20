// Library-agnostic helpers for poking at a pixi-live2d-display model.
//
// All these wrap optional methods that may or may not exist depending on
// the loaded runtime (Cubism 2 vs 4/5) and the model's own definition.
// Every helper returns silently on a miss so callers don't need to guard.

export interface ExpressiveModel {
  expression?: (name: string) => unknown;
  motion?: (group: string, index?: number, priority?: number) => unknown;
  internalModel?: {
    motionManager?: {
      definitions?: unknown;
      motionGroups?: unknown;
      settings?: unknown;
    };
  };
}

export async function tryMotion(
  model: unknown,
  group: string,
  index?: number,
  priority = 2,
): Promise<boolean> {
  const m = model as ExpressiveModel;
  if (typeof m.motion !== "function") return false;
  try {
    // Priority 2 = NORMAL in pixi-live2d-display; 3 = FORCE, which
    // interrupts a running motion. The library resolves its Promise
    // false for a missing group or a rejected priority, so the result
    // must be awaited before a caller decides whether to fall back.
    return !!(await m.motion(group, index, priority));
  } catch {
    /* no such motion group — ignore */
    return false;
  }
}

export async function tryMotionAny(
  model: unknown,
  groups: string[],
  priority = 2,
): Promise<boolean> {
  for (const g of groups) {
    if (await tryMotion(model, g, undefined, priority)) return true;
  }
  return false;
}

export function tryExpression(model: unknown, name: string): boolean {
  const m = model as ExpressiveModel;
  if (typeof m.expression !== "function") return false;
  try {
    const r = m.expression(name);
    // pixi-live2d-display returns false / a Promise<boolean> / undefined.
    if (r === false) return false;
    return true;
  } catch {
    return false;
  }
}

/**
 * Read the expression names declared in the loaded `.model3.json`
 * (Cubism 4/5) or `.model.json` (Cubism 2). Used so we only call
 * `model.expression(name)` with names that actually exist — the library
 * returns a Promise that may resolve to `false` for missing names, so we
 * can't reliably probe by trying.
 */
export function listExpressionNames(model: unknown): string[] {
  try {
    const settings = (
      model as {
        internalModel?: {
          settings?: {
            expressions?: Array<{
              Name?: string;
              name?: string;
              File?: string;
              file?: string;
            }>;
          };
        };
      }
    ).internalModel?.settings;
    const list = settings?.expressions ?? [];
    return list
      .map((e) => e?.Name ?? e?.name ?? "")
      .filter((s): s is string => !!s);
  } catch {
    return [];
  }
}

/**
 * Try a list of candidate expression names and play the first one the
 * loaded model actually defines. Logs the chosen name for debugging.
 */
export function tryExpressionAny(model: unknown, names: string[]): void {
  const available = listExpressionNames(model);
  for (const n of names) {
    if (available.length > 0 && !available.includes(n)) continue;
    if (tryExpression(model, n)) {
      console.log(`[live2d] expression → ${n}`);
      return;
    }
  }
  if (available.length > 0) {
    console.log(
      `[live2d] expression: none of [${names.join(", ")}] found in model (available: [${available.join(", ")}])`,
    );
  }
}

export interface CoreModelLike {
  setParameterValueById?: (id: string, value: number) => void;
  setParamFloat?: (id: string, value: number) => void;
  setParameterValueByIndex?: (index: number, value: number) => void;
  getParameterIndex?: (id: string) => number;
}

export function trySet(
  coreModel: CoreModelLike,
  id: string,
  value: number,
): void {
  try {
    if (typeof coreModel.setParameterValueById === "function") {
      coreModel.setParameterValueById(id, value);
      return;
    }
    // Cubism 2 core exposes `setParamFloat(id, value)` instead.
    if (typeof coreModel.setParamFloat === "function") {
      coreModel.setParamFloat(id, value);
      return;
    }
    // Last resort: index-based API (some Cubism 2 builds).
    if (
      typeof coreModel.getParameterIndex === "function" &&
      typeof coreModel.setParameterValueByIndex === "function"
    ) {
      const idx = coreModel.getParameterIndex(id);
      if (idx >= 0) coreModel.setParameterValueByIndex(idx, value);
    }
  } catch {
    /* parameter missing on this model */
  }
}
