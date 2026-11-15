import type { SavedWorkspace } from "../ipc/generated";
import { client, desktopAvailable, messageOf } from "../ipc/client";
import { useStudio } from "./studio";

export const RECOVERY_OPERATION_KEY = "joinery.metadataRecoveryOperation.v1";

function recoveryRemembered() {
  try {
    return typeof window !== "undefined" && sessionStorage.getItem(RECOVERY_OPERATION_KEY) !== null;
  } catch {
    // Storage failure must not let a previously pending restore overwrite its
    // workspace before the recovery UI can reconcile it.
    return true;
  }
}

/** Serialize saves so an older slow request can never replace a newer draft. */
export class WorkspaceWriter {
  private pending: SavedWorkspace | null = null;
  private running: Promise<void> | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private paused = false;
  private generation = 0;
  constructor(
    private write: (workspace: SavedWorkspace) => Promise<void>,
    private onError: (error: unknown) => void,
  ) {}
  schedule(workspace: SavedWorkspace) {
    this.pending = workspace;
    clearTimeout(this.timer);
    if (this.paused) return;
    this.timer = setTimeout(() => {
      void this.flush().catch(this.onError);
    }, 400);
  }
  async flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.running) {
      await this.running;
      if (this.paused) return;
      return this.flush();
    }
    if (this.paused) return;
    this.running = (async () => {
      while (this.pending && !this.paused) {
        const workspace = this.pending;
        const generation = this.generation;
        this.pending = null;
        try {
          await this.write(workspace);
        } catch (error) {
          if (generation === this.generation) this.pending ??= workspace;
          throw error;
        }
      }
    })();
    try {
      await this.running;
    } finally {
      this.running = null;
    }
  }

  async flushDurably(): Promise<void> {
    const requireWritable = () => {
      if (this.paused)
        throw new Error(
          "Workspace saving is paused for metadata recovery. Resolve recovery before closing Joinery; drafts have not been saved.",
        );
    };
    requireWritable();
    await this.flush();
    requireWritable();
  }

  isPaused() {
    return this.paused;
  }

  async pause(): Promise<void> {
    this.paused = true;
    clearTimeout(this.timer);
    await this.running;
  }

  discard() {
    this.generation += 1;
    this.pending = null;
    clearTimeout(this.timer);
  }

  resume() {
    this.paused = false;
  }
}

export function workspaceSnapshot(): SavedWorkspace {
  const state = useStudio.getState();
  const drafts = state.tabs
    .filter(
      (tab) =>
        tab.kind === "query" &&
        tab.fileToken === null &&
        tab.fileName === null &&
        state.bootstrap?.profiles.some(
          (profile) => profile.id === tab.connectionId && profile.storeDrafts,
        ),
    )
    .map((tab) => ({
      id: tab.id,
      connectionId: tab.connectionId!,
      title: tab.title,
      sql: tab.sql,
      ...(tab.pinned ? { pinned: true } : {}),
    }));
  const tableLayouts = state.tableLayouts.filter((layout) =>
    state.bootstrap?.profiles.some((profile) => profile.id === layout.connectionId),
  );
  return {
    theme: state.theme,
    density: state.density,
    sidebarVisible: state.sidebarVisible,
    sidebarCollapsed: !state.sidebarVisible,
    inspectorVisible: state.inspectorVisible,
    activeDraftId: drafts.some((draft) => draft.id === state.activeTabId)
      ? state.activeTabId
      : null,
    drafts,
    tableLayouts,
    shortcuts: state.shortcuts,
  };
}

const writer = new WorkspaceWriter(
  (workspace) => client.call("save_workspace", { workspace }),
  (error) => useStudio.getState().notify(`Workspace could not be saved: ${messageOf(error)}`),
);
if (recoveryRemembered()) void writer.pause();

let startupRecoveryHeld = false;
const recoveryListeners = new Set<() => void>();
function setStartupRecoveryHeld(value: boolean) {
  if (value === startupRecoveryHeld) return;
  startupRecoveryHeld = value;
  for (const listener of recoveryListeners) listener();
}

/** Call once on App mount, before exposing editors or starting reconciliation. */
export function initializeWorkspaceRecoveryStartupHold() {
  if (writer.isPaused() || recoveryRemembered()) {
    void writer
      .pause()
      .catch((error) =>
        useStudio
          .getState()
          .notify(`Workspace saving remains paused for recovery: ${messageOf(error)}`),
      );
    setStartupRecoveryHeld(true);
  }
  return startupRecoveryHeld;
}
export const workspaceRecoveryStartupHeld = () => startupRecoveryHeld;
export function subscribeWorkspaceRecovery(listener: () => void) {
  recoveryListeners.add(listener);
  return () => {
    recoveryListeners.delete(listener);
  };
}
export const workspaceRecoveryHeld = () => writer.isPaused();

export function pauseWorkspaceForRecovery() {
  return writer.pause();
}

export function discardRecoveryWorkspaceSaves() {
  writer.discard();
}

export function resumeWorkspaceAfterRecovery(discard: boolean) {
  if (discard) writer.discard();
  writer.resume();
  setStartupRecoveryHeld(false);
  if (useStudio.getState().bootstrap) writer.schedule(workspaceSnapshot());
}
export function watchWorkspace() {
  if (!desktopAvailable) return () => {};
  let previous = "";
  const changed = () => {
    if (!useStudio.getState().bootstrap) return;
    const workspace = workspaceSnapshot(),
      serialized = JSON.stringify(workspace);
    if (serialized === previous) return;
    previous = serialized;
    writer.schedule(workspace);
  };
  const flushOnHide = () => {
    if (document.visibilityState === "hidden") void writer.flush().catch(() => {});
  };
  const flushOnPageHide = () => {
    void writer.flush().catch(() => {});
  };
  changed();
  window.addEventListener("pagehide", flushOnPageHide);
  document.addEventListener("visibilitychange", flushOnHide);
  const unsubscribe = useStudio.subscribe(changed);
  return () => {
    unsubscribe();
    window.removeEventListener("pagehide", flushOnPageHide);
    document.removeEventListener("visibilitychange", flushOnHide);
  };
}
export function flushWorkspace() {
  return writer.flush();
}

/** A paused drain is useful internally, but cannot prove a safe durable close. */
export function flushWorkspaceForClose() {
  return writer.flushDurably();
}
