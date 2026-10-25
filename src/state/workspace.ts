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
  return {
    theme: state.theme,
    density: state.density,
    sidebarVisible: state.sidebarVisible,
    inspectorVisible: state.inspectorVisible,
    activeDraftId: drafts.some((draft) => draft.id === state.activeTabId)
      ? state.activeTabId
      : null,
    drafts,
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
  changed();
  return useStudio.subscribe(changed);
}
export function flushWorkspace() {
  return writer.flush();
}
