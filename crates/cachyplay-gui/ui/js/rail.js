// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// The game list on the left.

/* ---------- game rail ---------- */
function renderRail() {
  const rail = $("rail");
  const q = state.query;
  const games = state.overview.games.filter((g) => !q || g.name.toLowerCase().includes(q));
  const parts = [h("button", {
    class: "rail-item rail-home",
    "aria-current": state.selected ? "false" : "true",
    onclick: goHome,
  }, h("span", { class: "thumb home-thumb" }, icon("gamepad", 18)), h("span", { class: "rail-text" }, h("span", { class: "rail-name" }, "Library"), h("span", { class: "rail-status" }, "All games at a glance")))];
  for (const [title, cls, list] of railGroups(games)) {
    if (!list.length) continue;
    parts.push(h("h2", { class: `rail-title ${cls}` }, title, h("span", { class: "rail-count" }, String(list.length))));
    parts.push(...list.map(railItem));
  }
  if (!games.length) {
    parts.push(h("p", { class: "rail-empty" }, state.overview.games.length
      ? "No games match your search."
      : state.overview.steam ? "No installed Steam games found." : "Once Steam is found, your games appear here."));
  }
  rail.replaceChildren(...parts);
}

/* Needs you (not working or crashing), Testing a fix, Working (with or without a fix), and every
   other game. */
function railGroup(g) {
  if (needsAttention(g)) return 0;
  if (g.light === "yellow" || ["Testing a fix", "Seems to work", "Playing now"].includes(g.stage)) return 1;
  if (g.light === "green") return 2;
  return 3;
}

function railGroups(games) {
  const by = [[], [], [], []];
  for (const g of games) by[railGroup(g)].push(g);
  by[0].sort((a, b) => b.gpu_crashes - a.gpu_crashes);
  return [["Needs you", "attention", by[0]], ["Testing a fix", "testing", by[1]], ["Working", "working", by[2]], ["Other games", "", by[3]]];
}

/* The selected game (or Library, when no game is open) is marked in the list. */
function markRail() {
  for (const c of document.querySelectorAll(".rail-item")) {
    const here = c.classList.contains("rail-home") ? !state.selected : c.dataset.app === state.selected;
    c.setAttribute("aria-current", here ? "true" : "false");
  }
}

function goHome() {
  state.selected = null;
  state.detail = null;
  markRail();
  renderHome();
  $("main").scrollTop = 0;
}

function railItem(g) {
  const thumb = h("span", { class: "thumb" }, h("span", {}, g.name.slice(0, 2).toUpperCase()));
  loadCover(g.app_id).then((url) => {
    if (!url) return;
    thumb.replaceChildren(h("img", { src: url, alt: "" }));
  });
  return h("button", {
    class: `rail-item ${g.badge} light-${g.light}`,
    "data-app": g.app_id,
    "aria-current": state.selected === g.app_id ? "true" : "false",
    onclick: () => { state.tab = "overview"; showGame(g.app_id); },
  },
    thumb,
    h("span", { class: "rail-text" },
      h("span", { class: "rail-name" }, g.name),
      h("span", { class: "rail-status" }, h("span", { class: `light ${g.light}`, "aria-hidden": "true" }), statusText(g))));
}

/* Covers come from cover://localhost/<appid>, served straight from Steam's cache (no copying the
   image through the app). Resolves to the link once the image loads, or null if there's none. */
function loadCover(appId) {
  if (state.covers.has(appId)) return state.covers.get(appId);
  const url = `cover://localhost/${encodeURIComponent(appId)}`;
  const p = new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(url);
    img.onerror = () => resolve(null);
    img.src = url;
  });
  state.covers.set(appId, p);
  return p;
}
