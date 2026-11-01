// Tequila's website: the menu on narrow screens, the download that fits the
// visitor's computer, the newest release from GitHub, the picture viewer and
// the release notes. Every page works without it; it fills in the details.

(function () {
  'use strict';

  var REPO = 'ifpintac/tequila';
  var RELEASES = 'https://github.com/' + REPO + '/releases';
  var API = 'https://api.github.com/repos/' + REPO + '/releases';

  // ---------- Menu ----------

  var menu = document.querySelector('.menu-button');
  var nav = document.querySelector('.nav');
  if (menu && nav) {
    menu.addEventListener('click', function () {
      var open = nav.classList.toggle('open');
      menu.setAttribute('aria-expanded', open ? 'true' : 'false');
      menu.querySelector('.icon').textContent = open ? '' : '';
    });
  }

  // ---------- Reveal on scroll ----------

  var reveals = document.querySelectorAll('.reveal');
  if ('IntersectionObserver' in window) {
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('seen');
          seen.unobserve(entry.target);
        }
      });
    }, { rootMargin: '0px 0px -8% 0px' });
    reveals.forEach(function (el) { seen.observe(el); });
  } else {
    reveals.forEach(function (el) { el.classList.add('seen'); });
  }

  // ---------- Which computer is this? ----------

  // A best guess from what the browser says: an Intel Mac can't be told
  // from an Apple silicon one by the user agent alone, so a Mac is offered
  // Apple silicon, with the Intel build beside it.
  function platform() {
    var ua = navigator.userAgent || '';
    var hint = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    if (/Android|iPhone|iPad|iPod/i.test(ua)) return 'mobile';
    if (/Win/i.test(hint) || /Windows/i.test(ua)) return 'windows';
    if (/Mac/i.test(hint) || /Macintosh/i.test(ua)) return 'mac';
    if (/Linux/i.test(hint) || /Linux|X11/i.test(ua)) return 'linux';
    return 'other';
  }

  var NAMES = { windows: 'Windows', mac: 'Mac', intel: 'Intel Mac', linux: 'Linux' };

  // Each platform's file in a release, by the names the release workflows
  // give them.
  function assetFor(assets, which) {
    return assets.find(function (a) {
      var n = a.name;
      if (which === 'windows') return /^Tequila-Setup-.*\.exe$/.test(n);
      if (which === 'mac') return /^Tequila-[\d.]+\.dmg$/.test(n);
      if (which === 'intel') return /-intel\.dmg$/.test(n);
      if (which === 'linux') return /linux-x86_64\.tar\.gz$/.test(n);
      return false;
    });
  }

  function megabytes(bytes) {
    return Math.round(bytes / 1048576) + ' MB';
  }

  function versionOf(release) {
    return (release.tag_name || '').replace(/^v/, '');
  }

  function niceDate(iso) {
    try {
      return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' });
    } catch (e) {
      return iso.slice(0, 10);
    }
  }

  var mine = platform();

  // Download buttons say which computer they are for before GitHub answers.
  document.querySelectorAll('[data-download]').forEach(function (button) {
    var label = button.querySelector('[data-label]');
    if (label && NAMES[mine]) label.textContent = 'Download for ' + NAMES[mine];
  });

  document.querySelectorAll('.platform[data-platform]').forEach(function (card) {
    if (card.getAttribute('data-platform') === mine) {
      card.classList.add('mine');
      var title = card.querySelector('h3');
      if (title) title.insertAdjacentHTML('beforeend', '<span class="badge">This computer</span>');
    }
  });

  var latest = null;
  function withLatest(then) {
    if (latest) return then(latest);
    fetch(API + '/latest', { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (release) { latest = release; then(release); })
      .catch(function () { /* the links already go to the Releases page */ });
  }

  if (document.querySelector('[data-download], [data-version], .platform[data-platform]')) {
    withLatest(function (release) {
      var assets = release.assets || [];
      document.querySelectorAll('[data-version]').forEach(function (el) {
        el.textContent = versionOf(release);
      });
      document.querySelectorAll('[data-released]').forEach(function (el) {
        el.textContent = niceDate(release.published_at);
      });
      document.querySelectorAll('[data-download]').forEach(function (button) {
        var asset = NAMES[mine] && assetFor(assets, mine);
        if (asset) button.href = asset.browser_download_url;
      });
      document.querySelectorAll('.platform[data-platform]').forEach(function (card) {
        var asset = assetFor(assets, card.getAttribute('data-platform'));
        if (!asset) return;
        var link = card.querySelector('a.button');
        if (link) {
          link.href = asset.browser_download_url;
          var size = link.querySelector('[data-size]');
          if (size) size.textContent = megabytes(asset.size);
        }
        var file = card.querySelector('.file');
        if (file) file.textContent = asset.name;
      });
    });
  }

  // ---------- Picture viewer ----------

  var zoomable = document.querySelectorAll('[data-zoom]');
  if (zoomable.length) {
    var box = document.createElement('div');
    box.className = 'lightbox';
    box.setAttribute('role', 'dialog');
    box.setAttribute('aria-modal', 'true');
    box.setAttribute('aria-label', 'Picture');
    box.innerHTML = '<img alt=""><button type="button" aria-label="Close"><i class="icon" aria-hidden="true"></i></button>';
    document.body.appendChild(box);
    var big = box.querySelector('img');
    var close = function () { box.classList.remove('open'); big.removeAttribute('src'); };
    box.addEventListener('click', close);
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') close(); });
    zoomable.forEach(function (img) {
      img.addEventListener('click', function () {
        big.src = img.getAttribute('data-zoom') || img.currentSrc || img.src;
        big.alt = img.alt;
        box.classList.add('open');
      });
    });
  }

  // ---------- Release notes ----------

  function escape(text) {
    return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  // The few kinds of Markdown the release notes use: headings, nested
  // lists, paragraphs, rules, bold, inline code and links.
  function inline(text) {
    var out = escape(text);
    out = out.replace(/`([^`]+)`/g, '<code>$1</code>');
    out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, function (_, label, url) {
      var safe = /^(https?:|#|\.\.?\/)/.test(url) ? url : '#';
      return '<a href="' + safe + '">' + label + '</a>';
    });
    return out;
  }

  function markdown(source) {
    var lines = source.replace(/\r\n?/g, '\n').split('\n');
    var html = [];
    var paragraph = [];
    var depth = [];
    function flush() {
      if (paragraph.length) { html.push('<p>' + inline(paragraph.join(' ')) + '</p>'); paragraph = []; }
    }
    function closeLists(to) {
      while (depth.length > to) { html.push('</li></ul>'); depth.pop(); }
    }
    lines.forEach(function (line) {
      var item = /^(\s*)[-*] (.*)$/.exec(line);
      var heading = /^(#{1,4}) (.*)$/.exec(line);
      if (item) {
        flush();
        var level = Math.floor(item[1].length / 2) + 1;
        if (level > depth.length) {
          while (depth.length < level) { html.push('<ul><li>'); depth.push(level); }
        } else {
          closeLists(level);
          html.push('</li><li>');
        }
        html.push(inline(item[2]));
      } else if (/^\s+\S/.test(line) && depth.length) {
        html.push(' ' + inline(line.trim()));
      } else if (heading) {
        flush(); closeLists(0);
        var n = Math.min(heading[1].length + 1, 4);
        html.push('<h' + n + '>' + inline(heading[2]) + '</h' + n + '>');
      } else if (/^---+\s*$/.test(line)) {
        flush(); closeLists(0);
        html.push('<hr>');
      } else if (!line.trim()) {
        flush(); closeLists(0);
      } else {
        closeLists(0);
        paragraph.push(line.trim());
      }
    });
    flush(); closeLists(0);
    return html.join('');
  }

  var list = document.getElementById('releases');
  if (list) {
    fetch(API + '?per_page=12', { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (releases) {
        releases = releases.filter(function (r) { return !r.draft; });
        list.innerHTML = '';
        releases.forEach(function (release, i) {
          var article = document.createElement('article');
          article.className = 'release' + (i ? ' older' : '');
          article.innerHTML =
            '<header><h2>' + escape(versionOf(release)) + (i ? '' : ' <span class="badge">Newest</span>') + '</h2>' +
            '<time datetime="' + escape(release.published_at || '') + '">' + escape(niceDate(release.published_at)) + '</time>' +
            '<a href="' + escape(release.html_url) + '">Downloads on GitHub <i class="icon" aria-hidden="true"></i></a></header>' +
            '<div class="notes">' + markdown(release.body || 'No notes for this version.') + '</div>' +
            (i ? '<button type="button" class="button button-quiet button-small expand">Show all</button>' : '');
          list.appendChild(article);
          var more = article.querySelector('.expand');
          if (more) more.addEventListener('click', function () {
            var open = article.classList.toggle('open');
            more.textContent = open ? 'Show less' : 'Show all';
          });
        });
      })
      .catch(function () {
        list.innerHTML = '<p class="loading">The notes could not be loaded from GitHub just now. Every version and its notes are on the <a href="' + RELEASES + '">Releases page</a>.</p>';
      });
  }
})();
