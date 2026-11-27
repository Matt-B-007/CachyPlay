// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Staying up to date: the watcher's "live" events.

/* ---------- staying up to date ----------
   CachyPlay's watcher (in Rust, off the window's thread) sends a "live" event only when something
   changed: Steam opened or closed, a game started, ended or crashed, or a waiting change ran.
   Nothing runs here while the window is hidden. */
state.live = null;
state.lastLoad = Date.now();
state.openRuns = new Set();
state.pendingRefresh = false;

function blockedByUser() {
  const typing = document.activeElement && ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName);
  return typing || !!document.querySelector("dialog[open]") || state.busy;
}

async function onLive(l) {
  const prev = state.live;
  state.live = l;
  for (const x of l.ran || []) toast(x, x.startsWith("Done") ? "success" : "error");
  if ((l.ran || []).length) toast("Steam is closed and your changes are in.", "success", { label: "Open Steam", run: openSteam });
  if (!prev) return;
  const changed = prev.steam !== l.steam || prev.token !== l.token || (l.ran || []).length > 0
    || JSON.stringify(prev.waiting) !== JSON.stringify(l.waiting);
  if (!changed) return;
  if (document.hidden) { state.pendingRefresh = true; return; }
  // Don't redraw under someone who's typing or has a dialog open; catch up afterwards.
  if (blockedByUser()) {
    state.pendingRefresh = true;
    if (prev.steam !== l.steam) renderChrome();
    return;
  }
  state.pendingRefresh = false;
  state.lastLoad = Date.now();
  await load();
}

function catchUp() {
  if (state.pendingRefresh && !document.hidden && !blockedByUser()) {
    state.pendingRefresh = false;
    state.lastLoad = Date.now();
    load();
  }
}

// While a game runs, only the "Playing now, 12 of 18 min" card ticks over, once a minute.
async function tickProgress() {
  if (document.hidden || blockedByUser() || Date.now() - state.lastLoad < 60000) return;
  const ins = state.insights.get(state.selected);
  if (ins?.progress?.playing_secs == null) return;
  state.lastLoad = Date.now();
  try {
    const fresh = await invoke("game_insights", { appId: state.selected });
    state.insights.set(state.selected, fresh);
    const old = document.querySelector(".card.progress");
    if (old && state.detail) old.replaceWith(progressCard(state.detail, fresh.progress));
  } catch {
    /* next minute */
  }
}

function startWatching() {
  invoke("live_status").then((l) => { state.live = l; }).catch(() => {});
  window.__TAURI__.event.listen("live", (e) => onLive(e.payload));
  setInterval(tickProgress, 15000);
  document.addEventListener("visibilitychange", () => {
    invoke("set_visible", { visible: !document.hidden }).catch(() => {});
    if (document.hidden) stopGamepad();
    else {
      if (state.settings && state.settings.big_screen) startGamepad();
      catchUp();
    }
  });
  document.addEventListener("close", () => setTimeout(catchUp, 0), true); // a dialog closed
  document.addEventListener("focusout", () => setTimeout(catchUp, 0));
}

async function showLastCrash() {
  try {
    state.lastCrash = await invoke("last_crash");
  } catch {
    state.lastCrash = null;
  }
  if (state.lastCrash) redrawBanners();
}

function dismissCrash() {
  state.lastCrash = null;
  invoke("dismiss_crash").catch(() => {});
  redrawBanners();
}

function redrawBanners() {
  if (!state.overview) return;
  if (state.detail && state.selected) renderMain();
  else renderEmptyMain();
}
