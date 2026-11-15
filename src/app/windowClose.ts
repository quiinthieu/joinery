type CloseDependencies = {
  hasPendingWork(): boolean;
  flush(): Promise<void>;
  cancelConnections(): Promise<void>;
  shutdown(): Promise<void>;
  destroy(): Promise<void>;
  confirm(approve: () => Promise<void>): void;
  lockInteraction(): () => void;
};

export function createWindowCloser(actions: CloseDependencies) {
  let closing = false;
  const close = async (approved: boolean) => {
    if (closing) return;
    closing = true;
    const unlock = actions.lockInteraction();
    try {
      await actions.flush();
      // Work can also arrive from an already pending async action while the
      // workspace is being saved. It needs the same explicit user decision.
      if (!approved && actions.hasPendingWork()) {
        actions.confirm(() => close(true));
        return;
      }
      await actions.cancelConnections();
      await actions.shutdown();
      await actions.destroy();
    } finally {
      unlock();
      closing = false;
    }
  };
  return async () => {
    if (closing) return;
    if (actions.hasPendingWork()) actions.confirm(() => close(true));
    else await close(false);
  };
}

export function lockWindowInteraction() {
  const root = document.getElementById("root");
  const previous = root?.inert ?? false;
  if (root) root.inert = true;
  const block = (event: Event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  const events = ["keydown", "pointerdown", "click", "submit", "beforeinput"];
  for (const event of events) window.addEventListener(event, block, true);
  return () => {
    if (root) root.inert = previous;
    for (const event of events) window.removeEventListener(event, block, true);
  };
}
