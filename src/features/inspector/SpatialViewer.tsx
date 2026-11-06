import { useState, useMemo, useCallback, useRef } from "react";
import { ZoomIn, ZoomOut, Maximize2, Copy, Check, MapPin, Compass } from "lucide-react";
import type { ColumnInfo } from "../../ipc/generated";
import {
  parseSpatialGeometry,
  buildSvgRenderData,
  type ParsedSpatialGeometry,
} from "./spatial-format";
import { useStudio } from "../../state/studio";

export interface SpatialViewerProps {
  value: string | null;
  column?: Pick<ColumnInfo, "name" | "databaseType" | "kind">;
  compact?: boolean;
}

export function SpatialViewer({ value, column: _column, compact = false }: SpatialViewerProps) {
  const [zoom, setZoom] = useState<number>(1.0);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [hoverCoord, setHoverCoord] = useState<{ x: number; y: number } | null>(null);

  const svgRef = useRef<SVGSVGElement | null>(null);

  const parseResult = useMemo(() => parseSpatialGeometry(value), [value]);

  const svgData = useMemo(() => {
    if (!parseResult.ok || !parseResult.geometry) return null;
    return buildSvgRenderData(parseResult.geometry);
  }, [parseResult]);

  const handleCopy = useCallback(async (text: string, label: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(label);
    setTimeout(() => setCopiedKey(null), 1500);
    useStudio.getState().notify(`Copied ${label}`);
  }, []);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<SVGSVGElement>) => {
      if (!svgRef.current || !svgData) return;
      const rect = svgRef.current.getBoundingClientRect();
      const clientX = e.clientX - rect.left;
      const clientY = e.clientY - rect.top;

      // Map SVG pixels to viewBox coordinates
      const normX = clientX / rect.width;
      const normY = clientY / rect.height;

      const worldX = svgData.viewBoxBounds.minX + normX * svgData.viewBoxBounds.width;
      // Invert Y back to world coordinates
      const worldY =
        svgData.viewBoxBounds.minY +
        svgData.viewBoxBounds.height -
        normY * svgData.viewBoxBounds.height;

      setHoverCoord({
        x: Number(worldX.toFixed(4)),
        y: Number(worldY.toFixed(4)),
      });
    },
    [svgData],
  );

  const handleMouseLeave = useCallback(() => {
    setHoverCoord(null);
  }, []);

  if (value === null || !parseResult.ok || !svgData || !parseResult.geometry) {
    return (
      <div className="inspector-placeholder" data-testid="spatial-viewer">
        {value === null ? "NULL" : parseResult.error || "Invalid spatial geometry"}
      </div>
    );
  }

  const geom: ParsedSpatialGeometry = parseResult.geometry;

  return (
    <div
      className={`spatial-viewer-container ${compact ? "compact" : ""}`}
      data-testid="spatial-viewer"
    >
      {/* Header Toolbar: Type + SRID + Actions */}
      <div className="spatial-toolbar">
        <div className="spatial-badges">
          <span className="spatial-type-badge">
            <Compass size={11} />
            {geom.kind}
          </span>
          {geom.srid && <span className="spatial-srid-badge">SRID {geom.srid}</span>}
          <span className="spatial-vertex-badge">
            {geom.vertexCount} verti{geom.vertexCount === 1 ? "x" : "ces"}
          </span>
        </div>

        <div className="spatial-zoom-bar">
          <button
            type="button"
            className="icon-button micro"
            title="Zoom In"
            aria-label="Zoom in"
            onClick={() => setZoom((z) => Math.min(5.0, Number((z * 1.25).toFixed(2))))}
          >
            <ZoomIn size={12} />
          </button>
          <span className="spatial-zoom-percent">{Math.round(zoom * 100)}%</span>
          <button
            type="button"
            className="icon-button micro"
            title="Zoom Out"
            aria-label="Zoom out"
            onClick={() => setZoom((z) => Math.max(0.2, Number((z / 1.25).toFixed(2))))}
          >
            <ZoomOut size={12} />
          </button>
          <button
            type="button"
            className="icon-button micro"
            title="Reset Zoom"
            aria-label="Reset zoom"
            onClick={() => setZoom(1.0)}
          >
            <Maximize2 size={11} />
          </button>
        </div>
      </div>

      {/* Interactive Vector SVG Canvas */}
      <div className="spatial-canvas-wrapper" tabIndex={0}>
        <div
          className="spatial-svg-stage"
          style={{ transform: `scale(${zoom})`, transformOrigin: "center center" }}
        >
          <svg
            ref={svgRef}
            viewBox={svgData.viewBox}
            className="spatial-svg-plot"
            preserveAspectRatio="xMidYMid meet"
            onMouseMove={handleMouseMove}
            onMouseLeave={handleMouseLeave}
            aria-label={`Spatial plot: ${geom.kind}`}
          >
            {/* Vector paths (Polygons & Lines) */}
            {svgData.paths.map((p, idx) => (
              <path
                key={idx}
                d={p.d}
                fillRule={p.fillRule}
                className={`spatial-path spatial-path-${p.type}`}
                vectorEffect="non-scaling-stroke"
              />
            ))}

            {/* Vertices */}
            {svgData.vertices.map((v, idx) => (
              <circle
                key={idx}
                cx={v.cx}
                cy={v.cy}
                r="3"
                className="spatial-vertex"
                vectorEffect="non-scaling-stroke"
                onMouseEnter={() => setHoverCoord({ x: v.worldX, y: v.worldY })}
              />
            ))}
          </svg>
        </div>
      </div>

      {/* Coordinate & Bounding Box Footer */}
      <div className="spatial-footer">
        <div className="spatial-coords-readout">
          <MapPin size={11} />
          {hoverCoord ? (
            <span>
              X: <strong>{hoverCoord.x}</strong>, Y: <strong>{hoverCoord.y}</strong>
            </span>
          ) : (
            <span className="muted">Hover geometry for coordinates</span>
          )}
        </div>

        <div className="spatial-copy-actions">
          <button
            type="button"
            className="button ghost small"
            onClick={() => handleCopy(value || "", "WKT")}
          >
            {copiedKey === "WKT" ? <Check size={11} /> : <Copy size={11} />}
            Copy WKT
          </button>
        </div>
      </div>

      {/* Bounding Box Information */}
      <div className="spatial-bbox-info">
        <span className="bbox-label">BBOX:</span>
        <code className="bbox-val">[{geom.bbox.map((n) => Number(n.toFixed(4))).join(", ")}]</code>
      </div>
    </div>
  );
}
