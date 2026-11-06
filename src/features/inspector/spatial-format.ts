export interface Coordinate {
  x: number;
  y: number;
  z?: number;
  m?: number;
}

export type BoundingBox = [minX: number, minY: number, maxX: number, maxY: number];

export type SpatialKind =
  | "POINT"
  | "MULTIPOINT"
  | "LINESTRING"
  | "MULTILINESTRING"
  | "POLYGON"
  | "MULTIPOLYGON"
  | "GEOMETRYCOLLECTION";

export interface PolygonGeometry {
  exteriorRing: Coordinate[];
  interiorRings: Coordinate[][];
}

export interface ParsedSpatialGeometry {
  kind: SpatialKind;
  srid?: number;
  bbox: BoundingBox;
  vertexCount: number;
  points?: Coordinate[]; // POINT / MULTIPOINT
  lines?: Coordinate[][]; // LINESTRING / MULTILINESTRING
  polygons?: PolygonGeometry[]; // POLYGON / MULTIPOLYGON
  subGeometries?: ParsedSpatialGeometry[]; // GEOMETRYCOLLECTION
  rawType: string;
}

export interface SpatialParseResult {
  ok: boolean;
  geometry?: ParsedSpatialGeometry;
  error?: string;
}

export interface SvgRenderData {
  viewBox: string;
  paths: {
    d: string;
    type: "polygon" | "line" | "point";
    fillRule?: "evenodd" | "nonzero";
  }[];
  vertices: {
    cx: number;
    cy: number;
    worldX: number;
    worldY: number;
  }[];
  bbox: BoundingBox;
  viewBoxBounds: { minX: number; minY: number; width: number; height: number };
  emptyMessage?: string;
}

/**
 * Universal spatial parser for WKT and GeoJSON strings.
 */
export function parseSpatialGeometry(raw: string | null | undefined): SpatialParseResult {
  if (!raw) return { ok: false, error: "Empty or null geometry string" };
  const str = raw.trim();

  // 1. Try GeoJSON
  if (str.startsWith("{")) {
    try {
      const json = JSON.parse(str);
      return parseGeoJson(json);
    } catch {
      // not JSON, fallback to WKT
    }
  }

  // 2. WKT Parsing
  return parseWkt(str);
}

function parseGeoJson(data: any): SpatialParseResult {
  if (!data || typeof data !== "object") {
    return { ok: false, error: "Invalid GeoJSON object" };
  }

  // Handle GeoJSON Feature unwrapping
  if (data.type === "Feature") {
    if (!data.geometry) {
      return {
        ok: true,
        geometry: {
          kind: "GEOMETRYCOLLECTION",
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          subGeometries: [],
          rawType: "Feature",
        },
      };
    }
    return parseGeoJson(data.geometry);
  }

  const type = String(data.type || "").toUpperCase();

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let vertexCount = 0;

  const updateBbox = (pt: Coordinate) => {
    vertexCount++;
    if (pt.x < minX) minX = pt.x;
    if (pt.x > maxX) maxX = pt.x;
    if (pt.y < minY) minY = pt.y;
    if (pt.y > maxY) maxY = pt.y;
  };

  const toCoord = (c: number[]): Coordinate => {
    if (!c || c.length < 2) return { x: 0, y: 0 };
    const pt: Coordinate = { x: c[0], y: c[1] };
    if (c.length > 2) pt.z = c[2];
    if (c.length > 3) pt.m = c[3];
    updateBbox(pt);
    return pt;
  };

  const safeBbox = (): BoundingBox => (vertexCount > 0 ? [minX, minY, maxX, maxY] : [0, 0, 0, 0]);

  // 1. GeoJSON Point
  if (type === "POINT") {
    const coords = data.coordinates;
    const pt = Array.isArray(coords) && coords.length >= 2 ? toCoord(coords) : undefined;
    return {
      ok: true,
      geometry: {
        kind: "POINT",
        bbox: safeBbox(),
        vertexCount,
        points: pt ? [pt] : [],
        rawType: "Point",
      },
    };
  }

  // 2. GeoJSON MultiPoint
  if (type === "MULTIPOINT") {
    const coords = Array.isArray(data.coordinates) ? data.coordinates : [];
    const pts = coords.map(toCoord);
    return {
      ok: true,
      geometry: {
        kind: "MULTIPOINT",
        bbox: safeBbox(),
        vertexCount,
        points: pts,
        rawType: "MultiPoint",
      },
    };
  }

  // 3. GeoJSON LineString
  if (type === "LINESTRING") {
    const coords = Array.isArray(data.coordinates) ? data.coordinates : [];
    const line = coords.map(toCoord);
    return {
      ok: true,
      geometry: {
        kind: "LINESTRING",
        bbox: safeBbox(),
        vertexCount,
        lines: [line],
        rawType: "LineString",
      },
    };
  }

  // 4. GeoJSON MultiLineString
  if (type === "MULTILINESTRING") {
    const coords = Array.isArray(data.coordinates) ? data.coordinates : [];
    const lines = coords.map((l: number[][]) => (Array.isArray(l) ? l.map(toCoord) : []));
    return {
      ok: true,
      geometry: {
        kind: "MULTILINESTRING",
        bbox: safeBbox(),
        vertexCount,
        lines,
        rawType: "MultiLineString",
      },
    };
  }

  // 5. GeoJSON Polygon
  if (type === "POLYGON") {
    const rings = Array.isArray(data.coordinates) ? (data.coordinates as number[][][]) : [];
    const exterior = (rings[0] || []).map(toCoord);
    const interior = rings.slice(1).map((r) => r.map(toCoord));
    return {
      ok: true,
      geometry: {
        kind: "POLYGON",
        bbox: safeBbox(),
        vertexCount,
        polygons: [{ exteriorRing: exterior, interiorRings: interior }],
        rawType: "Polygon",
      },
    };
  }

  // 6. GeoJSON MultiPolygon
  if (type === "MULTIPOLYGON") {
    const polysData = Array.isArray(data.coordinates) ? (data.coordinates as number[][][][]) : [];
    const polys = polysData.map((rings) => {
      const exterior = (rings[0] || []).map(toCoord);
      const interior = rings.slice(1).map((r) => r.map(toCoord));
      return { exteriorRing: exterior, interiorRings: interior };
    });
    return {
      ok: true,
      geometry: {
        kind: "MULTIPOLYGON",
        bbox: safeBbox(),
        vertexCount,
        polygons: polys,
        rawType: "MultiPolygon",
      },
    };
  }

  // 7. GeoJSON GeometryCollection
  if (type === "GEOMETRYCOLLECTION") {
    const geoms = Array.isArray(data.geometries) ? data.geometries : [];
    const subGeometries: ParsedSpatialGeometry[] = [];
    const aggregatedPoints: Coordinate[] = [];
    const aggregatedLines: Coordinate[][] = [];
    const aggregatedPolygons: PolygonGeometry[] = [];
    let totalVertexCount = 0;

    for (const g of geoms) {
      const res = parseGeoJson(g);
      if (res.ok && res.geometry) {
        subGeometries.push(res.geometry);
        totalVertexCount += res.geometry.vertexCount;
        if (res.geometry.vertexCount > 0) {
          if (res.geometry.bbox[0] < minX) minX = res.geometry.bbox[0];
          if (res.geometry.bbox[1] < minY) minY = res.geometry.bbox[1];
          if (res.geometry.bbox[2] > maxX) maxX = res.geometry.bbox[2];
          if (res.geometry.bbox[3] > maxY) maxY = res.geometry.bbox[3];
        }
        if (res.geometry.points) aggregatedPoints.push(...res.geometry.points);
        if (res.geometry.lines) aggregatedLines.push(...res.geometry.lines);
        if (res.geometry.polygons) aggregatedPolygons.push(...res.geometry.polygons);
      }
    }

    return {
      ok: true,
      geometry: {
        kind: "GEOMETRYCOLLECTION",
        bbox: totalVertexCount > 0 ? [minX, minY, maxX, maxY] : [0, 0, 0, 0],
        vertexCount: totalVertexCount,
        subGeometries,
        points: aggregatedPoints.length > 0 ? aggregatedPoints : undefined,
        lines: aggregatedLines.length > 0 ? aggregatedLines : undefined,
        polygons: aggregatedPolygons.length > 0 ? aggregatedPolygons : undefined,
        rawType: "GeometryCollection",
      },
    };
  }

  // 8. GeoJSON FeatureCollection
  if (type === "FEATURECOLLECTION") {
    const features = Array.isArray(data.features) ? data.features : [];
    const subGeometries: ParsedSpatialGeometry[] = [];
    const aggregatedPoints: Coordinate[] = [];
    const aggregatedLines: Coordinate[][] = [];
    const aggregatedPolygons: PolygonGeometry[] = [];
    let totalVertexCount = 0;

    for (const f of features) {
      if (f && f.geometry) {
        const res = parseGeoJson(f.geometry);
        if (res.ok && res.geometry) {
          subGeometries.push(res.geometry);
          totalVertexCount += res.geometry.vertexCount;
          if (res.geometry.vertexCount > 0) {
            if (res.geometry.bbox[0] < minX) minX = res.geometry.bbox[0];
            if (res.geometry.bbox[1] < minY) minY = res.geometry.bbox[1];
            if (res.geometry.bbox[2] > maxX) maxX = res.geometry.bbox[2];
            if (res.geometry.bbox[3] > maxY) maxY = res.geometry.bbox[3];
          }
          if (res.geometry.points) aggregatedPoints.push(...res.geometry.points);
          if (res.geometry.lines) aggregatedLines.push(...res.geometry.lines);
          if (res.geometry.polygons) aggregatedPolygons.push(...res.geometry.polygons);
        }
      }
    }

    return {
      ok: true,
      geometry: {
        kind: "GEOMETRYCOLLECTION",
        bbox: totalVertexCount > 0 ? [minX, minY, maxX, maxY] : [0, 0, 0, 0],
        vertexCount: totalVertexCount,
        subGeometries,
        points: aggregatedPoints.length > 0 ? aggregatedPoints : undefined,
        lines: aggregatedLines.length > 0 ? aggregatedLines : undefined,
        polygons: aggregatedPolygons.length > 0 ? aggregatedPolygons : undefined,
        rawType: "FeatureCollection",
      },
    };
  }

  return { ok: false, error: `Unsupported GeoJSON type: ${type}` };
}

function parseWkt(str: string): SpatialParseResult {
  let cleaned = str.trim();
  let srid: number | undefined;

  // Extract SRID=xxxx; prefix
  const sridMatch = /^SRID=(\d+);/i.exec(cleaned);
  if (sridMatch) {
    srid = Number.parseInt(sridMatch[1], 10);
    cleaned = cleaned.slice(sridMatch[0].length).trim();
  }

  // Extract type keyword and dimensional modifier (ZM, Z, M)
  const typeMatch = /^([A-Z]+)(\s+ZM|\s+Z|\s+M)?/i.exec(cleaned);
  if (!typeMatch) {
    return { ok: false, error: "Invalid WKT: missing geometry type keyword" };
  }

  const kindStr = typeMatch[1].toUpperCase() as SpatialKind;
  const dimModifier = typeMatch[2] ? typeMatch[2].trim().toUpperCase() : "";
  const body = cleaned.slice(typeMatch[0].length).trim();

  // Validate parentheses balance before attempting coordinate parsing
  const parenValidation = validateWktParentheses(body);
  if (!parenValidation.valid) {
    return { ok: false, error: parenValidation.error };
  }

  const isEmpty = body.toUpperCase() === "EMPTY";

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let vertexCount = 0;

  const updateBbox = (pt: Coordinate) => {
    vertexCount++;
    if (pt.x < minX) minX = pt.x;
    if (pt.x > maxX) maxX = pt.x;
    if (pt.y < minY) minY = pt.y;
    if (pt.y > maxY) maxY = pt.y;
  };

  const parseCoordList = (text: string): Coordinate[] => {
    const coords: Coordinate[] = [];
    const pairs = text
      .split(",")
      .map((p) => p.trim())
      .filter(Boolean);
    for (const pair of pairs) {
      const nums = pair.split(/\s+/).map(Number);
      if (nums.length >= 2 && !Number.isNaN(nums[0]) && !Number.isNaN(nums[1])) {
        const pt: Coordinate = { x: nums[0], y: nums[1] };
        if (nums.length > 2) {
          pt.z = nums[2];
          if (dimModifier === "M") {
            pt.m = nums[2];
          }
        }
        if (nums.length > 3) {
          pt.m = nums[3];
        }
        updateBbox(pt);
        coords.push(pt);
      }
    }
    return coords;
  };

  const safeBbox = (): BoundingBox => (vertexCount > 0 ? [minX, minY, maxX, maxY] : [0, 0, 0, 0]);

  // 1. POINT
  if (kindStr === "POINT") {
    const coords = isEmpty ? [] : parseCoordList(extractParens(body));
    return {
      ok: true,
      geometry: {
        kind: "POINT",
        srid,
        bbox: safeBbox(),
        vertexCount,
        points: coords,
        rawType: "POINT",
      },
    };
  }

  // 2. LINESTRING
  if (kindStr === "LINESTRING") {
    const coords = isEmpty ? [] : parseCoordList(extractParens(body));
    return {
      ok: true,
      geometry: {
        kind: "LINESTRING",
        srid,
        bbox: safeBbox(),
        vertexCount,
        lines: [coords],
        rawType: "LINESTRING",
      },
    };
  }

  // 3. POLYGON
  if (kindStr === "POLYGON") {
    if (isEmpty) {
      return {
        ok: true,
        geometry: {
          kind: "POLYGON",
          srid,
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          polygons: [],
          rawType: "POLYGON",
        },
      };
    }
    const ringsText = extractRings(body);
    const rings = ringsText.map(parseCoordList);
    const exterior = rings[0] || [];
    const interior = rings.slice(1);
    return {
      ok: true,
      geometry: {
        kind: "POLYGON",
        srid,
        bbox: safeBbox(),
        vertexCount,
        polygons: [{ exteriorRing: exterior, interiorRings: interior }],
        rawType: "POLYGON",
      },
    };
  }

  // 4. MULTIPOLYGON
  if (kindStr === "MULTIPOLYGON") {
    if (isEmpty) {
      return {
        ok: true,
        geometry: {
          kind: "MULTIPOLYGON",
          srid,
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          polygons: [],
          rawType: "MULTIPOLYGON",
        },
      };
    }
    const polyBlocks = extractPolyBlocks(body);
    const polys: PolygonGeometry[] = [];
    for (const block of polyBlocks) {
      const ringsText = extractRings(`(${block})`);
      const rings = ringsText.map(parseCoordList);
      polys.push({
        exteriorRing: rings[0] || [],
        interiorRings: rings.slice(1),
      });
    }
    return {
      ok: true,
      geometry: {
        kind: "MULTIPOLYGON",
        srid,
        bbox: safeBbox(),
        vertexCount,
        polygons: polys,
        rawType: "MULTIPOLYGON",
      },
    };
  }

  // 5. MULTIPOINT
  if (kindStr === "MULTIPOINT") {
    if (isEmpty) {
      return {
        ok: true,
        geometry: {
          kind: "MULTIPOINT",
          srid,
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          points: [],
          rawType: "MULTIPOINT",
        },
      };
    }
    const inner = extractParens(body);
    const cleanedInner = inner.replace(/\(|\)/g, "");
    const coords = parseCoordList(cleanedInner);
    return {
      ok: true,
      geometry: {
        kind: "MULTIPOINT",
        srid,
        bbox: safeBbox(),
        vertexCount,
        points: coords,
        rawType: "MULTIPOINT",
      },
    };
  }

  // 6. MULTILINESTRING
  if (kindStr === "MULTILINESTRING") {
    if (isEmpty) {
      return {
        ok: true,
        geometry: {
          kind: "MULTILINESTRING",
          srid,
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          lines: [],
          rawType: "MULTILINESTRING",
        },
      };
    }
    const linesText = extractRings(body);
    const lines = linesText.map(parseCoordList);
    return {
      ok: true,
      geometry: {
        kind: "MULTILINESTRING",
        srid,
        bbox: safeBbox(),
        vertexCount,
        lines,
        rawType: "MULTILINESTRING",
      },
    };
  }

  // 7. GEOMETRYCOLLECTION
  if (kindStr === "GEOMETRYCOLLECTION") {
    if (isEmpty) {
      return {
        ok: true,
        geometry: {
          kind: "GEOMETRYCOLLECTION",
          srid,
          bbox: [0, 0, 0, 0],
          vertexCount: 0,
          subGeometries: [],
          points: [],
          lines: [],
          polygons: [],
          rawType: "GEOMETRYCOLLECTION",
        },
      };
    }

    const subWkts = splitWktCollection(body);
    const subGeometries: ParsedSpatialGeometry[] = [];
    const aggregatedPoints: Coordinate[] = [];
    const aggregatedLines: Coordinate[][] = [];
    const aggregatedPolygons: PolygonGeometry[] = [];
    let totalVertexCount = 0;

    for (const subWkt of subWkts) {
      const parsedSub = parseWkt(subWkt);
      if (parsedSub.ok && parsedSub.geometry) {
        subGeometries.push(parsedSub.geometry);
        totalVertexCount += parsedSub.geometry.vertexCount;
        if (parsedSub.geometry.vertexCount > 0) {
          if (parsedSub.geometry.bbox[0] < minX) minX = parsedSub.geometry.bbox[0];
          if (parsedSub.geometry.bbox[1] < minY) minY = parsedSub.geometry.bbox[1];
          if (parsedSub.geometry.bbox[2] > maxX) maxX = parsedSub.geometry.bbox[2];
          if (parsedSub.geometry.bbox[3] > maxY) maxY = parsedSub.geometry.bbox[3];
        }
        if (parsedSub.geometry.points) aggregatedPoints.push(...parsedSub.geometry.points);
        if (parsedSub.geometry.lines) aggregatedLines.push(...parsedSub.geometry.lines);
        if (parsedSub.geometry.polygons) aggregatedPolygons.push(...parsedSub.geometry.polygons);
      }
    }

    return {
      ok: true,
      geometry: {
        kind: "GEOMETRYCOLLECTION",
        srid,
        bbox: totalVertexCount > 0 ? [minX, minY, maxX, maxY] : [0, 0, 0, 0],
        vertexCount: totalVertexCount,
        subGeometries,
        points: aggregatedPoints.length > 0 ? aggregatedPoints : undefined,
        lines: aggregatedLines.length > 0 ? aggregatedLines : undefined,
        polygons: aggregatedPolygons.length > 0 ? aggregatedPolygons : undefined,
        rawType: "GEOMETRYCOLLECTION",
      },
    };
  }

  return { ok: false, error: `Unsupported WKT geometry: ${kindStr}` };
}

/**
 * Validates parenthesis balance and structural enclosure for WKT bodies.
 */
function validateWktParentheses(body: string): { valid: boolean; error?: string } {
  const trimmed = body.trim();
  if (trimmed.toUpperCase() === "EMPTY") {
    return { valid: true };
  }

  if (!trimmed.startsWith("(") || !trimmed.endsWith(")")) {
    return {
      valid: false,
      error: "Invalid WKT: coordinates must be enclosed in parentheses or specified as EMPTY",
    };
  }

  let depth = 0;
  for (let i = 0; i < trimmed.length; i++) {
    const ch = trimmed[i];
    if (ch === "(") {
      depth++;
    } else if (ch === ")") {
      depth--;
      if (depth < 0) {
        return { valid: false, error: "Invalid WKT: unexpected closing parenthesis" };
      }
      if (depth === 0 && i < trimmed.length - 1) {
        return {
          valid: false,
          error: "Invalid WKT: trailing content after closing parenthesis",
        };
      }
    }
  }

  if (depth !== 0) {
    return { valid: false, error: "Invalid WKT: unclosed parenthesis" };
  }

  return { valid: true };
}

function extractParens(str: string): string {
  const open = str.indexOf("(");
  const close = str.lastIndexOf(")");
  if (open !== -1 && close !== -1 && close > open) {
    return str.slice(open + 1, close).trim();
  }
  return str.trim();
}

function extractRings(str: string): string[] {
  const inner = extractParens(str);
  const rings: string[] = [];
  let depth = 0;
  let start = -1;

  for (let i = 0; i < inner.length; i++) {
    if (inner[i] === "(") {
      if (depth === 0) start = i + 1;
      depth++;
    } else if (inner[i] === ")") {
      depth--;
      if (depth === 0 && start !== -1) {
        rings.push(inner.slice(start, i).trim());
        start = -1;
      }
    }
  }

  if (rings.length === 0 && inner.length > 0) {
    rings.push(inner.trim());
  }

  return rings;
}

function extractPolyBlocks(str: string): string[] {
  const inner = extractParens(str);
  const blocks: string[] = [];
  let depth = 0;
  let start = -1;

  for (let i = 0; i < inner.length; i++) {
    if (inner[i] === "(") {
      if (depth === 0) start = i + 1;
      depth++;
    } else if (inner[i] === ")") {
      depth--;
      if (depth === 0 && start !== -1) {
        blocks.push(inner.slice(start, i).trim());
        start = -1;
      }
    }
  }

  return blocks;
}

/**
 * Splits top-level subgeometries inside a WKT GEOMETRYCOLLECTION body.
 */
function splitWktCollection(str: string): string[] {
  const inner = extractParens(str);
  const items: string[] = [];
  let depth = 0;
  let start = 0;

  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if (ch === "(") {
      depth++;
    } else if (ch === ")") {
      depth--;
    } else if (ch === "," && depth === 0) {
      const part = inner.slice(start, i).trim();
      if (part) items.push(part);
      start = i + 1;
    }
  }

  const last = inner.slice(start).trim();
  if (last) items.push(last);

  return items;
}

/**
 * Builds SVG viewBox bounds, path commands, and interactive vertex coordinates.
 */
export function buildSvgRenderData(geom: ParsedSpatialGeometry): SvgRenderData {
  // Safe handling for empty geometries (0 vertices)
  if (geom.vertexCount === 0) {
    return {
      viewBox: "0 0 100 100",
      paths: [],
      vertices: [],
      bbox: [0, 0, 0, 0],
      viewBoxBounds: { minX: 0, minY: 0, width: 100, height: 100 },
      emptyMessage: "Empty Geometry",
    };
  }

  const [minX, minY, maxX, maxY] = geom.bbox;

  let spanX = maxX - minX;
  let spanY = maxY - minY;

  // Single point or horizontal/vertical line margin protection
  if (spanX <= 0) spanX = 2;
  if (spanY <= 0) spanY = 2;

  const padX = spanX * 0.08;
  const padY = spanY * 0.08;

  const bMinX = minX - padX;
  const bMaxX = maxX + padX;
  const bMinY = minY - padY;
  const bMaxY = maxY + padY;

  const vWidth = bMaxX - bMinX;
  const vHeight = bMaxY - bMinY;

  // Coordinate projector: map world (x, y) to SVG (sx, sy) with inverted Y
  const project = (pt: Coordinate) => {
    const sx = pt.x - bMinX;
    const sy = bMaxY - pt.y; // Invert Y
    return { sx, sy };
  };

  const paths: SvgRenderData["paths"] = [];
  const vertices: SvgRenderData["vertices"] = [];

  // 1. Polygons
  if (geom.polygons) {
    for (const poly of geom.polygons) {
      let d = "";
      // Exterior ring
      if (poly.exteriorRing.length > 0) {
        poly.exteriorRing.forEach((pt, i) => {
          const { sx, sy } = project(pt);
          d += i === 0 ? `M ${sx} ${sy}` : ` L ${sx} ${sy}`;
          vertices.push({ cx: sx, cy: sy, worldX: pt.x, worldY: pt.y });
        });
        d += " Z";
      }
      // Interior rings (holes)
      for (const hole of poly.interiorRings) {
        if (hole.length > 0) {
          hole.forEach((pt, i) => {
            const { sx, sy } = project(pt);
            d += i === 0 ? ` M ${sx} ${sy}` : ` L ${sx} ${sy}`;
            vertices.push({ cx: sx, cy: sy, worldX: pt.x, worldY: pt.y });
          });
          d += " Z";
        }
      }

      if (d) {
        paths.push({
          d,
          type: "polygon",
          fillRule: "evenodd",
        });
      }
    }
  }

  // 2. Lines
  if (geom.lines) {
    for (const line of geom.lines) {
      let d = "";
      line.forEach((pt, i) => {
        const { sx, sy } = project(pt);
        d += i === 0 ? `M ${sx} ${sy}` : ` L ${sx} ${sy}`;
        vertices.push({ cx: sx, cy: sy, worldX: pt.x, worldY: pt.y });
      });
      if (d) {
        paths.push({ d, type: "line" });
      }
    }
  }

  // 3. Points
  if (geom.points) {
    for (const pt of geom.points) {
      const { sx, sy } = project(pt);
      vertices.push({ cx: sx, cy: sy, worldX: pt.x, worldY: pt.y });
    }
  }

  return {
    viewBox: `0 0 ${vWidth} ${vHeight}`,
    paths,
    vertices,
    bbox: geom.bbox,
    viewBoxBounds: { minX: bMinX, minY: bMinY, width: vWidth, height: vHeight },
  };
}
