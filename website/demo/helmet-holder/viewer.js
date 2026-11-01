// Tequila web viewer: a product configurator's live 3D view and photos,
// with no library and nothing fetched. Everything arrives in data.js as
// window.TEQUILA, written by Tequila's web export.
//
// The 3D view is a small WebGL2 physically based renderer for the glTF the
// export writes: metallic-roughness with glTF's transmission, IOR,
// clearcoat, sheen, specular and emissive-strength extensions, lit by the
// scene's environment as Tequila prefiltered it - a mip chain for
// reflections at each roughness, spherical harmonics for diffuse light -
// with a baked soft shadow on the ground. An approximation of the
// path-traced render, as the export's photos are the real thing.
(function () {
  'use strict';
  const D = window.TEQUILA;
  const message = document.getElementById('message');
  if (!D) { message.textContent = 'This folder is missing data.js.'; return; }
  const stage = document.getElementById('stage');
  const options = D.options || [];
  const rules = D.rules || [];
  let current = (D.current || []).slice();

  // ---- the configurator's rules, as Tequila applies them ---------------
  const indexOf = (id) => options.findIndex((o) => o.id === id);
  function clash(i, a, j, b) {
    const oi = options[i].id, oj = options[j].id;
    const breaks = (r, f, fc, s, sc) => r.option === f && r.choice === fc && r.other === s &&
      (r.kind === 'excludes' ? r.other_choice === sc : r.other_choice !== sc);
    return rules.some((r) => breaks(r, oi, a, oj, b) || breaks(r, oj, b, oi, a));
  }
  // The configuration nearest `from` with option i set to c, or null.
  function resolve(from, i, c) {
    const order = [i].concat(options.map((_, k) => k).filter((k) => k !== i));
    const assigned = new Array(options.length).fill(-1);
    const prefer = (k) => {
      if (k === i) return [c];
      const n = options[k].choices.length, keep = from[k];
      const out = keep >= 0 && keep < n ? [keep] : [];
      for (let x = 0; x < n; x++) if (x !== keep) out.push(x);
      return out;
    };
    const search = (depth) => {
      if (depth === order.length) return true;
      const k = order[depth];
      for (const x of prefer(k)) {
        if (assigned.every((b, j) => b < 0 || !clash(k, x, j, b))) {
          assigned[k] = x;
          if (search(depth + 1)) return true;
          assigned[k] = -1;
        }
      }
      return false;
    };
    return search(0) ? assigned.slice() : null;
  }

  // ---- the picker ----------------------------------------------------------
  const panel = document.createElement('div');
  panel.className = 'panel ' + (D.placement || 'right');
  document.body.appendChild(panel);
  let onChange = () => {};
  function choose(i, c) {
    if (c === current[i]) return;
    const next = resolve(current, i, c);
    if (!next) return;
    current = next;
    drawPicker();
    onChange();
  }
  function drawPicker() {
    panel.innerHTML = '';
    const head = document.createElement('div');
    head.className = 'head';
    if (D.title) { const h = document.createElement('h1'); h.textContent = D.title; head.appendChild(h); }
    if (D.subtitle) { const s = document.createElement('div'); s.className = 'sub'; s.textContent = D.subtitle; head.appendChild(s); }
    if (D.title || D.subtitle) panel.appendChild(head);
    options.forEach((option, i) => {
      const box = document.createElement('div');
      box.className = 'option';
      const name = document.createElement('div');
      name.className = 'name';
      name.textContent = option.name;
      const chosen = option.choices[current[i]];
      if (chosen) { const s = document.createElement('span'); s.textContent = chosen.label; name.appendChild(s); }
      box.appendChild(name);
      const blocked = option.choices.map((_, c) => resolve(current, i, c) === null);
      const consequence = (c) => {
        const next = resolve(current, i, c);
        if (!next) return option.choices[c].label + ' - not available';
        const moved = options.map((o, k) => (k !== i && next[k] !== current[k]) ? o.name + ' to ' + o.choices[next[k]].label : null).filter(Boolean);
        return option.choices[c].label + (moved.length ? '\nAlso changes ' + moved.join(', ') : '');
      };
      if (option.style === 'list') {
        const select = document.createElement('select');
        option.choices.forEach((choice, c) => {
          const o = document.createElement('option');
          o.value = c; o.textContent = choice.label; o.disabled = blocked[c]; o.selected = c === current[i];
          select.appendChild(o);
        });
        select.addEventListener('change', () => choose(i, +select.value));
        box.appendChild(select);
      } else {
        const row = document.createElement('div');
        row.className = option.style === 'swatches' ? 'swatches' : 'choices';
        option.choices.forEach((choice, c) => {
          const b = document.createElement('button');
          if (option.style === 'swatches' && choice.swatch) {
            b.className = 'swatch';
            b.style.background = choice.swatch;
            b.setAttribute('aria-label', choice.label);
          } else {
            b.className = 'choice';
            b.textContent = choice.label;
          }
          if (c === current[i]) b.classList.add('on');
          if (blocked[c]) { b.classList.add('off'); b.disabled = true; }
          b.title = consequence(c);
          b.addEventListener('click', () => choose(i, c));
          row.appendChild(b);
        });
        box.appendChild(row);
      }
      panel.appendChild(box);
    });
    if (!options.length && !D.title && !D.subtitle) panel.style.display = 'none';
  }

  // ---- photos --------------------------------------------------------------
  const photo = D.images ? document.createElement('img') : null;
  function showPhoto() {
    if (!photo) return;
    const src = D.images[current.join('-')];
    if (!src) return;
    const next = new Image();
    next.onload = () => { photo.style.opacity = 0; setTimeout(() => { photo.src = src; photo.style.opacity = 1; }, 120); };
    next.onerror = () => { message.textContent = 'This combination has not been rendered yet.'; };
    next.src = src;
    message.textContent = '';
  }

  // ---- the 3D view ---------------------------------------------------------
  let view = null;
  if (D.glb) {
    try { view = startView(); } catch (e) { console.error(e); view = null; }
  }

  // Which is shown: the 3D view, or the photos, or a switch between them.
  const tools = document.createElement('div');
  tools.className = 'tools';
  const reset = document.createElement('button');
  reset.textContent = 'Reset view';
  if (view && photo) {
    const live = document.createElement('button'); live.textContent = '3D';
    const still = document.createElement('button'); still.textContent = 'Photo';
    const set = (three) => {
      view.canvas.style.display = three ? 'block' : 'none';
      photo.style.display = three ? 'none' : 'block';
      live.classList.toggle('on', three); still.classList.toggle('on', !three);
      reset.style.display = three ? '' : 'none';
      if (!three) showPhoto();
    };
    live.onclick = () => set(true); still.onclick = () => set(false);
    tools.append(live, still);
    stage.appendChild(photo);
    set(true);
  } else if (photo) {
    stage.appendChild(photo);
  }
  if (view) {
    reset.onclick = () => view.reset();
    tools.appendChild(reset);
  }
  if (tools.children.length) document.body.appendChild(tools);

  onChange = () => { if (view) view.configure(current); showPhoto(); };
  drawPicker();
  onChange();
  if (!view && !photo) message.textContent = D.glb ? 'This browser cannot show the 3D view (WebGL 2 is needed).' : 'Nothing to show.';
  else if (view) message.textContent = '';

  function startView() {
    const canvas = document.createElement('canvas');
    stage.appendChild(canvas);
    const gl = canvas.getContext('webgl2', { antialias: true, alpha: false, premultipliedAlpha: false });
    if (!gl) { canvas.remove(); return null; }

    // ---- the glTF ------------------------------------------------------
    const raw = atob(D.glb);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
    const dv = new DataView(bytes.buffer);
    const jsonLength = dv.getUint32(12, true);
    const gltf = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + jsonLength)));
    const binStart = 20 + jsonLength + 8;
    const viewBytes = (i) => {
      const v = gltf.bufferViews[i];
      return bytes.subarray(binStart + (v.byteOffset || 0), binStart + (v.byteOffset || 0) + v.byteLength);
    };
    const accessor = (i) => {
      const a = gltf.accessors[i];
      const b = viewBytes(a.bufferView);
      const width = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 }[a.type];
      const Kind = a.componentType === 5125 ? Uint32Array : Float32Array;
      return { data: new Kind(b.buffer.slice(b.byteOffset, b.byteOffset + a.count * width * 4)), width, count: a.count };
    };

    // Textures: decoded from the binary chunk; base colour and emission are
    // sRGB, everything else is numbers.
    const textures = [];
    const pending = [];
    const srgbTextures = new Set();
    (gltf.materials || []).forEach((m) => {
      const p = m.pbrMetallicRoughness || {};
      if (p.baseColorTexture) srgbTextures.add(p.baseColorTexture.index);
      if (m.emissiveTexture) srgbTextures.add(m.emissiveTexture.index);
    });
    (gltf.textures || []).forEach((t, i) => {
      const image = gltf.images[t.source];
      const sampler = (gltf.samplers || [])[t.sampler] || {};
      const tex = gl.createTexture();
      textures[i] = tex;
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 255, 255, 255]));
      const blob = new Blob([viewBytes(image.bufferView)], { type: image.mimeType });
      pending.push(createImageBitmap(blob, { colorSpaceConversion: 'none', premultiplyAlpha: 'none' }).then((bitmap) => {
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
        gl.texImage2D(gl.TEXTURE_2D, 0, srgbTextures.has(i) ? gl.SRGB8_ALPHA8 : gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, bitmap);
        const mip = sampler.minFilter === 9987;
        if (mip) gl.generateMipmap(gl.TEXTURE_2D);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, mip ? gl.LINEAR_MIPMAP_LINEAR : gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        const wrap = sampler.wrapS === 33071 ? gl.CLAMP_TO_EDGE : gl.REPEAT;
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrap);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrap);
        draw();
      }));
    });

    // Meshes: one vertex array per primitive.
    const meshes = (gltf.meshes || []).map((mesh) => mesh.primitives.map((p) => {
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const attribute = (name, location, fallback) => {
        if (p.attributes[name] === undefined) { gl.disableVertexAttribArray(location); gl.vertexAttrib4f(location, ...fallback); return; }
        const a = accessor(p.attributes[name]);
        const buffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
        gl.bufferData(gl.ARRAY_BUFFER, a.data, gl.STATIC_DRAW);
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, a.width, gl.FLOAT, false, 0, 0);
      };
      attribute('POSITION', 0, [0, 0, 0, 1]);
      attribute('NORMAL', 1, [0, 1, 0, 0]);
      attribute('TEXCOORD_0', 2, [0, 0, 0, 0]);
      attribute('TEXCOORD_1', 3, [0, 0, 0, 0]);
      attribute('COLOR_0', 4, [1, 1, 1, 1]);
      const index = accessor(p.indices);
      const ib = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ib);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, index.data, gl.STATIC_DRAW);
      gl.bindVertexArray(null);
      const pos = gltf.accessors[p.attributes.POSITION];
      return { vao, count: index.count, material: p.material, min: pos.min, max: pos.max };
    }));
    const nodes = (gltf.nodes || []).map((n) => ({
      matrix: n.matrix ? new Float32Array(n.matrix) : identity(),
      mesh: n.mesh,
      visible: true,
      materials: (gltf.meshes[n.mesh] || { primitives: [] }).primitives.map((p) => p.material),
    }));

    // ---- shaders -------------------------------------------------------
    const surfaceVS = `#version 300 es
      layout(location=0) in vec3 aPos; layout(location=1) in vec3 aNor; layout(location=2) in vec2 aUv0; layout(location=3) in vec2 aUv1; layout(location=4) in vec4 aCol;
      uniform mat4 uModel, uViewProj; uniform mat3 uNormal;
      out vec3 vWorld; out vec3 vNormal; out vec2 vUv0; out vec2 vUv1; out vec4 vCol;
      void main() { vec4 w = uModel * vec4(aPos, 1.0); vWorld = w.xyz; vNormal = uNormal * aNor; vUv0 = aUv0; vUv1 = aUv1; vCol = aCol; gl_Position = uViewProj * w; }`;
    const common = `
      const float PI = 3.14159265;
      uniform sampler2D uEnv; uniform float uLevels, uRgbm, uEnvRot, uEnvGain, uExposure; uniform vec3 uSH[9]; uniform int uDisplay;
      vec3 turn(vec3 d) { float c = cos(uEnvRot), s = sin(uEnvRot); return vec3(c * d.x + s * d.z, d.y, -s * d.x + c * d.z); }
      vec2 equirect(vec3 d) { return vec2(0.5 + atan(d.x, -d.z) / (2.0 * PI), acos(clamp(d.y, -1.0, 1.0)) / PI); }
      vec3 env(vec3 d, float rough) { vec4 c = textureLod(uEnv, equirect(turn(normalize(d))), rough * (uLevels - 1.0)); return c.rgb * c.a * uRgbm * uEnvGain; }
      vec3 irradiance(vec3 n) {
        n = turn(n);
        const float c1 = 0.429043, c2 = 0.511664, c3 = 0.743125, c4 = 0.886227, c5 = 0.247708;
        vec3 e = c4 * uSH[0] + 2.0 * c2 * (uSH[3] * n.x + uSH[1] * n.y + uSH[2] * n.z)
          + c3 * uSH[6] * n.z * n.z - c5 * uSH[6] + c1 * uSH[8] * (n.x * n.x - n.y * n.y)
          + 2.0 * c1 * (uSH[4] * n.x * n.y + uSH[7] * n.x * n.z + uSH[5] * n.y * n.z);
        return max(e, vec3(0.0)) * uEnvGain;
      }
      // Tequila's display transforms: Khronos PBR Neutral (0), ACES by
      // the Narkowicz fit on a 0.6-scaled input (1), clipped sRGB (2).
      vec3 neutral(vec3 c) {
        const float start = 0.8 - 0.04, desaturation = 0.15;
        float x = min(c.r, min(c.g, c.b));
        float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
        c -= offset;
        float peak = max(c.r, max(c.g, c.b));
        if (peak < start) return c;
        float d = 1.0 - start;
        float next = 1.0 - d * d / (peak + d - start);
        c *= next / peak;
        float g = 1.0 - 1.0 / (desaturation * (peak - next) + 1.0);
        return mix(c, vec3(next), g);
      }
      vec3 tone(vec3 c) {
        c = max(c * exp2(uExposure), vec3(0.0));
        if (uDisplay == 1) { c *= 0.6; c = (c * (2.51 * c + 0.03)) / (c * (2.43 * c + 0.59) + 0.14); }
        else if (uDisplay == 0) c = neutral(c);
        c = clamp(c, 0.0, 1.0);
        return mix(12.92 * c, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
      }`;
    const surfaceFS = `#version 300 es
      precision highp float;
      in vec3 vWorld; in vec3 vNormal; in vec2 vUv0; in vec2 vUv1; in vec4 vCol;
      out vec4 outColor;
      uniform vec3 uEye;
      uniform vec4 uBase; uniform float uMetal, uRough; uniform vec3 uEmissive;
      uniform sampler2D uBaseTex, uMRTex, uEmTex; uniform int uHasBase, uHasMR, uHasEm, uBaseUv, uMRUv, uEmUv;
      uniform float uTransmission, uIor, uCoat, uCoatRough, uSheenRough, uSpecular; uniform vec3 uTint, uSheen, uSpecColor;
      ${common}
      vec2 brdf(float nov, float r) {
        const vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022); const vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
        vec4 q = r * c0 + c1; float a = min(q.x * q.x, exp2(-9.28 * nov)) * q.x + q.y;
        return vec2(-1.04, 1.04) * a + q.zw;
      }
      void main() {
        vec3 n = normalize(vNormal); if (!gl_FrontFacing) n = -n;
        vec3 v = normalize(uEye - vWorld); float nov = max(dot(n, v), 1e-4); vec3 r = reflect(-v, n);
        vec4 base = uBase * vCol;
        if (uHasBase == 1) base *= texture(uBaseTex, uBaseUv == 1 ? vUv1 : vUv0);
        float metal = uMetal, rough = uRough;
        if (uHasMR == 1) { vec4 mr = texture(uMRTex, uMRUv == 1 ? vUv1 : vUv0); rough *= mr.g; metal *= mr.b; }
        rough = clamp(rough, 0.03, 1.0);
        float f0d = pow((uIor - 1.0) / (uIor + 1.0), 2.0) * uSpecular;
        vec3 f0 = mix(vec3(f0d) * uSpecColor, base.rgb, metal);
        vec2 ab = brdf(nov, rough);
        vec3 fr = f0 * ab.x + ab.y;
        vec3 spec = env(r, rough) * fr;
        float opaque = (1.0 - metal) * (1.0 - uTransmission);
        vec3 diffuse = base.rgb * opaque * irradiance(n) / PI * (1.0 - fr);
        vec3 refracted = env(refract(-v, n, 1.0 / uIor), rough) * uTint * base.rgb * uTransmission * (1.0 - metal) * (1.0 - fr);
        vec3 colour = diffuse + spec + refracted;
        colour += uSheen * irradiance(n) / PI * (0.3 + 0.7 * pow(1.0 - nov, 2.0));
        if (uCoat > 0.0) {
          float fc = 0.04 + 0.96 * pow(1.0 - nov, 5.0);
          colour = colour * (1.0 - uCoat * fc) + env(r, clamp(uCoatRough, 0.03, 1.0)) * fc * uCoat;
        }
        vec3 glow = uEmissive; if (uHasEm == 1) glow *= texture(uEmTex, uEmUv == 1 ? vUv1 : vUv0).rgb;
        colour += glow;
        outColor = vec4(tone(colour), base.a);
      }`;
    const skyVS = `#version 300 es
      out vec2 vClip; void main() { vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2) * 2.0 - 1.0; vClip = p; gl_Position = vec4(p, 0.999, 1.0); }`;
    const skyFS = `#version 300 es
      precision highp float; in vec2 vClip; out vec4 outColor; uniform mat4 uInverse; uniform sampler2D uBackdrop;
      ${common}
      void main() {
        vec4 far = uInverse * vec4(vClip, 1.0, 1.0); vec4 near = uInverse * vec4(vClip, -1.0, 1.0);
        vec4 c = texture(uBackdrop, equirect(turn(normalize(far.xyz / far.w - near.xyz / near.w))));
        outColor = vec4(tone(c.rgb * c.a * uRgbm * uEnvGain), 1.0);
      }`;
    const shadowVS = `#version 300 es
      layout(location=0) in vec2 aCorner; uniform mat4 uViewProj; uniform vec2 uMin, uSize; uniform float uY; out vec2 vUv;
      void main() { vUv = aCorner; gl_Position = uViewProj * vec4(uMin.x + aCorner.x * uSize.x, uY, uMin.y + aCorner.y * uSize.y, 1.0); }`;
    const shadowFS = `#version 300 es
      precision highp float; in vec2 vUv; out vec4 outColor; uniform sampler2D uShadow; uniform float uStrength;
      void main() { outColor = vec4(0.0, 0.0, 0.0, texture(uShadow, vUv).a * uStrength); }`;

    function program(vs, fs) {
      const p = gl.createProgram();
      for (const [type, source] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
        const s = gl.createShader(type);
        gl.shaderSource(s, source);
        gl.compileShader(s);
        if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
        gl.attachShader(p, s);
      }
      gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
      const u = {};
      const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
      for (let i = 0; i < n; i++) {
        const info = gl.getActiveUniform(p, i);
        const name = info.name.replace(/\[0\]$/, '');
        u[name] = gl.getUniformLocation(p, info.name);
      }
      return { p, u };
    }
    const surface = program(surfaceVS, surfaceFS);
    const sky = program(skyVS, skyFS);
    const shade = program(shadowVS, shadowFS);

    // ---- the environment ---------------------------------------------
    const E = D.environment || { levels: [], sh: new Array(27).fill(0), rgbm: 16, background: { mode: 'color', color: [0.8, 0.8, 0.8] } };
    const envTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, envTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([40, 40, 40, 32]));
    let envLevels = 1;
    const levelImages = E.levels.map((src) => bitmap(src).catch(() => null));
    pending.push(Promise.all(levelImages).then((images) => {
      if (images.some((i) => !i)) return;
      gl.bindTexture(gl.TEXTURE_2D, envTex);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.texStorage2D(gl.TEXTURE_2D, images.length, gl.RGBA8, images[0].width, images[0].height);
      images.forEach((image, level) => gl.texSubImage2D(gl.TEXTURE_2D, level, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, image));
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, images.length - 1);
      envLevels = images.length;
      draw();
    }));
    let envRot = 0, envGain = 1, exposure = D.exposure || 0;
    const display = { neutral: 0, aces: 1, srgb: 2 }[D.display] || 0;
    // The backdrop, behind the product.
    const backdropTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, backdropTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([40, 40, 40, 32]));
    if (E.backdrop) {
      pending.push(bitmap(E.backdrop).then((image) => {
        gl.bindTexture(gl.TEXTURE_2D, backdropTex);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        draw();
      }).catch(() => {}));
    }

    // The ground shadow.
    let shadowTex = null;
    const quad = gl.createVertexArray();
    gl.bindVertexArray(quad);
    const qb = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, qb);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    if (D.shadow) {
      bitmap(D.shadow.image).then((i) => {
        shadowTex = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, shadowTex);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, i);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        draw();
      }).catch(() => {});
    }

    // ---- the camera: an orbit about a target ------------------------
    const cam = { target: [0, 0, 0], distance: 1, yaw: 0, pitch: 0, focal: 50, sensor: 36 };
    let home = null;
    function setCamera(c, animate) {
      const d = sub(c.position, c.target);
      const distance = Math.hypot(d[0], d[1], d[2]) || 1;
      const goal = { target: c.target.slice(), distance, yaw: Math.atan2(d[0], d[2]), pitch: Math.asin(clamp(d[1] / distance, -1, 1)), focal: c.focal || 50, sensor: c.sensor || 36 };
      if (!animate) { Object.assign(cam, goal); return; }
      const from = Object.assign({}, cam, { target: cam.target.slice() });
      let dy = goal.yaw - from.yaw; while (dy > Math.PI) dy -= 2 * Math.PI; while (dy < -Math.PI) dy += 2 * Math.PI;
      const start = performance.now();
      const step = (now) => {
        const t = Math.min(1, (now - start) / 650), e = t * t * (3 - 2 * t);
        cam.target = from.target.map((v, k) => v + (goal.target[k] - v) * e);
        cam.distance = from.distance * Math.pow(goal.distance / from.distance, e);
        cam.yaw = from.yaw + dy * e; cam.pitch = from.pitch + (goal.pitch - from.pitch) * e;
        cam.focal = from.focal + (goal.focal - from.focal) * e;
        draw();
        if (t < 1) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }
    if (D.camera) { setCamera(D.camera, false); home = D.camera; }

    // Drag to orbit, right-drag or shift-drag to pan, wheel or pinch to
    // zoom; it keeps turning a little after a flick.
    let drag = null, spin = [0, 0], last = 0;
    const pointers = new Map();
    canvas.addEventListener('pointerdown', (e) => {
      canvas.setPointerCapture(e.pointerId);
      pointers.set(e.pointerId, [e.clientX, e.clientY]);
      drag = { x: e.clientX, y: e.clientY, pan: e.button === 2 || e.shiftKey || pointers.size === 2 };
      canvas.classList.add('dragging');
      spin = [0, 0];
    });
    canvas.addEventListener('pointermove', (e) => {
      if (!drag || !pointers.has(e.pointerId)) return;
      const prev = pointers.get(e.pointerId);
      pointers.set(e.pointerId, [e.clientX, e.clientY]);
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        const others = [...pointers.entries()].filter(([id]) => id !== e.pointerId)[0][1];
        const before = Math.hypot(prev[0] - others[0], prev[1] - others[1]);
        const after = Math.hypot(a[0] - b[0], a[1] - b[1]);
        if (before > 0 && after > 0) cam.distance *= before / after;
      }
      const dx = e.clientX - prev[0], dy = e.clientY - prev[1];
      if (drag.pan) pan(dx, dy);
      else { cam.yaw -= dx * 0.006; cam.pitch = clamp(cam.pitch + dy * 0.006, -1.45, 1.45); spin = [dx * 0.006, dy * 0.006]; last = performance.now(); }
      draw();
    });
    const end = (e) => {
      pointers.delete(e.pointerId);
      if (pointers.size) return;
      drag = null;
      canvas.classList.remove('dragging');
      if (performance.now() - last < 60) coast();
    };
    canvas.addEventListener('pointerup', end);
    canvas.addEventListener('pointercancel', end);
    canvas.addEventListener('contextmenu', (e) => e.preventDefault());
    canvas.addEventListener('wheel', (e) => { e.preventDefault(); cam.distance *= Math.exp(e.deltaY * 0.0012); draw(); }, { passive: false });
    canvas.addEventListener('dblclick', () => reset());
    function coast() {
      const step = () => {
        if (drag || Math.abs(spin[0]) + Math.abs(spin[1]) < 0.0004) return;
        cam.yaw -= spin[0]; cam.pitch = clamp(cam.pitch + spin[1], -1.45, 1.45);
        spin = [spin[0] * 0.92, spin[1] * 0.92];
        draw();
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }
    function pan(dx, dy) {
      const { right, up } = basis();
      const scale = cam.distance * 2 * Math.tan(fov() / 2) / canvas.clientHeight;
      cam.target = cam.target.map((v, k) => v - right[k] * dx * scale + up[k] * dy * scale);
    }
    function reset() { if (home) setCamera(home, true); }

    // ---- configuring ----------------------------------------------------
    function configure(choices) {
      // Studios first, as Tequila applies them, then the other options.
      const ordered = options.map((o, i) => [o, i]).sort((a, b) => (a[0].drives === 'studio' ? 0 : 1) - (b[0].drives === 'studio' ? 0 : 1));
      for (const [option, i] of ordered) {
        const effect = (option.choices[choices[i]] || {}).effect;
        if (!effect) continue;
        if (effect.camera) setCamera(effect.camera, true);
        if (effect.environment) { envRot = effect.environment.rotation; envGain = effect.environment.intensity; }
        if (effect.exposure !== undefined) exposure = effect.exposure;
        if (effect.show) { const shown = new Set(effect.show); nodes.forEach((n, k) => { n.visible = shown.has(k); }); }
        if (effect.assign) for (const [node, material] of effect.assign) { if (nodes[node]) nodes[node].materials = nodes[node].materials.map(() => material); }
      }
      draw();
    }

    // ---- drawing --------------------------------------------------------
    // The render's framing: its vertical angle in a window as wide as the
    // render or wider, its horizontal one in a narrower window, so the
    // product is never cut off on a phone.
    function fov() {
      const aspect = canvas.clientWidth / Math.max(1, canvas.clientHeight);
      const render = D.aspect || 16 / 9;
      const horizontal = 2 * Math.atan(cam.sensor / 2 / cam.focal);
      return 2 * Math.atan(Math.tan(horizontal / 2) / Math.min(aspect, render));
    }
    function eye() {
      const c = Math.cos(cam.pitch);
      return [cam.target[0] + cam.distance * c * Math.sin(cam.yaw), cam.target[1] + cam.distance * Math.sin(cam.pitch), cam.target[2] + cam.distance * c * Math.cos(cam.yaw)];
    }
    function basis() {
      const f = normalize(sub(cam.target, eye()));
      const right = normalize(cross(f, [0, 1, 0]));
      return { forward: f, right, up: cross(right, f) };
    }
    let queued = false;
    function draw() { if (!queued) { queued = true; requestAnimationFrame(frame); } }
    function frame() {
      queued = false;
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.round(canvas.clientWidth * ratio), h = Math.round(canvas.clientHeight * ratio);
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      gl.viewport(0, 0, w, h);
      const e = eye();
      const far = cam.distance * 50 + 10, near = Math.max(cam.distance * 0.002, 0.0005);
      const proj = perspective(fov(), w / Math.max(1, h), near, far);
      const viewM = lookAt(e, cam.target, [0, 1, 0]);
      const viewProj = multiply(proj, viewM);
      const bg = E.background || { mode: 'color', color: [0.8, 0.8, 0.8] };
      gl.disable(gl.BLEND);
      gl.depthMask(true);
      if (bg.mode === 'color') {
        const s = bg.color.map((c) => Math.pow(Math.min(1, Math.max(0, c)), 1 / 2.2));
        gl.clearColor(s[0], s[1], s[2], 1);
      } else gl.clearColor(0.85, 0.86, 0.87, 1);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.enable(gl.DEPTH_TEST);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, envTex);
      const lights = (prog) => {
        gl.uniform1i(prog.u.uEnv, 0);
        gl.uniform1f(prog.u.uLevels, envLevels);
        gl.uniform1f(prog.u.uRgbm, E.rgbm || 16);
        gl.uniform1f(prog.u.uEnvRot, envRot);
        gl.uniform1f(prog.u.uEnvGain, envGain);
        gl.uniform1f(prog.u.uExposure, exposure);
        gl.uniform1i(prog.u.uDisplay, display);
        if (prog.u.uSH) gl.uniform3fv(prog.u.uSH, new Float32Array(E.sh));
      };
      if (bg.mode === 'environment') {
        gl.useProgram(sky.p);
        lights(sky);
        gl.uniformMatrix4fv(sky.u.uInverse, false, invert(viewProj));
        gl.activeTexture(gl.TEXTURE5); gl.bindTexture(gl.TEXTURE_2D, backdropTex); gl.uniform1i(sky.u.uBackdrop, 5);
        gl.activeTexture(gl.TEXTURE0);
        gl.depthMask(false);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.depthMask(true);
      }

      // Opaque first, then see-through back to front.
      const draws = [];
      nodes.forEach((n, k) => {
        if (!n.visible || n.mesh === undefined) return;
        meshes[n.mesh].forEach((prim, j) => {
          const mi = n.materials[j] !== undefined ? n.materials[j] : prim.material;
          const m = (gltf.materials || [])[mi] || {};
          const centre = transform(n.matrix, [(prim.min[0] + prim.max[0]) / 2, (prim.min[1] + prim.max[1]) / 2, (prim.min[2] + prim.max[2]) / 2]);
          const ext = m.extensions || {};
          const clear = m.alphaMode === 'BLEND' || (ext.KHR_materials_transmission && ext.KHR_materials_transmission.transmissionFactor > 0);
          draws.push({ n, prim, m, clear, depth: dot(sub(centre, e), sub(centre, e)) });
        });
      });
      draws.sort((a, b) => (a.clear - b.clear) || (a.clear ? b.depth - a.depth : a.depth - b.depth));

      gl.useProgram(surface.p);
      lights(surface);
      gl.uniformMatrix4fv(surface.u.uViewProj, false, viewProj);
      gl.uniform3fv(surface.u.uEye, e);
      gl.uniform1i(surface.u.uBaseTex, 1); gl.uniform1i(surface.u.uMRTex, 2); gl.uniform1i(surface.u.uEmTex, 3);
      let blending = false;
      const shadowAt = draws.findIndex((d) => d.clear);
      const drawShadow = () => {
        if (!shadowTex || !D.shadow) return;
        gl.useProgram(shade.p);
        gl.uniformMatrix4fv(shade.u.uViewProj, false, viewProj);
        gl.uniform2fv(shade.u.uMin, D.shadow.min); gl.uniform2fv(shade.u.uSize, D.shadow.size); gl.uniform1f(shade.u.uY, D.shadow.y);
        gl.uniform1f(shade.u.uStrength, 1.0);
        gl.activeTexture(gl.TEXTURE4); gl.bindTexture(gl.TEXTURE_2D, shadowTex); gl.uniform1i(shade.u.uShadow, 4);
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA); gl.depthMask(false);
        gl.enable(gl.POLYGON_OFFSET_FILL); gl.polygonOffset(-1, -1);
        gl.bindVertexArray(quad); gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        gl.disable(gl.POLYGON_OFFSET_FILL);
        gl.depthMask(true); gl.disable(gl.BLEND);
        gl.useProgram(surface.p);
        gl.activeTexture(gl.TEXTURE0);
      };
      draws.forEach((d, index) => {
        if (index === shadowAt) drawShadow();
        if (d.clear && !blending) { gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA); gl.depthMask(false); blending = true; }
        const m = d.m, p = m.pbrMetallicRoughness || {}, ext = m.extensions || {};
        gl.uniformMatrix4fv(surface.u.uModel, false, d.n.matrix);
        gl.uniformMatrix3fv(surface.u.uNormal, false, normalMatrix(d.n.matrix));
        gl.uniform4fv(surface.u.uBase, p.baseColorFactor || [1, 1, 1, 1]);
        gl.uniform1f(surface.u.uMetal, p.metallicFactor !== undefined ? p.metallicFactor : 1);
        gl.uniform1f(surface.u.uRough, p.roughnessFactor !== undefined ? p.roughnessFactor : 1);
        const strength = (ext.KHR_materials_emissive_strength || {}).emissiveStrength || 1;
        gl.uniform3fv(surface.u.uEmissive, (m.emissiveFactor || [0, 0, 0]).map((v) => v * strength));
        const bind = (info, unit, has, uv) => {
          gl.uniform1i(surface.u[has], info ? 1 : 0);
          gl.uniform1i(surface.u[uv], info && info.texCoord ? 1 : 0);
          if (info) { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, textures[info.index]); }
        };
        bind(p.baseColorTexture, 1, 'uHasBase', 'uBaseUv');
        bind(p.metallicRoughnessTexture, 2, 'uHasMR', 'uMRUv');
        bind(m.emissiveTexture, 3, 'uHasEm', 'uEmUv');
        gl.activeTexture(gl.TEXTURE0);
        const t = ext.KHR_materials_transmission, vol = ext.KHR_materials_volume, coat = ext.KHR_materials_clearcoat, sheen = ext.KHR_materials_sheen, spec = ext.KHR_materials_specular;
        gl.uniform1f(surface.u.uTransmission, t ? t.transmissionFactor || 0 : 0);
        gl.uniform3fv(surface.u.uTint, vol && vol.attenuationColor ? vol.attenuationColor : [1, 1, 1]);
        gl.uniform1f(surface.u.uIor, (ext.KHR_materials_ior || {}).ior || 1.5);
        gl.uniform1f(surface.u.uCoat, coat ? coat.clearcoatFactor || 0 : 0);
        gl.uniform1f(surface.u.uCoatRough, coat ? coat.clearcoatRoughnessFactor || 0 : 0);
        gl.uniform3fv(surface.u.uSheen, sheen ? sheen.sheenColorFactor || [0, 0, 0] : [0, 0, 0]);
        gl.uniform1f(surface.u.uSheenRough, sheen ? sheen.sheenRoughnessFactor || 0 : 0);
        gl.uniform1f(surface.u.uSpecular, spec && spec.specularFactor !== undefined ? spec.specularFactor : 1);
        gl.uniform3fv(surface.u.uSpecColor, spec && spec.specularColorFactor ? spec.specularColorFactor : [1, 1, 1]);
        gl.bindVertexArray(d.prim.vao);
        gl.drawElements(gl.TRIANGLES, d.prim.count, gl.UNSIGNED_INT, 0);
      });
      if (shadowAt < 0) drawShadow();
      gl.bindVertexArray(null);
      gl.depthMask(true);
      gl.disable(gl.BLEND);
    }
    window.addEventListener('resize', draw);
    Promise.all(pending).then(draw);
    draw();
    return { canvas, configure, reset };
  }

  // An image from a data URL, its alpha left as stored: an <img> may
  // premultiply, which would lose RGBM's dark values.
  function bitmap(src) {
    const comma = src.indexOf(',');
    const type = src.slice(5, src.indexOf(';'));
    const raw = atob(src.slice(comma + 1));
    const data = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) data[i] = raw.charCodeAt(i);
    return createImageBitmap(new Blob([data], { type }), { colorSpaceConversion: 'none', premultiplyAlpha: 'none' });
  }

  // ---- a little linear algebra, column-major as WebGL wants ----------------
  function identity() { return new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]); }
  function sub(a, b) { return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]; }
  function dot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
  function cross(a, b) { return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]; }
  function normalize(a) { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; }
  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }
  function transform(m, p) { return [m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12], m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13], m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]]; }
  function multiply(a, b) {
    const o = new Float32Array(16);
    for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
      let s = 0; for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k]; o[c * 4 + r] = s;
    }
    return o;
  }
  function perspective(fovy, aspect, near, far) {
    const f = 1 / Math.tan(fovy / 2), o = new Float32Array(16);
    o[0] = f / aspect; o[5] = f; o[10] = (far + near) / (near - far); o[11] = -1; o[14] = 2 * far * near / (near - far);
    return o;
  }
  function lookAt(e, t, up) {
    const z = normalize(sub(e, t)), x = normalize(cross(up, z)), y = cross(z, x);
    return new Float32Array([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dot(x, e), -dot(y, e), -dot(z, e), 1]);
  }
  function invert(m) {
    const inv = new Float32Array(16);
    inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
    inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
    inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
    inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
    inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
    inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
    inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
    inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
    inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
    inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
    inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
    inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
    inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
    const det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
    for (let i = 0; i < 16; i++) inv[i] /= det || 1;
    return inv;
  }
  // The inverse-transpose of the upper 3x3, for normals.
  function normalMatrix(m) {
    const a = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
    const det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[3] * (a[1] * a[8] - a[2] * a[7]) + a[6] * (a[1] * a[5] - a[2] * a[4]);
    const d = det || 1;
    return new Float32Array([
      (a[4] * a[8] - a[5] * a[7]) / d, (a[5] * a[6] - a[3] * a[8]) / d, (a[3] * a[7] - a[4] * a[6]) / d,
      (a[2] * a[7] - a[1] * a[8]) / d, (a[0] * a[8] - a[2] * a[6]) / d, (a[1] * a[6] - a[0] * a[7]) / d,
      (a[1] * a[5] - a[2] * a[4]) / d, (a[2] * a[3] - a[0] * a[5]) / d, (a[0] * a[4] - a[1] * a[3]) / d,
    ]);
  }
})();
