// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// The library home: games that need you (with their next step), this PC at a glance, and every
// game as a tile. Shown when no game is open, and in big-screen mode it's the couch view.

state.homeFilter = "all";
state.homeLoading = new Set();

function renderHome() {
  const o = state.overview;
  if (!o) return;
  const games = o.games;
  const groups = railGroups(games);
  const needs = groups[0][2];
  const testing = groups[1][2];
  const on = games.filter((g) => g.enabled).length;
  // Next steps come from each game's insights; load the ones not seen yet (a few games at most).
  for (const g of [...needs.slice(0, 4), ...testing.slice(0, 4)]) {
    if (state.insights.has(g.app_id) || state.homeLoading.has(g.app_id)) continue;
    state.homeLoading.add(g.app_id);
    invoke("game_insights", { appId: g.app_id }).then((ins) => {
      state.insights.set(g.app_id, ins);
      if (!state.selected) renderHome();
    }).catch(() => {}).finally(() => state.homeLoading.delete(g.app_id));
  }
  const main = $("main");
  const scroll = main.scrollTop;
  main.replaceChildren(...banners(),
    h("section", { class: "home" },
      h("div", { class: "home-head" },
        h("h1", {}, "Library"),
        h("p", { class: "muted" }, `${games.length} ${games.length === 1 ? "game" : "games"} · CachyPlay on for ${on}`)),
      needs.length ? h("section", { class: "home-section" },
        h("h2", { class: "home-title attention" }, "Needs you"),
        h("div", { class: "needs-grid" }, needs.slice(0, 4).map(needsCard)),
        needs.length > 4 ? h("p", { class: "muted" }, `And ${needs.length - 4} more in the game list.`) : null) : null,
      testing.length ? h("section", { class: "home-section" },
        h("h2", { class: "home-title testing" }, "Testing a fix"),
        h("div", { class: "needs-grid" }, testing.slice(0, 4).map(needsCard)),
        testing.length > 4 ? h("p", { class: "muted" }, `And ${testing.length - 4} more in the game list.`) : null) : null,
      h("section", { class: "home-section" },
        h("h2", { class: "home-title" }, "This PC"),
        h("div", { class: "sys-grid" }, ...systemCards())),
      h("section", { class: "home-section" },
        h("div", { class: "home-title-row" },
          h("h2", { class: "home-title" }, "Games"),
          h("div", { class: "filter-tabs", role: "tablist", "aria-label": "Show" },
            [["all", `All ${games.length}`], ["working", `Working ${groups[2][2].length}`], ["on", `CachyPlay on ${on}`]].map(([id, label]) =>
              h("button", { role: "tab", "aria-selected": state.homeFilter === id ? "true" : "false", onclick: () => { state.homeFilter = id; renderHome(); } }, label)))),
        tileGrid(games))));
  main.scrollTop = scroll;
}

function openGame(appId) {
  state.tab = "overview";
  showGame(appId);
}

function coverInto(box, appId) {
  loadCover(appId).then((url) => { if (url) box.replaceChildren(h("img", { src: url, alt: "" })); });
  return box;
}

function needsCard(g) {
  const ins = insightsFor(g.app_id);
  const sum = ins && ins.summary;
  const next = sum ? (sum.todo || sum.what) : "Working out the next step…";
  return h("article", { class: `card needs-card light-${g.light}` },
    coverInto(h("span", { class: "thumb big-thumb" }, h("span", {}, g.name.slice(0, 2).toUpperCase())), g.app_id),
    h("div", { class: "needs-text" },
      h("strong", {}, g.name),
      h("span", { class: "rail-status" }, h("span", { class: `light ${g.light}`, "aria-hidden": "true" }), statusText(g)),
      h("p", { class: "muted" }, h("strong", {}, "Next: "), next)),
    h("button", { class: "secondary", onclick: () => openGame(g.app_id) }, "Open"));
}

function systemCards() {
  const o = state.overview;
  const sys = o.system || {};
  const checks = state.pcCheck || [];
  const bad = checks.filter((c) => c.level === "bad").length;
  const warn = checks.filter((c) => c.level === "warn").length;
  const u = state.update;
  // Like the banner: games that work again don't count.
  const fixed = new Set(o.games.filter((g) => g.light === "green").map((g) => g.name));
  const crashing = o.system_crashes ? o.system_crashes.programs.filter((n) => !fixed.has(n)) : [];
  const sysCard = (title, value, detail, cls, onclick) => h("article", { class: `card sys-card ${cls || ""}` },
    h("h3", { class: "sys-title" }, title),
    h("strong", { class: "sys-value" }, value || "Unknown"),
    h("span", { class: "sys-detail" }, detail),
    onclick ? h("button", { class: "link", onclick }, "Details") : null);
  return [
    sysCard("Graphics", sys.driver || sys.gpu || "Not detected", crashing.length >= 2 ? `Driver crashes in ${crashing.length} games` : (sys.driver && sys.gpu ? sys.gpu : "No driver crashes across games"), crashing.length >= 2 ? "warn" : ""),
    sysCard("System", sys.distro, sys.kernel ? `Kernel ${sys.kernel}` : ""),
    sysCard("PC check", !state.pcCheck ? "Checking…" : bad ? `${bad} ${bad === 1 ? "problem" : "problems"}` : warn ? `${warn} to look at` : "All good",
      "Graphics, Vulkan, Steam and limits", bad ? "bad" : warn ? "warn" : "good",
      () => { openSettings(); setTimeout(() => $("set-pc").scrollIntoView({ block: "start" }), 50); }),
    sysCard("CachyPlay", o.version ? `v${o.version}` : "", updateWords(u), u && u.available && u.latest ? "info" : "",
      u && u.available && u.latest ? () => openUpdateDialog() : null),
  ];
}

function updateWords(u) {
  if (!u) return "Checking for updates…";
  if (u.error) return "Couldn't check for updates";
  if (u.available && u.latest) return `Update ready: ${u.latest.tag}`;
  if (u.no_releases) return "No release published yet";
  if (u.skipped && u.latest) return `You skipped ${u.latest.tag}`;
  return u.last_checked ? "Up to date" : "Not checked yet";
}

function tileGrid(games) {
  const q = state.query;
  const list = games.filter((g) => !q || g.name.toLowerCase().includes(q)).filter((g) => state.homeFilter === "on" ? g.enabled : state.homeFilter === "working" ? railGroup(g) === 2 : true)
    .slice().sort((a, b) => railGroup(a) - railGroup(b) || a.name.localeCompare(b.name));
  if (!list.length) return h("p", { class: "muted" }, q ? "No games match your search." : state.homeFilter === "on" ? "CachyPlay isn't on for any game yet. Open a game and turn on its switch." : "No games here yet.");
  return h("div", { class: "tile-grid" }, list.map((g) => h("button", { class: `tile light-${g.light}`, "data-app": g.app_id, onclick: () => openGame(g.app_id) },
    coverInto(h("span", { class: "tile-cover" }, h("span", {}, g.name.slice(0, 2).toUpperCase())), g.app_id),
    h("span", { class: "tile-name" }, g.name),
    h("span", { class: "rail-status" }, h("span", { class: `light ${g.light}`, "aria-hidden": "true" }), statusText(g)))));
}
