(function () {
  'use strict';
  var COLS = 24, ROWS = 16, CELL = 40, W = 960, H = 640, TW = 192, TH = 128, TICK = 160;
  var HOST_RGB = ['255,122,89', '91,140,255', '168,219,76'];
  var token = (document.querySelector('meta[name="csrf-token"]') || {}).content || '';
  var canvas = document.getElementById('dish'), ctx = canvas.getContext('2d');
  var showOwners = document.getElementById('show-owners'), showAgents = document.getElementById('show-agents');
  var frame = null, state = null, busy = false;
  // The trail is drawn into a 192 × 128 buffer and scaled up smooth.
  var buf = document.createElement('canvas'); buf.width = TW; buf.height = TH;
  var bctx = buf.getContext('2d'), img = bctx.createImageData(TW, TH);
  // A warm ramp: nothing, deep amber, gold, pale yellow.
  var ramp = [];
  (function () {
    var stops = [[0, 0, 0, 0, 0], [18, 64, 36, 6, 90], [60, 176, 104, 18, 200], [130, 232, 160, 44, 240], [210, 250, 214, 110, 255], [255, 255, 240, 180, 255]];
    for (var v = 0; v < 256; v++) {
      var i = 0; while (i < stops.length - 2 && v > stops[i + 1][0]) i++;
      var a = stops[i], b = stops[i + 1], t = (v - a[0]) / Math.max(1, b[0] - a[0]);
      ramp.push([a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t, a[4] + (b[4] - a[4]) * t]);
    }
  })();

  function el(tag, attrs, kids) {
    var e = document.createElement(tag);
    Object.keys(attrs || {}).forEach(function (k) {
      if (k === 'text') e.textContent = attrs[k];
      else if (k === 'style') e.setAttribute('style', attrs[k]);
      else e.setAttribute(k, attrs[k]);
    });
    (kids || []).forEach(function (c) { if (c) e.appendChild(c); });
    return e;
  }
  function secs(ms) { return (ms / 1000).toFixed(1) + 's'; }
  function n(x) { return (x || 0).toLocaleString('en-US'); }

  function size() {
    var dpr = window.devicePixelRatio || 1, r = canvas.getBoundingClientRect();
    canvas.width = Math.round(r.width * dpr);
    canvas.height = Math.round(r.width * dpr * H / W);
  }

  function paintTrail() {
    var hex = frame.trail, d = img.data;
    for (var i = 0, j = 0; i < TW * TH; i++, j += 2) {
      var v = parseInt(hex.substr(j, 2), 16) || 0, c = ramp[v], k = i * 4;
      d[k] = c[0]; d[k + 1] = c[1]; d[k + 2] = c[2]; d[k + 3] = c[3];
    }
    bctx.putImageData(img, 0, 0);
  }

  function draw() {
    var dpr = canvas.width / W;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    var g = ctx.createRadialGradient(W / 2, H / 2, 60, W / 2, H / 2, W * 0.7);
    g.addColorStop(0, '#171a12'); g.addColorStop(1, '#0a0b08');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    if (!frame || !frame.trail) return;
    // The veins, then a soft glow of the same.
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(buf, 0, 0, W, H);
    ctx.save();
    ctx.globalCompositeOperation = 'lighter'; ctx.globalAlpha = 0.28;
    if ('filter' in ctx) ctx.filter = 'blur(4px)';
    ctx.drawImage(buf, 0, 0, W, H);
    ctx.restore();
    // Owners, holes.
    for (var i = 0; i < frame.cells.length; i++) {
      var o = frame.cells[i], cx = (i % COLS) * CELL, cy = Math.floor(i / COLS) * CELL;
      if (o < 0) {
        ctx.fillStyle = '#04050a'; ctx.fillRect(cx, cy, CELL, CELL);
        ctx.strokeStyle = 'rgba(120,130,160,0.12)'; ctx.lineWidth = 1;
        for (var s = -CELL; s < CELL; s += 8) { ctx.beginPath(); ctx.moveTo(cx + s, cy + CELL); ctx.lineTo(cx + s + CELL, cy); ctx.stroke(); }
      } else if (showOwners.checked) {
        ctx.fillStyle = 'rgba(' + HOST_RGB[o] + ',0.14)'; ctx.fillRect(cx, cy, CELL, CELL);
        ctx.strokeStyle = 'rgba(' + HOST_RGB[o] + ',0.45)'; ctx.lineWidth = 0.6; ctx.strokeRect(cx + 0.5, cy + 0.5, CELL - 1, CELL - 1);
      }
    }
    // Agents: frozen ones always (they are the mold caught in the dark), moving ones on request.
    var a = frame.agents;
    for (var k = 0; k < a.length; k += 3) {
      if (a[k + 2] === 1) { ctx.fillStyle = 'rgba(170,180,200,0.75)'; ctx.fillRect(a[k] - 0.8, a[k + 1] - 0.8, 1.6, 1.6); }
      else if (showAgents.checked) { ctx.fillStyle = 'rgba(255,240,190,0.55)'; ctx.fillRect(a[k] - 0.6, a[k + 1] - 0.6, 1.2, 1.2); }
    }
    // Food: oat flakes.
    frame.foods.forEach(function (f, idx) {
      ctx.save(); ctx.translate(f[0], f[1]); ctx.rotate(idx * 1.3);
      ctx.fillStyle = 'rgba(0,0,0,0.35)'; ctx.beginPath(); ctx.ellipse(1.5, 2, 8, 5.5, 0, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = '#ece0c2'; ctx.beginPath(); ctx.ellipse(0, 0, 8, 5.5, 0, 0, Math.PI * 2); ctx.fill();
      ctx.strokeStyle = 'rgba(160,130,80,0.6)'; ctx.lineWidth = 0.8; ctx.beginPath(); ctx.moveTo(-5, 0); ctx.lineTo(5, 0); ctx.stroke();
      ctx.restore();
    });
    var at = state ? state.now_ms : 0;
    frame.marks.forEach(function (m) {
      var age = Math.max(0, Math.min(1, (at - m.at_ms) / 4000)), r = 3 + age * 10;
      ctx.strokeStyle = 'rgba(255,90,90,' + (1 - age).toFixed(2) + ')'; ctx.lineWidth = 1.2;
      ctx.beginPath(); ctx.arc(m.x, m.y, r, 0, Math.PI * 2); ctx.stroke();
    });
  }

  function loop() { draw(); requestAnimationFrame(loop); }

  function readout() {
    var r = document.getElementById('readout');
    if (!frame || !frame.tick) { r.textContent = 'waiting for the hosts to claim their cells…'; return; }
    r.textContent = 'tick ' + n(frame.tick) + ' · ' + n(frame.moving) + ' agents moving · ' + n(frame.frozen) +
      ' frozen · ' + frame.holes + ' holes · ' + frame.foods.length + ' food · ' + frame.tick_ms + ' ms/tick';
  }

  function hosts() {
    var root = document.getElementById('hosts');
    root.textContent = '';
    var counts = [0, 0, 0];
    if (frame) frame.cells.forEach(function (o) { if (o >= 0) counts[o]++; });
    state.hosts.forEach(function (h, idx) {
      var held = (h.view.held || []).map(function (r) {
        return el('div', { class: 'held' }, [document.createTextNode('holds '), el('b', { text: r.range }),
          document.createTextNode(' at epoch ' + r.epoch + (r.serving ? '' : ' — not serving'))]);
      });
      if (!held.length) held = [el('div', { class: 'held', text: h.view.reachable ? 'holds nothing' : 'not answering' })];
      (h.view.waiting || []).forEach(function (w) { held.push(el('div', { class: 'held', text: 'waiting to claim ' + w })); });
      var b = function (label, action, cls, enabled) {
        var x = el('button', { class: cls || '', 'data-post': '/hosts/' + h.name + '/' + action, text: label });
        if (!enabled) x.setAttribute('disabled', '');
        return x;
      };
      var p = h.process;
      root.appendChild(el('div', { class: 'host', style: '--lane:var(--' + h.name + ')' }, [
        el('div', {}, [el('span', { class: 'name', text: h.name + ' ' }), el('span', { class: 'pill ' + p, text: p })]),
        el('div', { class: 'role', text: h.role + ' · claims ' + h.slots + ' · :' + h.port + (h.pid ? ' · pid ' + h.pid : '') }),
        el('div', { class: 'cells' }, [el('b', { text: String(counts[idx]) }), document.createTextNode(' of 384 cells served last tick')]),
        h.view.lag_s == null ? null : el('div', { class: 'cells' + (h.view.lag_s > 6 ? ' behind' : '') }, [
          document.createTextNode('replica '), el('b', { text: h.view.lag_s.toFixed(0) + ' s' }),
          document.createTextNode(' behind · ' + (h.view.replicating || 0) + ' files shipping')])
      ].concat(held).concat([el('div', { class: 'buttons' }, [
        b('Start', 'start', 'go', p === 'stopped'),
        b('kill -9', 'kill', 'danger', p !== 'stopped'),
        b('Stop cleanly', 'term', '', p === 'running'),
        b('Freeze', 'freeze', '', p === 'running'),
        b('Thaw', 'thaw', '', p === 'frozen')
      ])])));
    });
    var grow = document.getElementById('grow');
    grow.textContent = state.growing ? 'Pause' : 'Grow';
    grow.setAttribute('data-post', state.growing ? '/mold/stop' : '/mold/start');
  }

  function stat(label, value, cls) {
    return el('div', { class: 'stat ' + (cls || '') }, [el('b', { text: value }), el('span', { text: label })]);
  }

  function lastFailureBefore(ms) {
    var f = null;
    (state.failures || []).forEach(function (x) { if (x.at_ms <= ms) f = x; });
    return f;
  }

  function headcount() {
    var root = document.getElementById('headcount');
    root.textContent = '';
    root.appendChild(el('h3', { text: 'Headcount' }));
    if (!frame || !frame.tick) { root.appendChild(el('div', { class: 'hint', text: 'The dish is inoculated once both primaries serve every cell.' })); return; }
    var c = frame.counts;
    root.appendChild(el('div', { class: 'verdict ' + (c.lost ? 'bad' : 'good'), text: c.lost ? n(c.lost) + ' agents lost' : 'No agent lost' }));
    root.appendChild(el('div', { class: 'stats' }, [
      stat('agents', n(frame.population)), stat('moving', n(frame.moving), 'ok'), stat('frozen in holes', n(frame.frozen), frame.frozen ? 'warn' : ''),
      stat('agents came back older', n(c.rewound), c.rewound ? 'warn' : ''), stat('cells’ trail came back older', n(c.trails_rewound), c.trails_rewound ? 'warn' : ''), stat('ghost copies cleared', n(c.ghosts)),
      stat('lost', n(c.lost), c.lost ? 'alert' : ''), stat('stale epochs refused', n(c.stale), c.stale ? 'alert' : ''), stat('cells served twice', n(c.double), c.double ? 'alert' : '')
    ]));
    var groups = {};
    (frame.rewinds || []).forEach(function (r) {
      var f = lastFailureBefore(r.seen_ms), key = f ? f.host + ' ' + f.what + ' at ' + secs(f.at_ms) : 'no failure before it';
      var g = groups[key] || (groups[key] = { agent: 0, trail: 0, max: 0 });
      g[r.kind]++; g.max = Math.max(g.max, r.by_ms);
    });
    var lostBy = {};
    (frame.lost || []).forEach(function (l) {
      var f = lastFailureBefore(l.found_ms), key = f ? f.host + ' ' + f.what + ' at ' + secs(f.at_ms) : 'no failure before it';
      lostBy[key] = (lostBy[key] || 0) + 1;
    });
    var keys = Object.keys(groups).concat(Object.keys(lostBy).filter(function (k) { return !groups[k]; }));
    if (keys.length) {
      root.appendChild(el('div', { class: 'losses' }, keys.map(function (k) {
        var g = groups[k] || { agent: 0, trail: 0, max: 0 }, l = lostBy[k] || 0;
        return el('div', { text: 'After ' + k + ': ' + g.trail + ' cells’ trail and ' + g.agent + ' agents came back up to ' + secs(g.max) +
          ' older; ' + l + ' agents lost (they moved into its cells after its last write reached the replica).' });
      })));
    } else {
      root.appendChild(el('div', { class: 'hint', text: 'Click the dish to drop food. An agent is lost only if it is in no cell and the dish has no holes left.' }));
    }
  }

  function timeline() {
    var root = document.getElementById('timeline');
    root.textContent = '';
    state.timeline.slice(0, 120).forEach(function (e) {
      root.appendChild(el('li', {}, [el('span', { class: 't', text: secs(e.at_ms) }), el('span', { class: 'k ' + e.kind, text: e.kind }), el('span', { text: e.text })]));
    });
  }

  function renderPanels() { if (!state) return; hosts(); headcount(); timeline(); }

  function loadFrame() {
    fetch('/_frame.json', { credentials: 'same-origin' }).then(function (r) { return r.json(); }).then(function (f) {
      if (frame && f.tick === frame.tick) return;
      frame = f;
      if (f.trail) paintTrail();
      readout();
    }).catch(function () {});
  }
  function loadState() {
    if (busy) return;
    fetch('/_state.json', { credentials: 'same-origin' }).then(function (r) { return r.json(); }).then(function (s) { state = s; renderPanels(); }).catch(function () {});
  }

  function post(url, b) {
    busy = true;
    return fetch(url, { method: 'POST', headers: { 'x-csrf-token': token }, credentials: 'same-origin' })
      .then(function (r) { return r.json().then(function (body) { if (!r.ok) throw new Error(body.detail || body.title || ('HTTP ' + r.status)); return body; }); })
      .then(function (s) { state = s; renderPanels(); })
      .catch(function (err) { if (!b) return; var t = el('div', { class: 'err', text: err.message }); b.parentNode.appendChild(t); setTimeout(function () { t.remove(); }, 5000); })
      .finally(function () { busy = false; if (b) b.removeAttribute('disabled'); loadState(); });
  }

  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-post]');
    if (!b || b.hasAttribute('disabled')) return;
    var c = b.getAttribute('data-confirm');
    if (c && !window.confirm(c)) return;
    b.setAttribute('disabled', '');
    post(b.getAttribute('data-post'), b);
  });
  canvas.addEventListener('click', function (e) {
    var r = canvas.getBoundingClientRect();
    var x = (e.clientX - r.left) / r.width * W, y = (e.clientY - r.top) / r.height * H;
    post('/food/' + x.toFixed(1) + '/' + y.toFixed(1), null);
  });

  window.addEventListener('resize', size);
  size();
  setInterval(loadFrame, TICK);
  setInterval(loadState, 500);
  loadFrame(); loadState();
  requestAnimationFrame(loop);
})();
