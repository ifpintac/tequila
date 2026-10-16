// Tequila spin viewer: a grid of renders around a product, turned under a
// finger or a mouse. No library.
//
// Two ways in. The exported folder's own page sets window.TEQUILA_SPIN
// (from manifest.js, since a page opened from the disk may not fetch) and
// has a #spin element. A site embeds it as an element instead:
//   <script src="spin/spin.js"></script>
//   <tequila-spin src="spin/manifest.json"></tequila-spin>
// Frame paths are relative to the manifest.
(function () {
  'use strict';

  function mount(host, manifest, base) {
    // Positioned, so the canvas and buttons can sit inside it - unless the
    // page has placed it already.
    if (getComputedStyle(host).position === 'static') host.style.position = 'relative';
    host.style.overflow = 'hidden';
    host.style.touchAction = 'none';
    host.style.userSelect = 'none';
    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;display:block;cursor:grab';
    host.appendChild(canvas);
    const ctx = canvas.getContext('2d');
    const columns = manifest.columns, rows = manifest.rows;
    const url = (path) => new URL(path, base).href;
    let column = 0, row = manifest.start_row || 0, angle = 0;
    let zoom = 1, centre = [0.5, 0.5];

    // Frames: every one eventually, the ones spread round first, so the
    // turn is whole early and fills in.
    const frames = manifest.frames.map(() => null);
    const order = [];
    for (const step of [8, 4, 2, 1]) {
      for (let i = 0; i < frames.length; i += step) if (!order.includes(i)) order.push(i);
    }
    let next = 0;
    const loadMore = () => {
      for (let k = 0; k < 4 && next < order.length; k++) {
        const index = order[next++];
        const image = new Image();
        image.onload = () => { frames[index] = image; draw(); loadMore(); };
        image.onerror = () => loadMore();
        image.src = url(manifest.frames[index]);
      }
    };
    let sprites = null;
    if (manifest.sprites) {
      const sheet = new Image();
      sheet.onload = () => { sprites = sheet; draw(); };
      sheet.src = url(manifest.sprites.src);
    }
    loadMore();

    // The frame to show, and the nearest loaded one while it is not.
    const at = (c, r) => r * columns + ((c % columns) + columns) % columns;
    function nearest(c, r) {
      for (let d = 0; d < columns; d++) {
        for (const s of [c + d, c - d]) { const f = frames[at(s, r)]; if (f) return f; }
      }
      return null;
    }

    let queued = false;
    function draw() { if (!queued) { queued = true; requestAnimationFrame(paint); } }
    function paint() {
      queued = false;
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.round(host.clientWidth * ratio), h = Math.round(host.clientHeight * ratio);
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      ctx.clearRect(0, 0, w, h);
      const fit = Math.min(w / manifest.width, h / manifest.height) * zoom;
      const dw = manifest.width * fit, dh = manifest.height * fit;
      const x = w / 2 - centre[0] * dw, y = h / 2 - centre[1] * dh;
      const exact = frames[at(column, row)];
      ctx.imageSmoothingQuality = 'high';
      if (exact) ctx.drawImage(exact, x, y, dw, dh);
      else if (sprites) {
        const s = manifest.sprites, index = at(column, row);
        ctx.drawImage(sprites, (index % columns) * s.width, Math.floor(index / columns) * s.height, s.width, s.height, x, y, dw, dh);
      } else {
        const near = nearest(column, row);
        if (near) ctx.drawImage(near, x, y, dw, dh);
      }
    }

    // Drag to turn (a full width is a full turn), up and down to tilt when
    // there are rows; it coasts after a flick and turns by itself until
    // touched.
    let drag = null, velocity = 0, spinning = true, lastMove = 0;
    const pointers = new Map();
    const setAngle = (a) => { angle = a; column = Math.round(angle); draw(); };
    canvas.addEventListener('pointerdown', (e) => {
      canvas.setPointerCapture(e.pointerId);
      pointers.set(e.pointerId, [e.clientX, e.clientY]);
      drag = { x: e.clientX, y: e.clientY, rowAt: e.clientY };
      spinning = false; velocity = 0;
      canvas.style.cursor = 'grabbing';
    });
    canvas.addEventListener('pointermove', (e) => {
      if (!drag || !pointers.has(e.pointerId)) return;
      const prev = pointers.get(e.pointerId);
      pointers.set(e.pointerId, [e.clientX, e.clientY]);
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        const other = [...pointers.entries()].find(([id]) => id !== e.pointerId)[1];
        const before = Math.hypot(prev[0] - other[0], prev[1] - other[1]), after = Math.hypot(a[0] - b[0], a[1] - b[1]);
        if (before > 0) { zoom = Math.min(4, Math.max(1, zoom * after / before)); draw(); }
        return;
      }
      const dx = e.clientX - prev[0];
      if (zoom > 1.01 && e.shiftKey) {
        centre[0] -= dx / (host.clientWidth * zoom); centre[1] -= (e.clientY - prev[1]) / (host.clientHeight * zoom);
        draw();
        return;
      }
      const step = -dx / Math.max(1, host.clientWidth) * columns;
      velocity = step; lastMove = performance.now();
      setAngle(angle + step);
      if (rows > 1) {
        const rowStep = Math.max(30, host.clientHeight / (rows * 2));
        const moved = Math.trunc((e.clientY - drag.rowAt) / rowStep);
        if (moved) { row = Math.min(rows - 1, Math.max(0, row - moved)); drag.rowAt += moved * rowStep; draw(); }
      }
    });
    const end = (e) => {
      pointers.delete(e.pointerId);
      if (pointers.size) return;
      drag = null;
      canvas.style.cursor = 'grab';
      if (performance.now() - lastMove < 60) coast();
    };
    canvas.addEventListener('pointerup', end);
    canvas.addEventListener('pointercancel', end);
    function coast() {
      const step = () => {
        if (drag || Math.abs(velocity) < 0.02) return;
        velocity *= 0.93;
        setAngle(angle + velocity);
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }
    canvas.addEventListener('wheel', (e) => {
      e.preventDefault();
      zoom = Math.min(4, Math.max(1, zoom * Math.exp(-e.deltaY * 0.0015)));
      if (zoom === 1) centre = [0.5, 0.5];
      draw();
    }, { passive: false });
    canvas.addEventListener('dblclick', reset);
    host.tabIndex = 0;
    host.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowLeft') setAngle(angle - 1);
      else if (e.key === 'ArrowRight') setAngle(angle + 1);
      else if (e.key === 'ArrowUp' && rows > 1) { row = Math.max(0, row - 1); draw(); }
      else if (e.key === 'ArrowDown' && rows > 1) { row = Math.min(rows - 1, row + 1); draw(); }
      else return;
      spinning = false;
      e.preventDefault();
    });
    function reset() { zoom = 1; centre = [0.5, 0.5]; row = manifest.start_row || 0; setAngle(0); }

    // The controls: turn by itself, and reset.
    const bar = document.createElement('div');
    bar.style.cssText = 'position:absolute;left:50%;bottom:14px;transform:translateX(-50%);display:flex;gap:6px';
    const button = (label, title, action) => {
      const b = document.createElement('button');
      b.textContent = label; b.title = title;
      b.style.cssText = 'font:13px system-ui,sans-serif;padding:6px 12px;border-radius:10px;border:1px solid rgba(0,0,0,.15);background:rgba(255,255,255,.9);cursor:pointer';
      b.addEventListener('click', action);
      bar.appendChild(b);
      return b;
    };
    const play = button('Pause', 'Turn by itself', () => { spinning = !spinning; play.textContent = spinning ? 'Pause' : 'Turn'; if (spinning) turn(); });
    button('Reset', 'Back to the first view', () => { reset(); });
    host.appendChild(bar);
    function turn() {
      let last = performance.now();
      const step = (now) => {
        if (!spinning) { play.textContent = 'Turn'; return; }
        setAngle(angle + (now - last) / 1000 * columns / 8);
        last = now;
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }
    turn();
    new ResizeObserver(draw).observe(host);
    draw();
  }

  // The element, for embedding.
  if (window.customElements && !customElements.get('tequila-spin')) {
    customElements.define('tequila-spin', class extends HTMLElement {
      connectedCallback() {
        if (this.mounted) return;
        this.mounted = true;
        if (!this.style.display) this.style.display = 'block';
        const src = new URL(this.getAttribute('src') || 'manifest.json', document.baseURI).href;
        fetch(src).then((r) => r.json()).then((m) => mount(this, m, src)).catch((e) => { this.textContent = 'The spin viewer could not load ' + src + ': ' + e; });
      }
    });
  }

  // The exported page.
  if (window.TEQUILA_SPIN) {
    const host = document.getElementById('spin');
    if (host) mount(host, window.TEQUILA_SPIN, document.baseURI);
  }
})();
