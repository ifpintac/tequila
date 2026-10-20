"""Write rgb.mp4 for tests/video.rs: three seconds of 64 x 48 video at one
frame a second - red, then green, then blue - in H.264, as phones and
screen recorders write it, so the system's own decoder reads it.

    blender --background --factory-startup --python make_video.py
"""

import os

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))

scene = bpy.context.scene
scene.render.resolution_x = 64
scene.render.resolution_y = 48
scene.render.resolution_percentage = 100
scene.render.fps = 1
scene.render.fps_base = 1.0
scene.frame_start = 1
scene.frame_end = 3
# The colours as they are, not through a filmic look.
scene.view_settings.view_transform = 'Standard'
scene.view_settings.look = 'None'

scene.sequence_editor_create()
strips = scene.sequence_editor.strips if hasattr(scene.sequence_editor, 'strips') else scene.sequence_editor.sequences
for frame, colour in [(1, (1.0, 0.0, 0.0)), (2, (0.0, 1.0, 0.0)), (3, (0.0, 0.0, 1.0))]:
    strip = strips.new_effect(name=f'frame{frame}', type='COLOR', channel=1, frame_start=frame, length=1)
    strip.color = colour

render = scene.render
if hasattr(render.image_settings, 'media_type'):
    render.image_settings.media_type = 'VIDEO'
render.image_settings.file_format = 'FFMPEG'
render.ffmpeg.format = 'MPEG4'
render.ffmpeg.codec = 'H264'
render.ffmpeg.constant_rate_factor = 'HIGH'
render.ffmpeg.gopsize = 1
render.ffmpeg.audio_codec = 'NONE'
render.use_file_extension = False
render.filepath = os.path.join(HERE, 'rgb.mp4')
bpy.ops.render.render(animation=True)
print('wrote', render.filepath, os.path.getsize(render.filepath))
