"""Writes pixar_written.usdc: a layer Pixar's own USD library wrote, for
`tests/usdz.rs` to check Tequila's crate reader against. Run it with the
Python Blender bundles USD in (Blender 5.2 here):

    blender -b --factory-startup --python make_pixar_written.py

The layer has what Tequila's USDZ stage has and what other tools write
beside: compressed integer and float arrays (both codings), a matrix, an
inlined vector, list ops (references, applied schemas, connections) and a
class prototype.
"""
import os
from pxr import Sdf

out = os.path.dirname(os.path.abspath(__file__))
text = '''#usda 1.0
(
    defaultPrim = "Product"
    doc = "Written by Tequila"
    metersPerUnit = 1
    upAxis = "Y"
)

def Xform "Product" (
    kind = "component"
)
{
    def Xform "Group"
    {
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0.25, 0.5, -1.5, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]

        def Mesh "Part" (
            prepend apiSchemas = ["MaterialBindingAPI"]
        )
        {
            float3[] extent = [(-1, -1, 0), (1, 1, 0.5)]
            int[] faceVertexCounts = [3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3]
            int[] faceVertexIndices = [0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3]
            rel material:binding = </Product/Materials/Red>
            normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1)] (
                interpolation = "vertex"
            )
            point3f[] points = [(-1, -1, 0), (1, -1, 0), (1, 1, 0.5), (-1, 1, 0.123)]
            texCoord2f[] primvars:st = [(0, 0), (1, 0), (1, 1), (0, 1)] (
                interpolation = "vertex"
            )
            float[] widths = [0.5, 1.5, 2, 3, 0.5, 1.5, 2, 3, 0.5, 1.5, 2, 3, 0.5, 1.5, 2, 3, 0.5, 1.5, 2, 3]
            float[] counts = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
            uniform token subdivisionScheme = "none"
            uniform bool doubleSided = 1
            token visibility = "invisible"
        }
    }

    def Xform "Bolt_1" (
        instanceable = true
        prepend references = </Prototypes/Bolt>
    )
    {
        rel material:binding = </Product/Materials/Red>
        double3 xformOp:translate = (1, 2, 3)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }

    def Scope "Materials"
    {
        def Material "Red"
        {
            token outputs:surface.connect = </Product/Materials/Red/Surface.outputs:surface>
            custom string tequila:definition = "{\\"a\\": 1}"

            def Shader "Surface"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor.connect = </Product/Materials/Red/Colour.outputs:rgb>
                float inputs:roughness = 0.4
                float inputs:metallic = 0.123456
                float inputs:ior = 1.5
                color3f inputs:emissiveColor = (0, 0, 0)
                normal3f inputs:normal.connect = </Product/Materials/Red/Normal.outputs:rgb>
                token outputs:surface
            }

            def Shader "Colour"
            {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @textures/red.png@
                float2 inputs:st.connect = </Product/Materials/Red/UV.outputs:result>
                token inputs:sourceColorSpace = "sRGB"
                token inputs:wrapS = "repeat"
                float3 outputs:rgb
                float outputs:a
            }

            def Shader "Normal"
            {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @textures/normal.png@
                float4 inputs:scale = (2, 2, 2, 1)
                float4 inputs:bias = (-1, -1, -1, 0)
                float2 inputs:st.connect = </Product/Materials/Red/UV.outputs:result>
                token inputs:sourceColorSpace = "raw"
                float3 outputs:rgb
            }

            def Shader "UV"
            {
                uniform token info:id = "UsdPrimvarReader_float2"
                string inputs:varname = "st"
                float2 outputs:result
            }
        }
    }
}

class Xform "Prototypes"
{
    def Xform "Bolt"
    {
        def Mesh "Bolt"
        {
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        }
    }
}
'''
layer = Sdf.Layer.CreateAnonymous('pixar_written.usda')
layer.ImportFromString(text)
layer.Export(os.path.join(out, 'pixar_written.usdc'))
