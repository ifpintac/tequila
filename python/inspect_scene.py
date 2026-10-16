"""Read-only starting point: python inspect_scene.py > scene-context.json"""
import json
from tequila import Client

with Client() as fs:
    print(json.dumps(fs.context(), indent=2))
