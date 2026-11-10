import { useEffect, useRef } from "react";
import { client, messageOf } from "../../ipc/client";
import type { ConnectionSecrets, SessionInfo } from "../../ipc/generated";
import { useStudio } from "../../state/studio";

const pending = new Set<ConnectionSetup>();

export function hasPendingConnections() {
  return pending.size > 0;
}

export async function cancelPendingConnections() {
  await Promise.all([...pending].map((setup) => setup.cancel()));
}

function reportCleanupError(error: unknown) {
  useStudio.getState().notify(`Connection cleanup failed: ${messageOf(error)}`);
}

class ConnectionSetup {
  active = true;
  private attemptId: string | null = null;
  private profileId: string | null = null;
  private cancellations = new Map<"attempt" | "session", Promise<void>>();
  private failures = new Set<"attempt" | "session">();
  private cleaning = 0;
  private adopted = false;
  private lateSessionId: string | null = null;
  private finished = false;

  constructor() {
    pending.add(this);
  }

  cancel() {
    this.active = false;
    if (this.adopted) return Promise.resolve();
    const work: Promise<void>[] = [];
    if (this.attemptId)
      work.push(
        this.cleanup("attempt", () =>
          client.call<void>("cancel_connection_attempt", { attemptId: this.attemptId! }),
        ),
      );
    if (this.lateSessionId) {
      const id = this.lateSessionId;
      work.push(
        this.cleanup("session", async () => {
          await client.call<void>("disconnect", { sessionId: id });
          if (this.lateSessionId === id) this.lateSessionId = null;
        }),
      );
    }
    return Promise.all(work).then(() => {});
  }

  private cleanup(kind: "attempt" | "session", action: () => Promise<void>) {
    const existing = this.cancellations.get(kind);
    if (existing) return existing;
    this.cleaning += 1;
    const work = action()
      .then(() => {
        this.failures.delete(kind);
      })
      .catch((error) => {
        this.cancellations.delete(kind);
        this.failures.add(kind);
        pending.add(this);
        reportCleanupError(error);
        throw error;
      })
      .finally(() => {
        this.cleaning -= 1;
        if (this.finished && this.cleaning === 0 && this.failures.size === 0) pending.delete(this);
      });
    this.cancellations.set(kind, work);
    return work;
  }

  async prepare(profileId: string) {
    if (!this.active) return null;
    if (this.attemptId) {
      if (this.profileId !== profileId) throw new Error("Connection setup profile changed");
      return this.attemptId;
    }
    this.profileId = profileId;
    this.attemptId = await client.call("prepare_connection", { id: profileId });
    if (!this.active) {
      await this.cancel();
      return null;
    }
    return this.attemptId;
  }

  async connect(
    profileId: string,
    secrets: ConnectionSecrets,
    accept: (session: SessionInfo) => void,
  ) {
    const attemptId = await this.prepare(profileId);
    if (!attemptId) return;
    const session = await client.call<SessionInfo>("connect_prepared", {
      attemptId,
      secrets,
    });
    if (!this.active) {
      // Publication can win the backend cancellation race. Retire only the
      // returned session; a later connection for this profile belongs elsewhere.
      this.lateSessionId = session.id;
      // A previously acknowledged attempt cancellation cannot settle a
      // publication that won that race. Its exact session needs its own close.
      await this.cancel();
      return;
    }
    this.adopted = true;
    accept(session);
  }

  finish() {
    this.active = false;
    this.finished = true;
    // A declined host-key prompt or failed preflight can leave an unclaimed
    // reservation. Retire it too; a published/adopted session belongs to the app.
    if (!this.adopted && this.attemptId) void this.cancel().catch(() => {});
    if (this.cleaning === 0 && this.failures.size === 0) pending.delete(this);
  }
}

export function useConnectionSetup() {
  const current = useRef<ConnectionSetup | null>(null);
  useEffect(
    () => () => {
      void current.current?.cancel().catch(() => {});
    },
    [],
  );
  return {
    begin() {
      if (current.current) return null;
      const setup = new ConnectionSetup();
      current.current = setup;
      return setup;
    },
    finish(setup: ConnectionSetup) {
      setup.finish();
      if (current.current === setup) current.current = null;
    },
    cancel() {
      void current.current?.cancel().catch(() => {});
    },
  };
}
