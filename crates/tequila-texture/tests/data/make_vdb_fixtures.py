"""Write small OpenVDB fixtures with known contents, using the OpenVDB that
ships inside Blender. Run as: blender --background --factory-startup --python make_fixtures.py -- OUTDIR"""
import sys, os, math
import openvdb as vdb

out = sys.argv[sys.argv.index('--') + 1]
os.makedirs(out, exist_ok=True)

def ramp():
    grid = vdb.FloatGrid()
    grid.name = 'density'
    grid.gridClass = vdb.GridClass.FOG_VOLUME
    grid.transform = vdb.createLinearTransform(voxelSize=0.05)
    acc = grid.getAccessor()
    for i in range(32):
        for j in range(32):
            for k in range(32):
                acc.setValueOn((i - 8, j, k - 4), (i - 8 + 8) * 0.01 + 0.001)
    return grid

def sphere():
    grid = vdb.FloatGrid()
    grid.name = 'density'
    grid.gridClass = vdb.GridClass.FOG_VOLUME
    grid.transform = vdb.createLinearTransform(voxelSize=0.1)
    acc = grid.getAccessor()
    for i in range(-12, 13):
        for j in range(-12, 13):
            for k in range(-12, 13):
                r = math.sqrt(i * i + j * j + k * k)
                if r <= 10.0:
                    acc.setValueOn((i, j, k), 1.0)
    return grid

g = ramp()
vdb.write(os.path.join(out, 'ramp.vdb'), grids=[g])
g.saveFloatAsHalf = True
vdb.write(os.path.join(out, 'ramp_half.vdb'), grids=[g])
print('FIXTURES written', sorted(os.listdir(out)))
s = sphere()
vdb.write(os.path.join(out, 'sphere.vdb'), grids=[s])
print('FIXTURES', {f: os.path.getsize(os.path.join(out, f)) for f in os.listdir(out)})
print('SAVE-OPTS', [n for n in dir(vdb) if 'ompress' in n or 'write' in n.lower()])
