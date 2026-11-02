import { useEffect, useRef, useState } from "react";
import type {
  CompletionCatalog,
  DatabaseObjectRef,
  ObjectDetails,
  TableSchema,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { metadataSessionForTab, useStudio, type StudioTab } from "../../state/studio";
import { isTabularObjectKind } from "../connections/explorer-tree";
import { DdlPanel, ObjectInspectorView } from "./ObjectInspectorView";
import { StructureView } from "./UtilityViews";

/** A read-only inspector belongs to one observed console context, never the connection default. */
export function SessionStructureView({ tab }: { tab: StudioTab }) {
  const state = useStudio();
  const session = metadataSessionForTab(tab, state);
  const target = tab.object;
  const origin = tab.metadataOrigin!;
  const key = JSON.stringify([session?.id, origin.catalogRevision, tab.id, tab.revision, target]);
  const action = useRef({ key, mounted: true, pending: false });
  action.current.key = key;
  useEffect(() => {
    action.current.mounted = true;
    return () => {
      action.current.mounted = false;
    };
  }, []);
  const withCurrentOrigin = async (apply: () => void) => {
    if (action.current.pending || !session) return;
    action.current.pending = true;
    const stillCurrent = () => {
      const live = useStudio.getState();
      return (
        action.current.mounted &&
        action.current.key === key &&
        live.tabs.some((entry) => entry.id === tab.id && entry.revision === tab.revision) &&
        metadataSessionForTab(tab, live)?.id === session.id
      );
    };
    try {
      if (!stillCurrent()) return;
      const catalog = await client.call<CompletionCatalog>("completion_catalog", {
        sessionId: session.id,
      });
      if (!stillCurrent()) return;
      if (catalog.revision !== origin.catalogRevision)
        throw new Error("Console context changed. Navigate from the console again.");
      apply();
    } catch (err) {
      if (stillCurrent()) useStudio.getState().notify(messageOf(err));
    } finally {
      action.current.pending = false;
    }
  };
  const onOpenObject = (object: DatabaseObjectRef) => {
    void withCurrentOrigin(() => useStudio.getState().openLinkedSessionObject(tab.id, object));
  };
  const onOpenInEditor = (ddl: string) => {
    void withCurrentOrigin(() => {
      if (!useStudio.getState().appendSessionDdl(tab.id, ddl))
        useStudio
          .getState()
          .notify("Source console is unavailable or running. Return to it before appending DDL.");
    });
  };
  const [result, setResult] = useState<{
    key: string;
    schema: TableSchema | null;
    details: ObjectDetails | null;
    error: string | null;
  } | null>(null);
  useEffect(() => {
    if (!session || !target || !origin.catalogRevision) return;
    let active = true;
    const sessionId = session.id;
    const stillCurrent = () =>
      active && metadataSessionForTab(tab, useStudio.getState())?.id === sessionId;
    void (async () => {
      let schema: TableSchema | null = null;
      let details: ObjectDetails | null = null;
      let error: string | null = null;
      const before = await client.call<CompletionCatalog>("completion_catalog", { sessionId });
      if (before.revision !== origin.catalogRevision)
        throw new Error("Console context changed. Navigate from the console again.");
      if (!stillCurrent()) return;
      if (isTabularObjectKind(target.kind)) {
        schema = await client.call<TableSchema>("completion_schema", {
          sessionId,
          table: { schema: target.schema, name: target.name },
          revision: before.revision,
        });
      }
      if (!stillCurrent()) return;
      if (session.capabilities.objectExplorer) {
        try {
          details = await client.call<ObjectDetails>("object_details", {
            sessionId,
            object: target,
          });
        } catch (err) {
          error = messageOf(err);
        }
      }
      const after = await client.call<CompletionCatalog>("completion_catalog", { sessionId });
      if (after.revision !== before.revision)
        throw new Error("Console context changed. Navigate from the console again.");
      if (stillCurrent()) setResult({ key, schema, details, error });
    })().catch((err) => {
      if (stillCurrent()) setResult({ key, schema: null, details: null, error: messageOf(err) });
    });
    return () => {
      active = false;
    };
  }, [key]);
  if (!session)
    return (
      <div className="utility-empty">
        <h3>Console context unavailable</h3>
        <p>
          Return to the originating console and navigate again after reconnecting or changing
          context.
        </p>
      </div>
    );
  if (!origin.catalogRevision)
    return <div className="grid-empty">Wait for console metadata, then navigate again.</div>;
  const current = result?.key === key ? result : null;
  if (!current) return <div className="grid-empty">Loading object metadata…</div>;
  if (current.error && !current.schema && !current.details)
    return (
      <div role="alert" className="inline-error">
        {current.error}
      </div>
    );
  if (tab.view === "ddl")
    return (
      <DdlPanel
        connectionId={null}
        ddlStatus={current.details?.ddl ?? null}
        loading={false}
        error={current.error}
        onOpenInEditor={onOpenInEditor}
        openLabel="Append DDL to source console"
      />
    );
  if (current.schema)
    return (
      <StructureView
        schema={current.schema}
        details={current.details}
        detailsError={current.error}
        highlightedColumn={tab.highlightedColumn}
        onOpenObject={onOpenObject}
      />
    );
  if (current.details)
    return (
      <ObjectInspectorView
        tab={{ ...tab, connectionId: null }}
        session={session}
        providedDetails={current.details}
        onOpenObject={onOpenObject}
        onOpenInEditor={onOpenInEditor}
        openLabel="Append DDL to source console"
      />
    );
  return <div className="grid-empty">Extended object details unavailable for this engine.</div>;
}
