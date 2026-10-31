import type { SavedWorkspace } from "../ipc/generated";
import { client, desktopAvailable, messageOf } from "../ipc/client";
import { useStudio } from "./studio";

/** Serialize saves so an older slow request can never replace a newer draft. */
export class WorkspaceWriter {
  private pending: SavedWorkspace | null = null;
  private running: Promise<void> | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    private write: (workspace: SavedWorkspace) => Promise<void>,
    private onError: (error: unknown) => void,
  ) {}
  schedule(workspace: SavedWorkspace) {
    this.pending = workspace;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      void this.flush().catch(this.onError);
    }, 400);
  }
  async flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.running) {
      await this.running;
      return this.flush();
    }
    this.running = (async () => {
      while (this.pending) {
        const workspace = this.pending;
        this.pending = null;
        try {
          await this.write(workspace);
        } catch (error) {
          this.pending ??= workspace;
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
    }));
  const tableLayouts = state.tableLayouts.filter((layout) =>
    state.bootstrap?.profiles.some((profile) => profile.id === layout.connectionId),
  );
  return {
    theme: state.theme,
    density: state.density,
    sidebarVisible: state.sidebarVisible,
    inspectorVisible: state.inspectorVisible,
    activeDraftId: drafts.some((draft) => draft.id === state.activeTabId)
      ? state.activeTabId
      : null,
    drafts,
    tableLayouts,
  };
}

const writer = new WorkspaceWriter(
  (workspace) => client.call("save_workspace", { workspace }),
  (error) => useStudio.getState().notify(`Workspace could not be saved: ${messageOf(error)}`),
);
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
