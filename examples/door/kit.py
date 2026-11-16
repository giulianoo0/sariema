# SPDX-License-Identifier: Apache-2.0
"""Writes kit.glb, the door example's kit: plain boxes, one material each.

    python3 kit.py

Pieces: floor_tile (1 m square, top at y 0), wall (1 m long, 2.2 m high),
low_wall (0.9 m high), and door: a lintel over a 1 m doorway and its leaf,
door_leaf, a node hinged on the doorway's -x jamb about a vertical axis.
"""

import json
import os
import struct

MATERIALS = [
    ("floor", [0.62, 0.50, 0.36, 1.0]),
    ("wall", [0.86, 0.84, 0.79, 1.0]),
    ("wood", [0.42, 0.25, 0.13, 1.0]),
]

FACES = [
    ((1, 0, 0), lambda lo, hi: [(hi[0], lo[1], hi[2]), (hi[0], lo[1], lo[2]), (hi[0], hi[1], lo[2]), (hi[0], hi[1], hi[2])]),
    ((-1, 0, 0), lambda lo, hi: [(lo[0], lo[1], lo[2]), (lo[0], lo[1], hi[2]), (lo[0], hi[1], hi[2]), (lo[0], hi[1], lo[2])]),
    ((0, 1, 0), lambda lo, hi: [(lo[0], hi[1], hi[2]), (hi[0], hi[1], hi[2]), (hi[0], hi[1], lo[2]), (lo[0], hi[1], lo[2])]),
    ((0, -1, 0), lambda lo, hi: [(lo[0], lo[1], lo[2]), (hi[0], lo[1], lo[2]), (hi[0], lo[1], hi[2]), (lo[0], lo[1], hi[2])]),
    ((0, 0, 1), lambda lo, hi: [(lo[0], lo[1], hi[2]), (hi[0], lo[1], hi[2]), (hi[0], hi[1], hi[2]), (lo[0], hi[1], hi[2])]),
    ((0, 0, -1), lambda lo, hi: [(hi[0], lo[1], lo[2]), (lo[0], lo[1], lo[2]), (lo[0], hi[1], lo[2]), (hi[0], hi[1], lo[2])]),
]


class Glb:
    def __init__(self):
        self.bin = bytearray()
        self.views, self.accessors, self.meshes = [], [], []

    def view(self, data, target):
        while len(self.bin) % 4:
            self.bin.append(0)
        self.views.append({"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(data), "target": target})
        self.bin += data
        return len(self.views) - 1

    def box(self, lo, hi, material):
        positions, normals, indices = [], [], []
        for normal, corners in FACES:
            base = len(positions)
            positions += corners(lo, hi)
            normals += [normal] * 4
            indices += [base, base + 1, base + 2, base, base + 2, base + 3]
        flat = lambda vs: b"".join(struct.pack("<3f", *v) for v in vs)
        pos = self.view(flat(positions), 34962)
        nor = self.view(flat(normals), 34962)
        idx = self.view(b"".join(struct.pack("<H", i) for i in indices), 34963)
        self.accessors += [
            {"bufferView": pos, "componentType": 5126, "count": len(positions), "type": "VEC3",
             "min": list(lo), "max": list(hi)},
            {"bufferView": nor, "componentType": 5126, "count": len(normals), "type": "VEC3"},
            {"bufferView": idx, "componentType": 5123, "count": len(indices), "type": "SCALAR"},
        ]
        a = len(self.accessors)
        self.meshes.append({"primitives": [{
            "attributes": {"POSITION": a - 3, "NORMAL": a - 2},
            "indices": a - 1,
            "material": [m[0] for m in MATERIALS].index(material),
        }]})
        return len(self.meshes) - 1


def main():
    g = Glb()
    nodes = [
        {"name": "floor_tile", "mesh": g.box((-0.5, -0.1, -0.5), (0.5, 0.0, 0.5), "floor")},
        {"name": "wall", "mesh": g.box((-0.5, 0.0, -0.075), (0.5, 2.2, 0.075), "wall")},
        {"name": "low_wall", "mesh": g.box((-0.5, 0.0, -0.075), (0.5, 0.9, 0.075), "wall")},
        {"name": "door", "children": [4, 5]},
        {"name": "door_lintel", "mesh": g.box((-0.5, 2.0, -0.075), (0.5, 2.2, 0.075), "wall")},
        {"name": "door_leaf", "translation": [-0.5, 0.0, 0.0],
         "extras": {"giro": {"eixo": "vertical"}},
         "mesh": g.box((0.02, 0.0, -0.03), (0.98, 1.98, 0.03), "wood")},
    ]
    doc = {
        "asset": {"version": "2.0", "generator": "examples/door/kit.py"},
        "scene": 0,
        "scenes": [{"nodes": [0, 1, 2, 3]}],
        "nodes": nodes,
        "meshes": g.meshes,
        "materials": [
            {"name": name, "pbrMetallicRoughness": {"baseColorFactor": color, "metallicFactor": 0.0, "roughnessFactor": 0.85}}
            for name, color in MATERIALS
        ],
        "accessors": g.accessors,
        "bufferViews": g.views,
        "buffers": [{"byteLength": len(g.bin)}],
    }
    js = json.dumps(doc, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    bin_ = bytes(g.bin) + b"\0" * (-len(g.bin) % 4)
    total = 12 + 8 + len(js) + 8 + len(bin_)
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "kit.glb")
    with open(out, "wb") as f:
        f.write(struct.pack("<4sII", b"glTF", 2, total))
        f.write(struct.pack("<I4s", len(js), b"JSON") + js)
        f.write(struct.pack("<I4s", len(bin_), b"BIN\0") + bin_)


if __name__ == "__main__":
    main()
