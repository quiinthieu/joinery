import { describe, expect, it } from "vitest";
import { parseSpatialGeometry, buildSvgRenderData } from "./spatial-format";

describe("spatial-format", () => {
  it("parses WKT POINT and computes bounding box", () => {
    const res = parseSpatialGeometry("POINT (30 10)");
    expect(res.ok).toBe(true);
    expect(res.geometry?.kind).toBe("POINT");
    expect(res.geometry?.vertexCount).toBe(1);
    expect(res.geometry?.bbox).toEqual([30, 10, 30, 10]);

    if (res.geometry) {
      const svg = buildSvgRenderData(res.geometry);
      expect(svg.vertices).toHaveLength(1);
      expect(svg.viewBoxBounds.width).toBeGreaterThan(0);
    }
  });

  it("parses WKT LINESTRING", () => {
    const res = parseSpatialGeometry("LINESTRING (30 10, 10 30, 40 40)");
    expect(res.ok).toBe(true);
    expect(res.geometry?.kind).toBe("LINESTRING");
    expect(res.geometry?.vertexCount).toBe(3);
    expect(res.geometry?.bbox).toEqual([10, 10, 40, 40]);
  });

  it("parses WKT POLYGON and verifies SVG path commands", () => {
    const res = parseSpatialGeometry("POLYGON ((30 10, 40 40, 20 40, 10 20, 30 10))");
    expect(res.ok).toBe(true);
    expect(res.geometry?.kind).toBe("POLYGON");
    expect(res.geometry?.vertexCount).toBe(5);
    expect(res.geometry?.bbox).toEqual([10, 10, 40, 40]);

    if (res.geometry) {
      const svg = buildSvgRenderData(res.geometry);
      expect(svg.paths).toHaveLength(1);
      expect(svg.paths[0].d).toMatch(/^M\s+[\d.]+\s+[\d.]+/);
      expect(svg.paths[0].d).toContain("Z");
      expect(svg.paths[0].fillRule).toBe("evenodd");
    }
  });

  it("parses WKT with SRID prefix", () => {
    const res = parseSpatialGeometry("SRID=4326;POINT (121.5 25.0)");
    expect(res.ok).toBe(true);
    expect(res.geometry?.srid).toBe(4326);
    expect(res.geometry?.points?.[0]).toEqual({ x: 121.5, y: 25.0 });
  });

  it("parses GeoJSON Point and Polygon objects", () => {
    const geoPoint = '{"type": "Point", "coordinates": [100.0, 0.0]}';
    const resPoint = parseSpatialGeometry(geoPoint);
    expect(resPoint.ok).toBe(true);
    expect(resPoint.geometry?.kind).toBe("POINT");
    expect(resPoint.geometry?.points?.[0]).toEqual({ x: 100.0, y: 0.0 });

    const geoPoly =
      '{"type": "Polygon", "coordinates": [[[30, 10], [40, 40], [20, 40], [10, 20], [30, 10]]]}';
    const resPoly = parseSpatialGeometry(geoPoly);
    expect(resPoly.ok).toBe(true);
    expect(resPoly.geometry?.kind).toBe("POLYGON");
    expect(resPoly.geometry?.vertexCount).toBe(5);
  });
});
