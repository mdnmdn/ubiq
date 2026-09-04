(function () {
  'use strict';

  // --- Theme Management ---
  const THEME_KEY = 'markdown-web-theme';

  function getPreferredTheme() {
    const saved = localStorage.getItem(THEME_KEY);
    if (saved === 'dark' || saved === 'light') {
      return saved;
    }
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  function setTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem(THEME_KEY, theme);
    updateThemeIcon(theme);
  }

  function updateThemeIcon(theme) {
    const btn = document.getElementById('theme-toggle-btn');
    if (!btn) return;
    if (theme === 'dark') {
      btn.innerHTML = `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="5"></circle><line x1="12" y1="1" x2="12" y2="3"></line><line x1="12" y1="21" x2="12" y2="23"></line><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"></line><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"></line><line x1="1" y1="12" x2="3" y2="12"></line><line x1="21" y1="12" x2="23" y2="12"></line><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"></line><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"></line></svg>`;
      btn.setAttribute('title', 'Switch to light theme');
    } else {
      btn.innerHTML = `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path></svg>`;
      btn.setAttribute('title', 'Switch to dark theme');
    }
  }

  function initTheme() {
    const currentTheme = getPreferredTheme();
    setTheme(currentTheme);

    const toggleBtn = document.getElementById('theme-toggle-btn');
    if (toggleBtn) {
      toggleBtn.addEventListener('click', () => {
        const active = document.documentElement.getAttribute('data-theme');
        setTheme(active === 'dark' ? 'light' : 'dark');
      });
    }

    if (window.matchMedia) {
      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
        if (!localStorage.getItem(THEME_KEY)) {
          setTheme(e.matches ? 'dark' : 'light');
        }
      });
    }
  }

  // --- Sidebar Directory Toggle & Filter ---

  // Sidebar DOM survives SPA nav (open/collapsed folders persist) — only the active link and its
  // open ancestors move. Shared by initSidebar()'s initial-load call and afterContentSwap().
  function syncActiveNavLink(pathname) {
    document.querySelectorAll('.nav-link.active').forEach(link => link.classList.remove('active'));
    const activeLink = document.querySelector(`.nav-link[href="${CSS.escape(pathname)}"]`);
    if (!activeLink) return;
    activeLink.classList.add('active');
    let cur = activeLink.parentElement;
    while (cur && !cur.classList.contains('sidebar-tree-container')) {
      if (cur.classList.contains('nav-dir')) {
        cur.classList.add('open');
      }
      cur = cur.parentElement;
    }
    activeLink.scrollIntoView({ block: 'center', behavior: 'instant' });
  }

  // Composes the name filter and the docs-only scope toggle: a row must pass both, so both
  // handlers route through this one function rather than independently stomping on
  // `style.display`. Module-scoped (not closed over initSidebar's elements) so afterContentSwap()
  // can re-apply it cheaply without re-running the rest of initSidebar's wiring.
  function applySidebarVisibility() {
    const sidebarEl = document.getElementById('app-sidebar');
    const filterInput = document.getElementById('sidebar-filter');
    const query = filterInput ? filterInput.value.toLowerCase().trim() : '';
    const docsOnly = sidebarEl ? sidebarEl.classList.contains('docs-only') : false;

    const allItems = document.querySelectorAll('.nav-tree .nav-item:not(.nav-dir)');
    allItems.forEach(item => {
      const title = item.dataset.title || '';
      const filename = item.dataset.filename || '';
      const matchesQuery = !query || title.includes(query) || filename.includes(query);
      const matchesScope = !docsOnly || item.dataset.doc === '1';
      item.style.display = (matchesQuery && matchesScope) ? '' : 'none';
    });

    // Show dirs with a visible descendant, or whose own name/title matches the name filter
    // (scope never hides a dir by name match alone — only by having no visible doc inside it).
    const allDirs = document.querySelectorAll('.nav-tree .nav-dir');
    allDirs.forEach(dir => {
      const visibleChildren = dir.querySelectorAll('.nav-item:not(.nav-dir):not([style*="display: none"])');
      const dirTitle = dir.dataset.title || '';
      const dirName = dir.dataset.filename || '';
      const selfMatches = !!query && (dirTitle.includes(query) || dirName.includes(query));
      if (visibleChildren.length > 0 || selfMatches) {
        dir.style.display = '';
        if (query || docsOnly) dir.classList.add('open');
      } else {
        dir.style.display = 'none';
      }
    });
  }

  function initSidebar() {
    const sidebarEl = document.getElementById('app-sidebar');
    const dirHeaders = document.querySelectorAll('.nav-dir-header');
    dirHeaders.forEach(header => {
      header.addEventListener('click', () => {
        const parent = header.closest('.nav-dir');
        if (parent) {
          parent.classList.toggle('open');
        }
      });
    });

    syncActiveNavLink(location.pathname);

    // Sidebar label mode (title vs. file name)
    const LABEL_KEY = 'markdown-web-nav-labels';
    const labelToggle = document.getElementById('sidebar-label-toggle');
    if (labelToggle && sidebarEl) {
      const applyLabelMode = (mode) => {
        const showFilenames = mode === 'filename';
        sidebarEl.classList.toggle('show-filenames', showFilenames);
        labelToggle.setAttribute('aria-pressed', showFilenames ? 'true' : 'false');
        labelToggle.setAttribute('title', showFilenames ? 'Show titles' : 'Show file names');
      };

      applyLabelMode(localStorage.getItem(LABEL_KEY) === 'filename' ? 'filename' : 'title');

      labelToggle.addEventListener('click', () => {
        const next = sidebarEl.classList.contains('show-filenames') ? 'title' : 'filename';
        localStorage.setItem(LABEL_KEY, next);
        applyLabelMode(next);
      });
    }

    // Sidebar scope toggle (all files vs. docs only)
    const SCOPE_KEY = 'markdown-web-nav-scope';
    const scopeToggle = document.getElementById('sidebar-scope-toggle');

    // Sidebar Filter input (matches both display titles and file names) + scope toggle compose:
    // a row must pass both the name filter and the docs-only check, so both handlers route
    // through this one function rather than independently stomping on `style.display`.
    const filterInput = document.getElementById('sidebar-filter');
    if (filterInput) {
      filterInput.addEventListener('input', applySidebarVisibility);
    }

    if (scopeToggle && sidebarEl) {
      const applyScopeMode = (mode) => {
        const docsOnly = mode === 'docs';
        sidebarEl.classList.toggle('docs-only', docsOnly);
        scopeToggle.setAttribute('aria-pressed', docsOnly ? 'true' : 'false');
        scopeToggle.setAttribute('title', docsOnly ? 'Show all files' : 'Docs only');
      };

      applyScopeMode(localStorage.getItem(SCOPE_KEY) === 'docs' ? 'docs' : 'all');

      scopeToggle.addEventListener('click', () => {
        const next = sidebarEl.classList.contains('docs-only') ? 'all' : 'docs';
        localStorage.setItem(SCOPE_KEY, next);
        applyScopeMode(next);
        applySidebarVisibility();
      });
    }

    applySidebarVisibility();

    // Mobile Sidebar Toggle
    const mobileBtn = document.getElementById('sidebar-toggle-btn');
    const sidebar = sidebarEl;
    if (mobileBtn && sidebar) {
      mobileBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        sidebar.classList.toggle('mobile-open');
      });

      document.addEventListener('click', (e) => {
        if (sidebar.classList.contains('mobile-open') && !sidebar.contains(e.target) && e.target !== mobileBtn) {
          sidebar.classList.remove('mobile-open');
        }
      });
    }
  }

  // --- Header Content Search (searches file contents via the server's `_search` route, unlike
  // the sidebar filter above which only matches names already loaded in the DOM) ---
  function initContentSearch() {
    const input = document.getElementById('content-search');
    const resultsEl = document.getElementById('content-search-results');
    const brandLink = document.querySelector('.brand-link');
    if (!input || !resultsEl || !brandLink) return;
    const homeUrl = brandLink.getAttribute('href');

    function closeResults() {
      resultsEl.hidden = true;
      resultsEl.innerHTML = '';
    }

    function renderResults(data) {
      resultsEl.innerHTML = '';
      // Carry the query onto the file link so landing there hands off straight into in-page find.
      const term = input.value.trim();
      if (data.results.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'search-result-empty';
        empty.textContent = `No matches for "${data.query}"`;
        resultsEl.appendChild(empty);
      } else {
        data.results.forEach(file => {
          const link = document.createElement('a');
          link.className = 'search-result-file';
          link.href = homeUrl + file.path + '?q=' + encodeURIComponent(term);

          const pathEl = document.createElement('span');
          pathEl.className = 'search-result-path';
          pathEl.textContent = file.path;
          link.appendChild(pathEl);

          file.lines.forEach(hit => {
            const lineEl = document.createElement('span');
            lineEl.className = 'search-result-line';
            lineEl.textContent = `L${hit.line}: ${hit.text}`;
            link.appendChild(lineEl);
          });

          resultsEl.appendChild(link);
        });
        if (data.truncated) {
          const note = document.createElement('div');
          note.className = 'search-result-truncated';
          note.textContent = `Showing first ${data.results.length}, more results truncated.`;
          resultsEl.appendChild(note);
        }
      }
      resultsEl.hidden = false;
    }

    function runSearch() {
      const term = input.value.trim();
      if (!term) {
        closeResults();
        return;
      }
      fetch(`${homeUrl}_search?q=${encodeURIComponent(term)}`)
        .then(res => res.json())
        .then(renderResults)
        .catch(() => closeResults());
    }

    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        runSearch();
      } else if (e.key === 'Escape') {
        closeResults();
        input.blur();
      }
    });

    document.addEventListener('click', (e) => {
      if (!resultsEl.hidden && !resultsEl.contains(e.target) && e.target !== input) {
        closeResults();
      }
    });
  }

  // --- Table of Contents (TOC) ScrollSpy ---
  // Re-run after every SPA swap (the aside and its headings are new DOM each time), so the
  // heading/link lists live at module scope and get rebuilt on each call rather than only once;
  // the scroll listener itself is attached at most once, reading whatever the latest call left in
  // these variables.
  let tocHeadings = [];
  let tocLinks = [];
  let tocActiveLink = null;
  let tocScrollBound = false;

  function updateActiveHeading() {
    if (tocHeadings.length === 0) return;
    const scrollY = window.scrollY;
    const offset = 90; // Header height + leeway

    let currentHeading = tocHeadings[0];
    for (let i = 0; i < tocHeadings.length; i++) {
      if (tocHeadings[i].offsetTop - offset <= scrollY) {
        currentHeading = tocHeadings[i];
      } else {
        break;
      }
    }

    const id = currentHeading.id;
    tocLinks.forEach(link => {
      if (link.getAttribute('href') === '#' + id) {
        if (tocActiveLink !== link) {
          if (tocActiveLink) tocActiveLink.classList.remove('active');
          link.classList.add('active');
          tocActiveLink = link;
        }
      }
    });
  }

  function initTOC() {
    tocLinks = Array.from(document.querySelectorAll('.toc-link'));
    tocActiveLink = null;
    tocHeadings = tocLinks.length === 0 ? [] : Array.from(
      document.querySelectorAll('.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4')
    ).filter(h => h.id);

    if (tocLinks.length === 0 || tocHeadings.length === 0) return;

    if (!tocScrollBound) {
      window.addEventListener('scroll', updateActiveHeading, { passive: true });
      tocScrollBound = true;
    }
    updateActiveHeading();
  }

  // --- SPA Router ---
  // Progressive enhancement over full navigations: the server renders a complete, correct page
  // for every URL regardless (direct links, hard refresh and fetch failures all still work), this
  // just avoids the full-page reload for same-origin content links.
  function isRoutableLink(link) {
    const href = link.getAttribute('href');
    if (!href || href.includes('raw=')) return false;
    if (link.target === '_blank') return false;
    let url;
    try {
      url = new URL(link.href, location.href);
    } catch (e) {
      return false;
    }
    if (url.origin !== location.origin) return false;
    if (url.pathname.startsWith('/_assets/')) return false;
    // An in-page anchor — a bullet-list TOC, a footnote back-reference — has to reach the
    // browser's native same-page scroll; swapping the page for an identical copy of itself would
    // only replace the anchor jump with a wasted fetch.
    if (url.hash) return false;
    return link.matches('.nav-tree a, main.app-main ul li a, .brand-link, .search-result-file');
  }

  function afterContentSwap() {
    if (window.hljs) hljs.highlightAll();
    initTOC();
    syncActiveNavLink(location.pathname);
    initFileView();
    applySidebarVisibility();
  }

  function swapTo(href, pushState) {
    fetch(href)
      .then(res => {
        if (!res.ok) throw new Error('fetch failed');
        return res.text();
      })
      .then(text => {
        const doc = new DOMParser().parseFromString(text, 'text/html');
        const newMain = doc.querySelector('main.app-main');
        const curMain = document.querySelector('main.app-main');
        if (!newMain || !curMain) throw new Error('no .app-main in response');
        curMain.innerHTML = newMain.innerHTML;

        const curToc = document.querySelector('.app-toc');
        if (curToc) curToc.remove();
        const newToc = doc.querySelector('.app-toc');
        if (newToc) {
          curMain.insertAdjacentElement('afterend', newToc.cloneNode(true));
        }

        document.title = doc.title;
        if (pushState) history.pushState({}, '', href);
        afterContentSwap();
      })
      .catch(() => {
        window.location.href = href;
      });
  }

  function initRouter() {
    document.addEventListener('click', (e) => {
      if (e.defaultPrevented || e.button !== 0) return;
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const link = e.target.closest('a');
      if (!link || !isRoutableLink(link)) return;
      e.preventDefault();
      swapTo(link.href, true);
    });

    window.addEventListener('popstate', () => {
      swapTo(location.href, false);
    });
  }

  // --- File View: toolbar, in-page find, show-text and the ?q= search hand-off ---
  function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  }

  function initFileView() {
    const copyBtn = document.getElementById('file-copy-btn');
    if (copyBtn) {
      copyBtn.addEventListener('click', () => {
        fetch(location.pathname + '?raw=1')
          .then(res => res.text())
          .then(text => navigator.clipboard.writeText(text))
          .then(() => {
            const original = copyBtn.textContent;
            copyBtn.textContent = 'Copied';
            setTimeout(() => { copyBtn.textContent = original; }, 1200);
          })
          .catch(() => {});
      });
    }

    const showTextBtn = document.getElementById('file-show-text-btn');
    if (showTextBtn) {
      showTextBtn.addEventListener('click', () => {
        fetch(location.pathname + '?raw=1')
          .then(res => res.text())
          .then(text => {
            const fileView = showTextBtn.closest('.file-view');
            if (!fileView) return;
            fileView.classList.add('wide');
            // ponytail: skip Copy on this rarer opt-in path — same static shape toolbar_html's
            // find-only case renders server-side, just built by hand since there's no page reload.
            fileView.innerHTML =
              '<div class="file-toolbar"><a class="file-btn" href="?raw=1&dl=1">Download</a>' +
              '<button type="button" class="file-btn" id="file-find-btn">Find</button></div>' +
              '<div class="file-find-bar" id="file-find-bar" hidden>' +
              '<input type="text" id="file-find-input" placeholder="Find in file…" autocomplete="off">' +
              '<span class="file-find-count" id="file-find-count"></span>' +
              '<button type="button" class="file-find-nav" id="file-find-prev" aria-label="Previous match">↑</button>' +
              '<button type="button" class="file-find-nav" id="file-find-next" aria-label="Next match">↓</button>' +
              '<button type="button" class="file-find-nav" id="file-find-close" aria-label="Close find">✕</button>' +
              '</div>' +
              '<div id="file-content" class="file-content"><pre><code>' + escapeHtml(text) + '</code></pre></div>';
            initFileView();
          })
          .catch(() => {});
      });
    }

    const findBtn = document.getElementById('file-find-btn');
    const findBar = document.getElementById('file-find-bar');
    const findInput = document.getElementById('file-find-input');
    const findCount = document.getElementById('file-find-count');
    const findPrev = document.getElementById('file-find-prev');
    const findNext = document.getElementById('file-find-next');
    const findClose = document.getElementById('file-find-close');
    const content = document.getElementById('file-content');
    if (!findBtn || !findBar || !findInput || !findCount || !content) return;

    let marks = [];
    let activeIndex = -1;

    function clearMarks() {
      content.querySelectorAll('mark.file-find-hit').forEach(mark => {
        const parent = mark.parentNode;
        if (!parent) return;
        parent.replaceChild(document.createTextNode(mark.textContent), mark);
        parent.normalize();
      });
      marks = [];
      activeIndex = -1;
    }

    function runSearch(query) {
      clearMarks();
      if (!query) {
        findCount.textContent = '';
        return;
      }
      const lower = query.toLowerCase();
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          return node.parentNode && node.parentNode.closest('mark.file-find-hit')
            ? NodeFilter.FILTER_REJECT
            : NodeFilter.FILTER_ACCEPT;
        },
      });
      const textNodes = [];
      let node;
      while ((node = walker.nextNode())) textNodes.push(node);

      textNodes.forEach(textNode => {
        const text = textNode.nodeValue;
        const textLower = text.toLowerCase();
        if (!textLower.includes(lower)) return;
        const frag = document.createDocumentFragment();
        let last = 0;
        let idx;
        while ((idx = textLower.indexOf(lower, last)) !== -1) {
          if (idx > last) frag.appendChild(document.createTextNode(text.slice(last, idx)));
          const mark = document.createElement('mark');
          mark.className = 'file-find-hit';
          mark.textContent = text.slice(idx, idx + lower.length);
          frag.appendChild(mark);
          marks.push(mark);
          last = idx + lower.length;
        }
        if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
        textNode.parentNode.replaceChild(frag, textNode);
      });

      if (marks.length > 0) {
        activeIndex = 0;
        marks[0].classList.add('file-find-hit-active');
        marks[0].scrollIntoView({ block: 'center' });
        findCount.textContent = `1 / ${marks.length}`;
      } else {
        findCount.textContent = 'No matches';
      }
    }

    function goTo(delta) {
      if (marks.length === 0) return;
      marks[activeIndex].classList.remove('file-find-hit-active');
      activeIndex = (activeIndex + delta + marks.length) % marks.length;
      marks[activeIndex].classList.add('file-find-hit-active');
      marks[activeIndex].scrollIntoView({ block: 'center' });
      findCount.textContent = `${activeIndex + 1} / ${marks.length}`;
    }

    function closeFind() {
      findBar.hidden = true;
      clearMarks();
      findCount.textContent = '';
    }

    findBtn.addEventListener('click', () => {
      if (findBar.hidden) {
        findBar.hidden = false;
        findInput.focus();
      } else {
        closeFind();
      }
    });

    findInput.addEventListener('input', () => runSearch(findInput.value));
    findInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        goTo(e.shiftKey ? -1 : 1);
      } else if (e.key === 'Escape') {
        closeFind();
      }
    });
    if (findNext) findNext.addEventListener('click', () => goTo(1));
    if (findPrev) findPrev.addEventListener('click', () => goTo(-1));
    if (findClose) findClose.addEventListener('click', closeFind);

    // Search hand-off: a content-search result link (or any ?q= URL) opens find pre-run.
    const q = new URLSearchParams(location.search).get('q');
    if (q) {
      findBar.hidden = false;
      findInput.value = q;
      runSearch(q);
    }
  }

  // --- Initialize on DOMContentLoaded ---
  document.addEventListener('DOMContentLoaded', () => {
    initTheme();
    initSidebar();
    initContentSearch();
    initTOC();
    initRouter();
    initFileView();
  });
})();
