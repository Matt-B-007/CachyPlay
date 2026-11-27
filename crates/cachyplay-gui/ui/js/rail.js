// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// The game list on the left.

/* ---------- game rail ---------- */
function renderRail() {
  const rail = $("rail");
  const q = state.query;
  const games = state.overview.games.filter((g) => !q || g.name.toLowerCase().includes(q));
  const attention = games.filter(needsAttention).sort((a, b) => b.gpu_crashes - a.gpu_crashes);
  const rest = games.filter((g) => !needsAttention(g));
  const parts = [];
  if (attention.length) {
    parts.push(h("h2", { class: "rail-title attention" }, "Needs attention"));
    parts.push(...attention.map(railItem));
  }
  if (rest.length) {
    parts.push(h("h2", { class: "rail-title" }, attention.length ? "All games" : `Your games (${rest.length})`));
    parts.push(...rest.map(railItem));
  }
  if (!games.length) {
    parts.push(h("p", { class: "rail-empty" }, state.overview.games.length
      ? "No games match your search."
      : state.overview.steam ? "No installed Steam games found." : "Once Steam is found, your games appear here."));
  }
  rail.replaceChildren(...parts);
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
