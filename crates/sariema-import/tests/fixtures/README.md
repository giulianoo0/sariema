# Test fixtures

## `montagem_min.json`, `luz_min.json`, `sala_min.json`

Hand-written, in the montage (`montagem.json`) and light (`luz.json`) formats the importer reads,
over `kit_min.opt.glb`. Room `a` (corner at the origin) holds two `caixa` and a
`mesa`, lit by a spot and a point light; room `b`, 2 cells (3 m) east, holds the
`lampada` and has no local lights. `sala_min.json` is the room description that
shows both, validated against `schemas/room.schema.json`.

## `kit_min.opt.glb`

A three-piece kit, made for these tests:

- `caixa`: a 1 m box, 12 triangles.
- `mesa`: a node without a mesh. Its only child, `mesa_tampo`, is a 1 m box raised 1 m.
- `lampada`: a 1 m box with `extras.oculto_no_inicio = true`.

All boxes use the material `mat0`: a 4×4 PNG mapped through `KHR_texture_transform`
with scale 2. Each box has its own vertex colour, which also keeps gltfpack from
merging the three identical boxes into one shared mesh.

To rebuild it, save the script below as `kit_min.py` in an empty directory and run,
from that directory (Blender 5.3, gltfpack 1.3):

```sh
blender -b --factory-startup --python kit_min.py -- "$PWD/kit_min.gltf"
npx --yes gltfpack -i kit_min.gltf -o kit_min.opt.glb -cc -kn -ke -km -vtf
```

`-vtf` keeps texture coordinates as floats. Quantized texture coordinates make
gltfpack fold its dequantization into the material's `KHR_texture_transform`, which
would replace the authored scale of 2. The output still requires
`EXT_meshopt_compression` (octahedral and exponential filters) and
`KHR_mesh_quantization`.

```python
import json
import os
import sys

import bmesh
import bpy

out = sys.argv[sys.argv.index("--") + 1]

bpy.ops.wm.read_factory_settings(use_empty=True)

img = bpy.data.images.new("xadrez", 4, 4)
img.pixels = [
    c
    for y in range(4)
    for x in range(4)
    for c in ((0.9, 0.9, 0.9, 1.0) if (x + y) % 2 == 0 else (0.2, 0.3, 0.8, 1.0))
]
img.filepath_raw = os.path.join(os.path.dirname(out), "xadrez.png")
img.file_format = "PNG"
img.save()

mat = bpy.data.materials.new("mat0")
nodes, links = mat.node_tree.nodes, mat.node_tree.links
bsdf = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
tex = nodes.new("ShaderNodeTexImage")
tex.image = img
mapping = nodes.new("ShaderNodeMapping")
mapping.inputs["Scale"].default_value = (2.0, 2.0, 1.0)
coord = nodes.new("ShaderNodeTexCoord")
links.new(coord.outputs["UV"], mapping.inputs["Vector"])
links.new(mapping.outputs["Vector"], tex.inputs["Vector"])
links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])


def box(name, color):
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bm.loops.layers.uv.new("UVMap")
    bmesh.ops.create_cube(bm, size=1.0, calc_uvs=True)
    bm.to_mesh(me)
    bm.free()
    attr = me.color_attributes.new("Color", "BYTE_COLOR", "CORNER")
    for d in attr.data:
        d.color = color
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


box("caixa", (1.0, 0.5, 0.5, 1.0))
mesa = bpy.data.objects.new("mesa", None)
bpy.context.scene.collection.objects.link(mesa)
tampo = box("mesa_tampo", (0.5, 1.0, 0.5, 1.0))
tampo.parent = mesa
tampo.location = (0.0, 0.0, 1.0)
lampada = box("lampada", (1.0, 1.0, 0.5, 1.0))
lampada["oculto_no_inicio"] = True

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLTF_SEPARATE",
    export_extras=True,
    export_vertex_color="ACTIVE",
    export_animations=False,
)

# The exporter sorts root nodes by name; the tests expect authoring order.
with open(out) as f:
    gltf = json.load(f)
names = [n["name"] for n in gltf["nodes"]]
gltf["scenes"][0]["nodes"] = [names.index(n) for n in ("caixa", "mesa", "lampada")]
with open(out, "w") as f:
    json.dump(gltf, f, indent=2)
```
