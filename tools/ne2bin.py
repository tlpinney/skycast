"""Convert Natural Earth GeoJSON line/polygon files to a compact polyline file.
Format (little endian): u32 n_lines, then per line: u32 n_points, n_points * (f32 lon, f32 lat)."""
import json, struct, sys
out, inputs = sys.argv[1], sys.argv[2:]
lines = []
def add(coords):
    if len(coords) >= 2:
        lines.append(coords)
for path in inputs:
    for feat in json.load(open(path))["features"]:
        g = feat["geometry"]
        if g is None: continue
        t, c = g["type"], g["coordinates"]
        if t == "LineString": add(c)
        elif t == "MultiLineString": [add(l) for l in c]
        elif t == "Polygon": [add(r) for r in c]
        elif t == "MultiPolygon": [add(r) for p in c for r in p]
with open(out, "wb") as f:
    f.write(struct.pack("<I", len(lines)))
    for l in lines:
        f.write(struct.pack("<I", len(l)))
        for x, y, *_ in l:
            f.write(struct.pack("<ff", x, y))
print(out, len(lines), "lines", sum(len(l) for l in lines), "points")
