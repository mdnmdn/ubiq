// app.js — the draw.io chrome. A classic script: this tenant imports no module of its own.
//
// draw.io is not a component this page mounts — it is a whole webapp, mirrored under `vendor/`
// and framed. It speaks its own embed protocol (`?embed=1&proto=json`): JSON strings over
// `postMessage`, same origin, both ways. This file is the translation between that protocol and
// Ubiq's bridge frames, and nothing else.
//
// Boot order:
//   1. register the bridge handler and queue whatever arrives before the frame is up;
//   2. point the frame at the mirrored `index.html` with the embed parameters;
//   3. wait for the editor's `init` event, with a ceiling that fails loudly;
//   4. drain the queue and `post({type:"ready"})` — an `open` can be accepted from here;
//   5. `load` the document into the editor, and translate its events back.
(function () {
  "use strict";

  // The mirror's root is the webapp's own root: draw.io is a whole site rather than a package
  // imported through a map, and every reference inside it is relative to the directory its
  // `index.html` sits in. So the bundle is mirrored from `src/main/webapp/` down and this page
  // names no version at all — the host says where the bundle landed, and `vendor/` is it.
  var APP = "vendor/index.html";

  // How long the drawing must be still before the document is written, matching the Excalidraw
  // tenant. draw.io's own autosave already coalesces; this is the second, slower gate.
  var IDLE_MS = 600;
  // The editor loads a few megabytes of mirrored script before it says `init`.
  var INIT_CEILING_MS = 30000;

  var frame = document.getElementById("frame");
  var failure = document.getElementById("failure");

  var queued = [];
  var handle = function (f) { queued.push(f); };
  window.ubiq.on(function (f) { handle(f); });

  var inited = false;
  var pending = null;      // a document that arrived before the editor was up
  var current = "";        // the last document loaded or reported, for change comparison
  var palette = "light";
  var idle = null;
  var ceiling = null;
  var saveWanted = false;   // an export is standing in for a save: the editor owns the live xml

  function post(f) { window.ubiq.post(f); }

  function fail(message) {
    failure.hidden = false;
    failure.textContent = "draw.io could not start.\n\n" + message;
    post({ type: "error", message: String(message) });
  }

  function send(action) {
    if (frame.contentWindow) frame.contentWindow.postMessage(JSON.stringify(action), "*");
  }

  // --- The frame ----------------------------------------------------------------------------

  // `offline=1&stealth=1` keep the webapp off every remote origin: no analytics, no font CDN, no
  // storage back end, no plugin fetch. The mirror has no such files and the policy would refuse
  // them anyway, so this only spares us the failed attempts. The storage back ends are off one
  // by one because the file lives in Ubiq's buffer and nowhere else.
  function src() {
    return APP + "?embed=1&proto=json&spin=1&libraries=1&noExitBtn=1&saveAndExit=0" +
      "&offline=1&stealth=1&analytics=0&picker=0&mode=device&local=0&drive=0&od=0&gh=0" +
      "&gl=0&db=0&dbx=0&tr=0&plugins=0&lang=en&dark=" + (palette === "dark" ? "1" : "0");
  }

  function mount() {
    inited = false;
    frame.src = src();
    clearTimeout(ceiling);
    ceiling = setTimeout(function () {
      if (!inited) fail(APP + ": the editor did not report `init` within " +
        INIT_CEILING_MS / 1000 + "s. The vendor mirror may be incomplete.");
    }, INIT_CEILING_MS);
  }

  // --- The editor's side --------------------------------------------------------------------

  window.addEventListener("message", function (evt) {
    if (!frame.contentWindow || evt.source !== frame.contentWindow) return;
    var msg;
    try { msg = JSON.parse(evt.data); } catch (e) { return; }
    if (!msg || !msg.event) return;

    switch (msg.event) {
      case "init":
        clearTimeout(ceiling);
        inited = true;
        onReady();
        break;
      case "autosave":
        settle(msg.xml);
        break;
      case "save":
        clearTimeout(idle);
        current = msg.xml;
        post({ type: "save", document: msg.xml });
        requestPreview();
        break;
      case "export":
        onExport(msg);
        break;
      default:
        // `exit`, `load`, `configure`, anything a newer build adds: not ours to act on.
        break;
    }
  });

  function load(document_) {
    current = document_;
    // `autosave:1` is what makes the editor report every settled edit rather than only saves.
    send({ action: "load", autosave: 1, xml: document_ });
    requestPreview();
  }

  function settle(xml) {
    if (xml === current) return;
    post({ type: "dirty" });
    clearTimeout(idle);
    idle = setTimeout(function () {
      current = xml;
      post({ type: "changed", document: xml });
      requestPreview();
    }, IDLE_MS);
  }

  // The Preview position has no native draw.io renderer behind it, so the editor renders it:
  // an SVG export, cached by the interface beside the file. It is a document, like every other
  // frame — the interface asks for nothing and the page volunteers nothing else.
  function requestPreview() {
    if (inited) send({ action: "export", format: "xmlsvg", background: null, scale: 1 });
  }

  function onExport(msg) {
    if (saveWanted) {
      saveWanted = false;
      // The export carries the xml as it is on screen, which is what a save must write.
      if (typeof msg.xml === "string") {
        current = msg.xml;
        post({ type: "save", document: msg.xml });
      }
    }
    var data = msg.data || "";
    var comma = data.indexOf(",");
    if (data.indexOf("base64") < 0 || comma < 0) return;
    var svg;
    try {
      // The export is a base64 data URI of UTF-8 SVG; `atob` gives bytes, not characters.
      svg = new TextDecoder().decode(
        Uint8Array.from(atob(data.slice(comma + 1)), function (c) { return c.charCodeAt(0); })
      );
    } catch (e) {
      return;
    }
    post({ type: "preview", svg: svg });
  }

  // --- Ubiq's side --------------------------------------------------------------------------

  function onFrame(f) {
    if (!f || !f.type) return;
    switch (f.type) {
      case "open":
        palette = f.palette || palette;
        if (inited) load(f.document || ""); else pending = f.document || "";
        break;
      case "reload":
        if (inited) load(f.document || ""); else pending = f.document || "";
        break;
      case "palette":
        if (f.palette && f.palette !== palette) {
          // The webapp reads its theme once, at boot: a palette change is a remount, with the
          // document on screen carried back into it.
          palette = f.palette;
          pending = current;
          mount();
        }
        break;
      case "save_requested":
        // The document travels with the save, so ask the editor for what is on screen now
        // rather than for whatever the idle gate last reported.
        if (inited) {
          saveWanted = true;
          send({ action: "export", format: "xmlsvg", background: null, scale: 1 });
        } else {
          post({ type: "save", document: current });
        }
        break;
      default:
        break;
    }
  }

  var booted = false;

  function onReady() {
    handle = onFrame;
    var drained = queued;
    queued = [];
    // `ready` is what lifts the panel's loader, and it is sent once: a remount for a palette
    // change would otherwise be answered with a second `open` racing the document being carried
    // across, and the loader has long since lifted.
    if (!booted) {
      booted = true;
      post({ type: "ready" });
    }
    for (var i = 0; i < drained.length; i++) onFrame(drained[i]);
    if (pending !== null) { load(pending); pending = null; }
  }

  try {
    mount();
  } catch (err) {
    fail(err && err.message ? err.message : String(err));
  }
})();
