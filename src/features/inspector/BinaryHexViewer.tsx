import { useState, useMemo, useEffect, useCallback } from "react";
import {
  ZoomIn,
  ZoomOut,
  Maximize2,
  RotateCcw,
  Copy,
  Check,
  Download,
  AlertTriangle,
  FileCode,
} from "lucide-react";
import type { ColumnInfo } from "../../ipc/generated";
import {
  parseBinaryBytes,
  detectImageFormat,
  extractImageDimensions,
  formatHexDumpLines,
  bytesToDataUrl,
  formatByteSize,
  DEFAULT_HEX_MAX_BYTES,
  type ImageDimensions,
} from "./binary-utils";

export interface BinaryHexViewerProps {
  value: string | null;
  column?: Pick<ColumnInfo, "name" | "databaseType" | "kind">;
  compact?: boolean;
  maxBytes?: number;
}

export function BinaryHexViewer({
  value,
  column,
  compact = false,
  maxBytes = DEFAULT_HEX_MAX_BYTES,
}: BinaryHexViewerProps) {
  const [viewMode, setViewMode] = useState<"image" | "hex">("hex");
  const [zoom, setZoom] = useState<number>(1.0);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [imageError, setImageError] = useState<boolean>(false);
  const [loadedDims, setLoadedDims] = useState<ImageDimensions | null>(null);

  // 1. Parse bytes
  const binaryData = useMemo(() => parseBinaryBytes(value, maxBytes), [value, maxBytes]);

  // 2. Detect image format
  const imageFormat = useMemo(
    () => (binaryData.bytes.length > 0 ? detectImageFormat(binaryData.bytes) : null),
    [binaryData.bytes],
  );

  // Auto-switch to "image" mode if image format detected and no error
  useEffect(() => {
    if (imageFormat && !imageError) {
      setViewMode("image");
      setImageError(false);
      setZoom(1.0);
      const headerDims = extractImageDimensions(binaryData.bytes, imageFormat.format);
      setLoadedDims(headerDims);
    } else {
      setViewMode("hex");
    }
  }, [imageFormat, binaryData.bytes, imageError]);

  // Generate Data URL for image rendering
  const dataUrl = useMemo(() => {
    if (!imageFormat || binaryData.bytes.length === 0) return null;
    return bytesToDataUrl(binaryData.bytes, imageFormat.mime);
  }, [imageFormat, binaryData.bytes]);

  // Generate classic 3-column hex lines
  const hexLines = useMemo(() => formatHexDumpLines(binaryData.bytes, true), [binaryData.bytes]);

  const handleCopy = useCallback(async (text: string, key: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  }, []);

  const handleDownload = useCallback(() => {
    if (!dataUrl || !imageFormat) return;
    const a = document.createElement("a");
    a.href = dataUrl;
    a.download = `${column?.name || "image"}.${imageFormat.format}`;
    a.click();
  }, [dataUrl, imageFormat, column?.name]);

  if (value === null || binaryData.totalBytes === 0) {
    return (
      <div className="inspector-placeholder" data-testid="binary-hex-viewer">
        {value === null ? "NULL" : "0 bytes (empty binary value)"}
      </div>
    );
  }

  return (
    <div
      className={`binary-hex-viewer ${compact ? "compact" : ""}`}
      data-testid="binary-hex-viewer"
    >
      {/* Header Toolbar */}
      <div className="binary-toolbar">
        {imageFormat && !imageError ? (
          <div className="binary-mode-tabs" role="tablist" aria-label="Binary view mode">
            <button
              type="button"
              role="tab"
              aria-selected={viewMode === "image"}
              className={`binary-mode-tab ${viewMode === "image" ? "active" : ""}`}
              onClick={() => setViewMode("image")}
            >
              Image Preview
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={viewMode === "hex"}
              className={`binary-mode-tab ${viewMode === "hex" ? "active" : ""}`}
              onClick={() => setViewMode("hex")}
            >
              Hex Dump
            </button>
          </div>
        ) : (
          <span className="binary-format-tag">
            <FileCode size={12} />
            Hex / ASCII Dump
          </span>
        )}

        <div className="binary-size-meta">
          <span>{formatByteSize(binaryData.totalBytes)}</span>
          {binaryData.truncated && (
            <span className="binary-truncation-pill">first {(maxBytes / 1024).toFixed(0)} KB</span>
          )}
        </div>
      </div>

      {/* Main Content Area */}
      {viewMode === "image" && dataUrl && !imageError ? (
        <div className="image-previewer-stage">
          {/* Zoom & Action Controls */}
          <div className="image-zoom-bar">
            <button
              type="button"
              className="icon-button small"
              title="Zoom In"
              aria-label="Zoom in"
              onClick={() => setZoom((z) => Math.min(5.0, Number((z * 1.25).toFixed(2))))}
            >
              <ZoomIn size={13} />
            </button>
            <span className="image-zoom-percent">{Math.round(zoom * 100)}%</span>
            <button
              type="button"
              className="icon-button small"
              title="Zoom Out"
              aria-label="Zoom out"
              onClick={() => setZoom((z) => Math.max(0.2, Number((z / 1.25).toFixed(2))))}
            >
              <ZoomOut size={13} />
            </button>
            <button
              type="button"
              className="icon-button small"
              title="100% Scale"
              aria-label="Reset zoom"
              onClick={() => setZoom(1.0)}
            >
              <RotateCcw size={12} />
            </button>
            <button
              type="button"
              className="icon-button small"
              title="Fit to view"
              aria-label="Fit to view"
              onClick={() => setZoom(1.0)}
            >
              <Maximize2 size={12} />
            </button>
            <button
              type="button"
              className="icon-button small"
              title="Download image"
              aria-label="Download image"
              onClick={handleDownload}
            >
              <Download size={13} />
            </button>
          </div>

          {/* Image Display with Transparency Checkerboard */}
          <div className="image-viewport image-checkerboard" tabIndex={0}>
            <img
              src={dataUrl}
              alt={column?.name || "Binary image preview"}
              style={{ transform: `scale(${zoom})`, transformOrigin: "center center" }}
              onLoad={(e) => {
                const img = e.currentTarget;
                setLoadedDims({ width: img.naturalWidth, height: img.naturalHeight });
              }}
              onError={() => {
                setImageError(true);
                setViewMode("hex");
              }}
            />
          </div>

          {/* Dimensions & Metadata Footer */}
          <div className="image-details-footer">
            <span className="badge">{imageFormat?.label}</span>
            {loadedDims && (
              <span>
                {loadedDims.width} × {loadedDims.height} px
              </span>
            )}
            {loadedDims && (
              <span className="muted">({(loadedDims.width / loadedDims.height).toFixed(2)}:1)</span>
            )}
          </div>
        </div>
      ) : (
        /* Hex Dump 3-Column Classic View */
        <div className="hex-dump-container">
          {imageError && (
            <div className="hex-fallback-alert" role="alert">
              <AlertTriangle size={13} />
              <span>Image stream incomplete or corrupted; displaying Hex Dump.</span>
            </div>
          )}

          {/* Copy Bar */}
          <div className="hex-actions-bar">
            <button
              type="button"
              className="button ghost small"
              onClick={() => {
                const hexOnly = Array.from(binaryData.bytes)
                  .map((b) => b.toString(16).padStart(2, "0"))
                  .join(" ");
                handleCopy(hexOnly, "hex");
              }}
            >
              {copiedKey === "hex" ? <Check size={11} /> : <Copy size={11} />}
              Copy Hex
            </button>
            <button
              type="button"
              className="button ghost small"
              onClick={() => {
                const asciiOnly = Array.from(binaryData.bytes)
                  .map((b) => (b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : "."))
                  .join("");
                handleCopy(asciiOnly, "ascii");
              }}
            >
              {copiedKey === "ascii" ? <Check size={11} /> : <Copy size={11} />}
              Copy ASCII
            </button>
          </div>

          {/* 3-Column Hex Dump Grid */}
          <div className="hex-dump-grid" role="table" aria-label="Binary hex dump" tabIndex={0}>
            <div className="hex-grid-header">
              <span className="hex-col-offset">OFFSET</span>
              <span className="hex-col-bytes">HEX BYTES (00-07 08-0F)</span>
              <span className="hex-col-ascii">ASCII</span>
            </div>
            <div className="hex-grid-body">
              {hexLines.map((line) => (
                <div key={line.offset} className="hex-row" role="row">
                  {/* Column 1: Offset */}
                  <span className="hex-cell-offset" role="rowheader">
                    {line.offsetHex}
                  </span>

                  {/* Column 2: 16 Hex Bytes with 8-byte spacer */}
                  <span className="hex-cell-bytes">
                    <span className="hex-subgroup">{line.hexGroup1}</span>
                    <span className="hex-spacer"> </span>
                    <span className="hex-subgroup">{line.hexGroup2}</span>
                  </span>

                  {/* Column 3: Printable ASCII */}
                  <span className="hex-cell-ascii">|{line.ascii}|</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
