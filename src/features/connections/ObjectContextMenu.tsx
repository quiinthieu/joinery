import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  Archive,
  ArrowRight,
  Clock,
  Code2,
  Copy,
  Database,
  Eye,
  FileCode2,
  FileText,
  GitPullRequest,
  Play,
  Radio,
  RotateCcw,
  Table2,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import { client, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import type {
  ConnectionProfile,
  DatabaseObjectRef,
  ObjectDetails,
  SessionInfo,
} from "../../ipc/generated";
import type { ExplorerItem } from "./explorer-tree";
import type { TableObjectDialogRequest } from "../studio/TableObjectDialog";

export interface ObjectContextMenuProps {
  x: number;
  y: number;
  item: ExplorerItem;
  profile: ConnectionProfile;
  session?: SessionInfo;
  onClose: () => void;
  onRequestTableDialog?: (req: TableObjectDialogRequest) => void;
  onRequestMaintenance?: () => void;
}

interface MenuItemDef {
  id: string;
  label: string;
  icon: React.ReactNode;
  disabled?: boolean;
  disabledReason?: string;
  onClick: () => void;
  separatorAfter?: boolean;
}

export function ObjectContextMenu({
  x,
  y,
  item,
  profile,
  session,
  onClose,
  onRequestTableDialog,
  onRequestMaintenance,
}: ObjectContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ x, y });
  const [focusedIndex, setFocusedIndex] = useState(0);
  const isReadOnly = profile.safeMode === "readOnly";
  const isTableKind = item.object.kind === "table" || item.object.kind === "partitionedTable";
  const isViewKind = item.object.kind === "view" || item.object.kind === "materializedView";
  const isRelation = isTableKind || isViewKind;

  useLayoutEffect(() => {
    if (!menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    const maxX = window.innerWidth - rect.width - 8;
    const maxY = window.innerHeight - rect.height - 8;
    setPosition({
      x: Math.max(8, Math.min(x, maxX)),
      y: Math.max(8, Math.min(y, maxY)),
    });
  }, [x, y]);

  useEffect(() => {
    const handlePointerDown = (e: PointerEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, [onClose]);

  const quoteIdent = (name: string) => {
    if (profile.engine === "mysql") return `\`${name.replace(/`/g, "``")}\``;
    return `"${name.replace(/"/g, '""')}"`;
  };

  const qualifiedName = `${quoteIdent(item.object.schema)}.${quoteIdent(item.object.name)}`;

  const menuItems: MenuItemDef[] = [];

  // 1. Select from Table / View / Stream
  if (isRelation || item.object.kind === "stream") {
    menuItems.push({
      id: "select",
      label: "Select from Table",
      icon: <Play size={13} />,
      onClick: () => {
        const sql = `SELECT * FROM ${qualifiedName} LIMIT 100;`;
        useStudio.getState().openQuery(profile.id, sql);
        onClose();
      },
    });
  }

  // 2. Snowflake Stage List
  if (item.object.kind === "stage") {
    menuItems.push({
      id: "list-stage",
      label: "List stage files",
      icon: <Archive size={13} />,
      onClick: () => {
        const sql = `LIST @${qualifiedName};`;
        useStudio.getState().openQuery(profile.id, sql);
        onClose();
      },
    });
  }

  // 3. Snowflake Task History
  if (item.object.kind === "task") {
    menuItems.push({
      id: "task-history",
      label: "View task history",
      icon: <Clock size={13} />,
      onClick: () => {
        const sql = `SELECT * FROM TABLE(INFORMATION_SCHEMA.TASK_HISTORY(TASK_NAME => '${item.object.name}')) ORDER BY SCHEDULED_TIME DESC LIMIT 100;`;
        useStudio.getState().openQuery(profile.id, sql);
        onClose();
      },
    });
  }

  // 4. Snowflake Pipe Status
  if (item.object.kind === "pipe") {
    menuItems.push({
      id: "pipe-status",
      label: "Check pipe status",
      icon: <GitPullRequest size={13} />,
      onClick: () => {
        const sql = `SELECT SYSTEM$PIPE_STATUS('${qualifiedName}');`;
        useStudio.getState().openQuery(profile.id, sql);
        onClose();
      },
    });
  }

  // 5. Open Data / Browse
  menuItems.push({
    id: "open-data",
    label: isRelation ? "Browse data" : "Open object",
    icon: <Database size={13} />,
    onClick: () => {
      if (isRelation) {
        void useStudio
          .getState()
          .openTable(
            profile.id,
            { schema: item.object.schema, name: item.object.name },
            undefined,
            "data",
          );
      } else {
        void useStudio.getState().openObject(profile.id, item.object);
      }
      onClose();
    },
  });

  // 6. Inspect Structure
  if (isRelation) {
    menuItems.push({
      id: "inspect-structure",
      label: "Inspect structure",
      icon: <Table2 size={13} />,
      separatorAfter: true,
      onClick: () => {
        void useStudio
          .getState()
          .openTable(
            profile.id,
            { schema: item.object.schema, name: item.object.name },
            undefined,
            "structure",
          );
        onClose();
      },
    });
  } else {
    // Add separator after open
    if (menuItems.length > 0) {
      menuItems[menuItems.length - 1].separatorAfter = true;
    }
  }

  // 7. Copy DDL
  menuItems.push({
    id: "copy-ddl",
    label: "Copy DDL",
    icon: <Copy size={13} />,
    onClick: () => {
      onClose();
      const sessId = session?.id ?? "";
      void client
        .call<ObjectDetails>("object_details", { sessionId: sessId, object: item.object })
        .then((res) => {
          if (res.ddl.status === "available") {
            void navigator.clipboard.writeText(res.ddl.ddl);
            useStudio.getState().notify("Copied DDL to clipboard");
          } else {
            useStudio.getState().notify(res.ddl.reason || "DDL unavailable for this object");
          }
        })
        .catch((err) => useStudio.getState().notify(messageOf(err)));
    },
  });

  // 8. Generate DDL in Console
  menuItems.push({
    id: "generate-ddl",
    label: "Generate DDL in Console",
    icon: <FileCode2 size={13} />,
    separatorAfter: true,
    onClick: () => {
      onClose();
      const sessId = session?.id ?? "";
      void client
        .call<ObjectDetails>("object_details", { sessionId: sessId, object: item.object })
        .then((res) => {
          if (res.ddl.status === "available") {
            useStudio.getState().openQuery(profile.id, res.ddl.ddl);
          } else {
            useStudio.getState().notify(res.ddl.reason || "DDL unavailable for this object");
          }
        })
        .catch((err) => useStudio.getState().notify(messageOf(err)));
    },
  });

  // 9. Copy Name & Qualified Name
  menuItems.push({
    id: "copy-name",
    label: "Copy object name",
    icon: <Copy size={13} />,
    onClick: () => {
      void navigator.clipboard.writeText(item.object.name);
      useStudio.getState().notify(`Copied "${item.object.name}"`);
      onClose();
    },
  });
  menuItems.push({
    id: "copy-qualified-name",
    label: "Copy qualified name",
    icon: <Copy size={13} />,
    separatorAfter: true,
    onClick: () => {
      void navigator.clipboard.writeText(qualifiedName);
      useStudio.getState().notify(`Copied "${qualifiedName}"`);
      onClose();
    },
  });

  // 10. Table Maintenance
  if (
    isTableKind &&
    (profile.engine === "postgres" || profile.engine === "mysql") &&
    onRequestMaintenance
  ) {
    menuItems.push({
      id: "maintenance",
      label: "Table maintenance…",
      icon: <Wrench size={13} />,
      onClick: () => {
        onRequestMaintenance();
        onClose();
      },
    });
  }

  // 11. Truncate Table
  if (isTableKind) {
    menuItems.push({
      id: "truncate",
      label: "Truncate table…",
      icon: <RotateCcw size={13} />,
      disabled: isReadOnly,
      disabledReason: isReadOnly ? "Connection is in read-only safe mode" : undefined,
      onClick: () => {
        if (isReadOnly) return;
        const sql = `TRUNCATE TABLE ${qualifiedName};`;
        useStudio.getState().openQuery(profile.id, sql);
        onClose();
      },
    });
  }

  // 12. Drop Object
  menuItems.push({
    id: "drop",
    label: `Drop ${item.object.kind}…`,
    icon: <Trash2 size={13} />,
    disabled: isReadOnly,
    disabledReason: isReadOnly ? "Connection is in read-only safe mode" : undefined,
    onClick: () => {
      if (isReadOnly) return;
      if (
        onRequestTableDialog &&
        (profile.engine === "postgres" || profile.engine === "mysql") &&
        isRelation
      ) {
        onRequestTableDialog({ kind: "dropObject", object: item.object });
      } else {
        const kindKeyword =
          item.object.kind === "fileFormat"
            ? "FILE FORMAT"
            : item.object.kind === "materializedView"
              ? "MATERIALIZED VIEW"
              : item.object.kind === "partitionedTable"
                ? "TABLE"
                : item.object.kind.toUpperCase();
        const sql = `DROP ${kindKeyword} ${qualifiedName};`;
        useStudio.getState().openQuery(profile.id, sql);
      }
      onClose();
    },
  });

  // Keyboard navigation across items
  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setFocusedIndex((prev) => (prev + 1) % menuItems.length);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setFocusedIndex((prev) => (prev - 1 + menuItems.length) % menuItems.length);
      return;
    }
    if (e.key === "Home") {
      e.preventDefault();
      setFocusedIndex(0);
      return;
    }
    if (e.key === "End") {
      e.preventDefault();
      setFocusedIndex(menuItems.length - 1);
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const current = menuItems[focusedIndex];
      if (current && !current.disabled) {
        current.onClick();
      }
    }
  };

  const itemButtonsRef = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    itemButtonsRef.current[focusedIndex]?.focus();
  }, [focusedIndex]);

  return (
    <div
      ref={menuRef}
      className="object-context-menu"
      style={{ left: position.x, top: position.y }}
      role="menu"
      aria-label={`${item.primaryLabel} context actions`}
      tabIndex={-1}
      onKeyDown={handleKeyDown}
    >
      <div className="object-context-menu-header">
        <span className="object-context-menu-title" title={item.tooltip}>
          {item.primaryLabel}
        </span>
        <span className="object-context-menu-kind">{item.object.kind}</span>
      </div>
      <div className="object-context-menu-separator" />
      {menuItems.map((menuItem, idx) => (
        <div key={menuItem.id}>
          <button
            ref={(el) => {
              itemButtonsRef.current[idx] = el;
            }}
            type="button"
            role="menuitem"
            className={`object-context-menu-item ${menuItem.disabled ? "disabled" : ""}`}
            disabled={menuItem.disabled}
            title={menuItem.disabledReason}
            tabIndex={focusedIndex === idx ? 0 : -1}
            onClick={() => {
              if (!menuItem.disabled) menuItem.onClick();
            }}
          >
            <span className="object-context-menu-item-content">
              {menuItem.icon}
              <span>{menuItem.label}</span>
            </span>
          </button>
          {menuItem.separatorAfter && <div className="object-context-menu-separator" />}
        </div>
      ))}
    </div>
  );
}
