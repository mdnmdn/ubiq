// bridge.js — dependency-free shim exposing window.ubiq.post(msg) / window.ubiq.on(handler).
//
// Two transports:
//   A. Embedded webview (wry): window.ipc.postMessage carries frames out, and Rust delivers
//      inbound frames by calling window.__ubiq_receive(json) via evaluate_script. This path
//      keeps a future embedded container working with no change to any chrome page.
//   B. External browser (shipped now): frames go out via POST /bridge, and a long-poll GET
//      /bridge loop brings frames back, with backoff on network error.
(function () {
  "use strict";

  var parts = location.pathname.split("/").filter(Boolean);
  // Expect shape: _web/<app>/<token>/...
  var appIndex = parts.indexOf("_web");
  var app = appIndex >= 0 ? parts[appIndex + 1] : "";
  var token = appIndex >= 0 ? parts[appIndex + 2] : "";

  var handlers = [];

  function dispatch(frame) {
    for (var i = 0; i < handlers.length; i++) {
      handlers[i](frame);
    }
  }

  // wry defines `window.ipc.postMessage` in every page it loads, whether or not the host
  // registered a handler for it — so its presence proves nothing. Transport A is used only when
  // the host says so by injecting `window.__ubiq_ipc = true`, which it does alongside the
  // handler that reads those frames. Everything else, the embedded webview included, long-polls
  // the loopback bridge that serves this page.
  var hasIpc = window.__ubiq_ipc === true && !!(window.ipc && window.ipc.postMessage);

  // A dropped POST is retried rather than swallowed. `ready` is posted exactly once and the
  // panel's loader lifts on nothing else, so one lost request — a loopback port not yet
  // accepting, a transient OS error — used to mean a panel that loads for ever with no trace.
  var POST_TRIES = 5;

  function post(frame) {
    var envelope = { token: token, frame: frame };
    if (hasIpc) {
      window.ipc.postMessage(JSON.stringify(envelope));
      return;
    }
    var body = JSON.stringify(envelope);
    var attempt = function (left, wait) {
      fetch("bridge", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: body,
      })
        .then(function (response) {
          if (!response.ok) throw new Error("the bridge answered " + response.status);
        })
        .catch(function (error) {
          if (left > 1) {
            console.warn("ubiq bridge: retrying a " + frame.type + " frame:", error);
            setTimeout(function () {
              attempt(left - 1, Math.min(wait * 2, 4000));
            }, wait);
          } else {
            console.error("ubiq bridge: gave up on a " + frame.type + " frame:", error);
          }
        });
    };
    attempt(POST_TRIES, 250);
  }

  function on(handler) {
    handlers.push(handler);
  }

  if (hasIpc) {
    // Transport A: Rust calls this to deliver inbound frames.
    window.__ubiq_receive = function (json) {
      try {
        dispatch(JSON.parse(json));
      } catch (e) {}
    };
  } else {
    // Transport B: long-poll GET /bridge, backing off on error, stopping on pagehide.
    var stopped = false;
    var backoff = 500;
    var maxBackoff = 8000;

    window.addEventListener("pagehide", function () {
      stopped = true;
    });

    function poll() {
      if (stopped) return;
      fetch("bridge")
        .then(function (res) {
          return res.json();
        })
        .then(function (frames) {
          backoff = 500;
          if (Array.isArray(frames)) {
            for (var i = 0; i < frames.length; i++) {
              dispatch(frames[i]);
            }
          }
          poll();
        })
        .catch(function () {
          setTimeout(poll, backoff);
          backoff = Math.min(backoff * 2, maxBackoff);
        });
    }

    poll();
  }

  // macOS hands an arrow, Home, Delete-forward... to a text field's input machinery as a character
  // in the private-use area (U+F700..U+F8FF, `NSUpArrowFunctionKey` and friends) when the same
  // key event has already been through another input context — here, the host window's. The
  // field then types it as a glyph "not in the font". These are never text: veto them before
  // they land. Capturing, so the editor's own handlers never see the insertion either.
  var PRIVATE_USE = /[\uF700-\uF8FF]/;
  function guardText(doc) {
    doc.addEventListener("beforeinput", function (event) {
      if (typeof event.data === "string" && PRIVATE_USE.test(event.data)) {
        event.preventDefault();
        event.stopPropagation();
      }
    }, true);
    doc.addEventListener("keypress", function (event) {
      if (event.charCode >= 0xf700 && event.charCode <= 0xf8ff) event.preventDefault();
    }, true);
  }
  guardText(document);

  window.ubiq = { post: post, on: on, guardText: guardText };
})();
