/* =====================================================================
   UBIQ DESIGN SYSTEM  ·  ubiq.js
   Shared chrome behaviour for every app screen (except index.html):
   theme (light / dark / system), sidebar collapse, .sw-toggle delegation,
   and a single xterm.js terminal factory so palette logic lives in one place.

   Load AFTER xterm.js + addon-fit (when a page uses terminals), e.g.
     <script src=".../xterm.js"></script>
     <script src=".../addon-fit.js"></script>
     <script src="js/ubiq.js"></script>
     <script> ...page code, calls Ubiq.makeTerm(...)... </script>

   Hooks a page may set:  Ubiq.onTheme(eff)   Ubiq.onSidebar(collapsed)
   ===================================================================== */
(function () {
  var root = document.documentElement;
  var THEME_KEY = "ubiq-theme", MODE_KEY = "ubiq-theme-mode", SIDEBAR_KEY = "ubiq-sidebar";
  var U = (window.Ubiq = window.Ubiq || {});
  U.TERMS = [];

  /* ---------------------------------------------------------- THEME */
  function effective(mode) {
    return mode === "system"
      ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")
      : mode;
  }
  U.currentMode = function () {
    return localStorage.getItem(MODE_KEY) || localStorage.getItem(THEME_KEY) || "dark";
  };
  U.applyTheme = function (mode) {
    if (mode !== "light" && mode !== "dark" && mode !== "system") mode = "dark";
    localStorage.setItem(MODE_KEY, mode);
    var eff = effective(mode);
    root.dataset.theme = eff;
    localStorage.setItem(THEME_KEY, eff);
    document.querySelectorAll("#themeSeg [data-t]").forEach(function (b) {
      b.classList.toggle("on", b.dataset.t === mode);
    });
    U.refreshTermThemes();
    if (typeof U.onTheme === "function") U.onTheme(eff);
  };
  U.toggleTheme = function () {
    U.applyTheme(root.dataset.theme === "dark" ? "light" : "dark");
  };

  /* ------------------------------------------------- TERMINAL FACTORY */
  U.termTheme = function () {
    var dark = root.dataset.theme === "dark";
    return dark
      ? { background:"#14161b", foreground:"#cdd2da", cursor:"#7aa2f7", cursorAccent:"#14161b",
          selectionBackground:"#2a3a55", black:"#2a2e37", red:"#f7768e", green:"#9ece6a",
          yellow:"#e0af68", blue:"#7aa2f7", magenta:"#bb9af7", cyan:"#7dcfff", white:"#c0caf5",
          brightBlack:"#565f89", brightRed:"#ff7a93", brightGreen:"#9ece6a", brightYellow:"#e0af68",
          brightBlue:"#7aa2f7", brightMagenta:"#bb9af7", brightCyan:"#7dcfff", brightWhite:"#e9ebf2" }
      : { background:"#f8f9fb", foreground:"#2b3138", cursor:"#3056d3", cursorAccent:"#f8f9fb",
          selectionBackground:"#cfe0ff", black:"#3b424b", red:"#cf222e", green:"#1a7f37",
          yellow:"#9a6700", blue:"#3056d3", magenta:"#8250df", cyan:"#0a7ea4", white:"#6e7781",
          brightBlack:"#8c95a1", brightRed:"#cf222e", brightGreen:"#1a7f37", brightYellow:"#9a6700",
          brightBlue:"#3056d3", brightMagenta:"#8250df", brightCyan:"#0a7ea4", brightWhite:"#24292f" };
  };
  U.refreshTermThemes = function () {
    var th = U.termTheme();
    U.TERMS.forEach(function (t) { try { t.options.theme = th; } catch (e) {} });
  };
  // opts: { fontSize, cursorBlink, scrollback }
  U.makeTerm = function (mount, opts) {
    opts = opts || {};
    var el = typeof mount === "string" ? document.getElementById(mount) : mount;
    var term = new window.Terminal({
      fontFamily: '"SF Mono","JetBrains Mono",ui-monospace,Menlo,monospace',
      fontSize: opts.fontSize || 13,
      lineHeight: 1.35,
      cursorBlink: opts.cursorBlink !== undefined ? opts.cursorBlink : false,
      convertEol: true,
      theme: U.termTheme(),
      scrollback: opts.scrollback || 1000
    });
    var fit;
    if (window.FitAddon) { fit = new window.FitAddon.FitAddon(); term.loadAddon(fit); }
    term.open(el);
    if (fit) { try { fit.fit(); } catch (e) {} term._fit = fit; }
    U.TERMS.push(term);
    return term;
  };
  U.refitTerms = function () {
    U.TERMS.forEach(function (t) { if (t._fit) { try { t._fit.fit(); } catch (e) {} } });
  };
  // write an array of [text] (or [text,delay]) lines instantly — no animation
  U.writeLines = function (term, lines) {
    lines.forEach(function (l) { term.writeln(Array.isArray(l) ? l[0] : l); });
  };

  /* -------------------------------------------------- SIDEBAR COLLAPSE */
  U.initSidebar = function () {
    var sb = document.querySelector(".sidebar");
    if (!sb) return;
    var btn = document.getElementById("collapseBtn");
    U.setCollapsed = function (c) {
      sb.classList.toggle("collapsed", c);
      localStorage.setItem(SIDEBAR_KEY, c ? "1" : "0");
      setTimeout(function () {
        U.refitTerms();
        if (typeof U.onSidebar === "function") U.onSidebar(c);
      }, 230);
    };
    if (btn) btn.onclick = function () { U.setCollapsed(!sb.classList.contains("collapsed")); };
    if (localStorage.getItem(SIDEBAR_KEY) === "1") sb.classList.add("collapsed");
  };

  /* ------------------------------------------------------------- INIT */
  function init() {
    U.applyTheme(U.currentMode());
    var tb = document.getElementById("themeBtn");
    if (tb) tb.onclick = U.toggleTheme;
    var seg = document.getElementById("themeSeg");
    if (seg) seg.addEventListener("click", function (e) {
      var b = e.target.closest("[data-t]");
      if (b) U.applyTheme(b.dataset.t);
    });
    U.initSidebar();
    // any .sw-toggle anywhere flips on click
    document.body.addEventListener("click", function (e) {
      var t = e.target.closest(".sw-toggle");
      if (t) t.classList.toggle("on");
    });
    window.addEventListener("resize", U.refitTerms);
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
