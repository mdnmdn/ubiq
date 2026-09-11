// app.js — the Excalidraw chrome. A classic script, deliberately: an import map must be in the
// document before any module loads, so nothing here may be a module until the map is injected.
//
// Boot order, and every step of it is load-bearing:
//   1. register the bridge handler and queue whatever arrives before the mount;
//   2. fetch `vendor/importmap.json` — the map the host generated beside the mirror;
//   3. inject it as an inline `<script type="importmap">` carrying the page's CSP nonce.
//      `dist/prod/index.js` imports 32 bare specifiers a browser cannot resolve, and the map is
//      also what collapses the stray React copies `+esm` hard-pins onto one;
//   4. set `window.EXCALIDRAW_ASSET_PATH` to an *absolute* URL of the directory holding `fonts/`.
//      A `/`-relative value resolves against `location.origin` and throws; any font it then fails
//      to find is a silent attempt at `esm.sh`, which the CSP turns into a loud one;
//   5. link `index.css` (its four Assistant faces are relative `url("./fonts/…")`, which is why
//      the stylesheet must be loaded from inside the mirror and not copied);
//   6. dynamically import React, react-dom/client and the entry point, and mount.
//
// There is no service worker. There is one module worker — Excalidraw's font subsetting, built
// from `import.meta.url`, so it is same-origin and `default-src 'self'` already permits it; it is
// wrapped upstream in a try/catch with a main-thread fallback. Nothing here constructs a worker.
(function () {
  "use strict";

  // The mirror's layout, and the one place this page names a version. `crates/ubiq` cannot depend
  // on `crates/ubiq-host`, so the host's `manifest::ASSET_SUBPATH` cannot be read from here; this
  // must equal it. A version bump that moves the directory shows up as the explicit failure
  // below rather than as a blank page.
  var DIST = "vendor/npm/@excalidraw/excalidraw@0.18.0/dist/prod/";
  var IMPORT_MAP = "vendor/importmap.json";

  // How long the drawing must be still before the document is serialised. `onChange` fires
  // continuously; `dirty` goes out on the first change, `changed` only on idle.
  var IDLE_MS = 600;

  var failure = document.getElementById("failure");
  var rootEl = document.getElementById("root");
  var nonce = (document.getElementById("ubiq-app") || {}).dataset;
  nonce = nonce ? nonce.nonce : "";

  var queued = [];
  var handle = function (frame) {
    queued.push(frame);
  };
  window.ubiq.on(function (frame) {
    handle(frame);
  });

  function post(frame) {
    window.ubiq.post(frame);
  }

  function fail(message) {
    failure.hidden = false;
    failure.textContent = "Excalidraw could not start.\n\n" + message;
    post({ type: "error", message: String(message) });
  }

  // --- Scene comparison -------------------------------------------------------------------
  //
  // Excalidraw reserialises the whole document: key order, a `version`/`versionNonce` pair bumped
  // on every element it touches, `appState` fields the file may never have carried. Writing that
  // into the buffer on every change would mark a tab dirty for having been looked at, so the
  // buffer is written only when this signature actually moves. It does not make the diff small.

  var VOLATILE = { version: true, versionNonce: true, updated: true };

  function stable(value) {
    if (value === null || typeof value !== "object") return JSON.stringify(value);
    if (Array.isArray(value)) return "[" + value.map(stable).join(",") + "]";
    var keys = Object.keys(value).sort();
    var parts = [];
    for (var i = 0; i < keys.length; i++) {
      parts.push(JSON.stringify(keys[i]) + ":" + stable(value[keys[i]]));
    }
    return "{" + parts.join(",") + "}";
  }

  /// The comparable shape of a serialised document: its elements without the churn counters, and
  /// the `appState` subset Excalidraw itself chose to persist.
  function signature(doc) {
    var elements = (doc.elements || []).map(function (element) {
      var out = {};
      Object.keys(element).forEach(function (key) {
        if (!VOLATILE[key]) out[key] = element[key];
      });
      return out;
    });
    return stable({ elements: elements, appState: doc.appState || {}, files: doc.files || {} });
  }

  // --- Mount ------------------------------------------------------------------------------

  function injectImportMap(text) {
    var el = document.createElement("script");
    el.type = "importmap";
    if (nonce) el.nonce = nonce;
    el.textContent = text;
    // Before any module is loaded, and before the stylesheet, so nothing can race it.
    document.head.appendChild(el);
  }

  function linkStylesheet() {
    var link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = DIST + "index.css";
    document.head.appendChild(link);
  }

  function boot() {
    return fetch(IMPORT_MAP)
      .then(function (res) {
        if (!res.ok) throw new Error(IMPORT_MAP + ": HTTP " + res.status);
        return res.text();
      })
      .then(function (text) {
        injectImportMap(text);

        // Absolute, and pointing at the directory that contains `fonts/`.
        window.EXCALIDRAW_ASSET_PATH = new URL(DIST, document.baseURI).href;
        linkStylesheet();

        return Promise.all([
          import("react"),
          import("react-dom/client"),
          import("./" + DIST + "index.js"),
        ]);
      })
      .then(function (mods) {
        // `+esm` output exposes CommonJS packages as a default export and a set of named ones;
        // which of the two carries the members differs per package, so take whichever does.
        var react = mods[0].createElement ? mods[0] : mods[0].default;
        var reactDom = mods[1].createRoot ? mods[1] : mods[1].default;
        mount(react, reactDom, mods[2]);
      });
  }

  function mount(React, ReactDOM, Excal) {
    if (!Excal || !Excal.Excalidraw) {
      throw new Error("the bundle exports no Excalidraw component");
    }

    var api = null;
    var theme = "light";
    var baseline = null; // the signature of what is in the buffer
    var envelope = {}; // the file's own `source` and `type`, preserved across a round trip
    var dirty = false;
    var idle = null;
    var root = ReactDOM.createRoot(rootEl);

    function draw() {
      root.render(
        React.createElement(Excal.Excalidraw, {
          theme: theme,
          excalidrawAPI: function (value) {
            api = value;
          },
          onChange: changed,
          // The panel's own header is above this page, not in it, so the one action that has to
          // be reachable from inside the drawing gets a button of its own beside the toolbar.
          renderTopRightUI: function () {
            return React.createElement(
              "button",
              {
                className: "ubiq-save",
                title: "Save this drawing to the file",
                onClick: save,
              },
              "Save",
            );
          },
        }),
      );
    }

    function serialise() {
      if (!api) return null;
      var text = Excal.serializeAsJSON(
        api.getSceneElements(),
        api.getAppState(),
        api.getFiles(),
        "local",
      );
      var doc = JSON.parse(text);
      // Mitigation (b): the file's own `source` and `type` survive, rather than the editor's.
      if (envelope.source !== undefined) doc.source = envelope.source;
      if (envelope.type !== undefined) doc.type = envelope.type;
      return doc;
    }

    function flush() {
      idle = null;
      var doc = serialise();
      if (!doc) return;
      var now = signature(doc);
      // Mitigation (a): a document merely opened and looked at never reaches the buffer.
      if (now === baseline) return;
      baseline = now;
      post({ type: "changed", document: JSON.stringify(doc, null, 2) + "\n" });
    }

    /// Write what is on screen now, rather than what the debounce last sent.
    ///
    /// The baseline moves with it, so the change that has just been written is not sent a second
    /// time by the idle timer that was already running.
    function save() {
      if (idle) {
        clearTimeout(idle);
        idle = null;
      }
      var doc = serialise();
      if (!doc) return;
      baseline = signature(doc);
      dirty = false;
      post({ type: "save", document: JSON.stringify(doc, null, 2) + "\n" });
    }

    function changed() {
      if (baseline === null) return; // still loading the scene
      if (!dirty) {
        var doc = serialise();
        if (doc && signature(doc) === baseline) return;
        dirty = true;
        post({ type: "dirty" });
      }
      if (idle) clearTimeout(idle);
      idle = setTimeout(flush, IDLE_MS);
    }

    function load(text) {
      var parsed;
      try {
        parsed = text && text.trim() ? JSON.parse(text) : {};
      } catch (err) {
        post({ type: "error", message: "not a readable Excalidraw document: " + err });
        return;
      }
      envelope = { source: parsed.source, type: parsed.type };
      var scene = Excal.restore
        ? Excal.restore(parsed, null, null)
        : {
            elements: parsed.elements || [],
            appState: parsed.appState || {},
            files: parsed.files || {},
          };
      baseline = null;
      dirty = false;
      if (idle) {
        clearTimeout(idle);
        idle = null;
      }
      api.updateScene({ elements: scene.elements, appState: scene.appState });
      if (scene.files && Object.keys(scene.files).length) api.addFiles(Object.values(scene.files));
      // The baseline is what the editor makes of the file, not the file — otherwise `restore`'s
      // own normalisation would read as an edit the moment the scene lands.
      baseline = signature(serialise() || {});
    }

    function apply(frame) {
      if (!frame || typeof frame.type !== "string") return;
      switch (frame.type) {
        case "open":
          if (frame.palette) {
            theme = frame.palette === "dark" ? "dark" : "light";
            draw();
          }
          load(frame.document);
          break;
        case "reload":
          load(frame.document);
          break;
        case "palette":
          theme = frame.palette === "dark" ? "dark" : "light";
          draw();
          break;
        case "save_requested":
          save();
          break;
        default:
          // A frame from a newer interface. Dropped, exactly as the Rust side drops ours.
          break;
      }
    }

    // ⌘S and Ctrl-S never reach GPUI: while the webview has the keyboard the native view
    // swallows the chord, so the page is the only place that can hear it. Captured, so the
    // component's own handlers cannot take it first.
    document.addEventListener(
      "keydown",
      function (event) {
        if (event.key !== "s" || !(event.metaKey || event.ctrlKey)) return;
        event.preventDefault();
        event.stopPropagation();
        save();
      },
      true,
    );

    draw();

    // The component calls `excalidrawAPI` during its first render; wait for it rather than
    // assuming, then drain whatever arrived while the bundle was still downloading.
    var waited = 0;
    (function ready() {
      if (!api) {
        if ((waited += 50) > 30000) {
          fail("the component mounted but never handed over its API");
          return;
        }
        setTimeout(ready, 50);
        return;
      }
      handle = apply;
      queued.forEach(apply);
      queued = [];
      post({ type: "ready" });
    })();
  }

  try {
    boot().catch(function (err) {
      fail(err && err.stack ? err.stack : String(err));
    });
  } catch (err) {
    fail(err && err.stack ? err.stack : String(err));
  }
})();
